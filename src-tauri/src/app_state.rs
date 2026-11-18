use crate::{
    certificates::CertificateManager,
    protocol_capture::{CaptureSummary, ProtocolCaptureSession},
    proxy::{self, ActiveProxyMode, CaptureArm, ProxyController},
    qq_identity::{self, LocalQqIdentity},
    qq_login_history::QqLoginHistory,
    server_sync::{ServerClient, SyncAccountInput},
    settings::{AppSettings, SettingsStore, SettingsView},
    status::StatusPayload,
    system_proxy::SystemProxyManager,
};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::{AppHandle, Emitter};
use tokio::{
    sync::{Mutex, mpsc},
    task::{JoinHandle, JoinSet},
    time::{Duration, sleep},
};
use tokio_util::sync::CancellationToken;

struct RuntimeState {
    starting: bool,
    cancellation: Option<CancellationToken>,
    task: Option<JoinHandle<()>>,
    capture_task: Option<JoinHandle<()>>,
    identity_task: Option<JoinHandle<()>>,
    proxy_controller: Option<Arc<ProxyController>>,
    capture_sender: Option<mpsc::Sender<proxy::CaptureEvent>>,
    proxy_port: u16,
    current_arm: Option<Arc<CaptureArm>>,
    capture_hint: &'static str,
    attempted_qqs: HashSet<String>,
    dismissed_qqs: HashSet<String>,
    codes_by_qq: HashMap<String, String>,
    last_code_qq: Option<String>,
    identity_warning: Option<String>,
    protocol_session: Option<Arc<ProtocolCaptureSession>>,
    sync_tasks: JoinSet<()>,
    status: StatusPayload,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            starting: false,
            cancellation: None,
            task: None,
            capture_task: None,
            identity_task: None,
            proxy_controller: None,
            capture_sender: None,
            proxy_port: 8899,
            current_arm: None,
            capture_hint: "",
            attempted_qqs: HashSet::new(),
            dismissed_qqs: HashSet::new(),
            codes_by_qq: HashMap::new(),
            last_code_qq: None,
            identity_warning: None,
            protocol_session: None,
            sync_tasks: JoinSet::new(),
            status: StatusPayload::idle(),
        }
    }
}

pub struct AppCore {
    certificates: CertificateManager,
    diagnostics_path: PathBuf,
    network_recovery: Mutex<()>,
    protocol_capture_root: PathBuf,
    proxy_manager: SystemProxyManager,
    qq_login_history: Arc<QqLoginHistory>,
    settings: SettingsStore,
    transport_stop: Mutex<()>,
    runtime: Mutex<RuntimeState>,
    startup_warning: Mutex<Option<String>>,
    pub exiting: AtomicBool,
}

impl AppCore {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            certificates: CertificateManager::new(data_dir.clone()),
            diagnostics_path: data_dir.join("traffic-diagnostics.log"),
            network_recovery: Mutex::new(()),
            protocol_capture_root: data_dir.join("protocol-captures"),
            proxy_manager: SystemProxyManager::new(data_dir.clone()),
            qq_login_history: Arc::new(QqLoginHistory::new(data_dir.clone())),
            settings: SettingsStore::new(data_dir),
            transport_stop: Mutex::new(()),
            runtime: Mutex::new(RuntimeState::default()),
            startup_warning: Mutex::new(None),
            exiting: AtomicBool::new(false),
        }
    }

    pub fn login_history(&self) -> Arc<QqLoginHistory> {
        self.qq_login_history.clone()
    }

    pub fn recover_stale(&self) {
        let result = self
            .proxy_manager
            .recover_stale()
            .and_then(|_| self.certificates.recover_stale());
        if let Err(error) = result {
            *self.startup_warning.blocking_lock() =
                Some(format!("上次异常退出后的网络恢复未完全成功: {error}"));
        }
    }

    pub async fn bootstrap(&self) -> Result<(SettingsView, StatusPayload, Option<String>), String> {
        let runtime = self.runtime.lock().await;
        let settings = self.settings.view()?;
        let warning = self.startup_warning.lock().await.clone();
        Ok((settings, runtime.status.clone(), warning))
    }

    pub async fn save_settings(
        &self,
        app: &AppHandle,
        mut settings: AppSettings,
        token: Option<String>,
    ) -> Result<SettingsView, String> {
        let mut runtime = self.runtime.lock().await;
        if runtime.cancellation.is_some() {
            settings.proxy_port = runtime.proxy_port;
            settings.protocol_capture = runtime.protocol_session.is_some();
        }
        let view = self.settings.save(settings, token)?;
        Self::refresh_running_sync_settings(app, &mut runtime, &view.settings);
        Ok(view)
    }

    pub async fn save_sync_options(
        &self,
        app: &AppHandle,
        auto_sync: bool,
        sync_official_friends: bool,
    ) -> Result<SettingsView, String> {
        let mut runtime = self.runtime.lock().await;
        let view = self
            .settings
            .set_sync_options(auto_sync, sync_official_friends)?;
        Self::refresh_running_sync_settings(app, &mut runtime, &view.settings);
        Ok(view)
    }

    fn refresh_running_sync_settings(
        app: &AppHandle,
        runtime: &mut RuntimeState,
        settings: &AppSettings,
    ) {
        if runtime.cancellation.is_none() {
            return;
        }
        let hint = capture_hint(settings.auto_sync, settings.sync_official_friends);
        runtime.capture_hint = hint;
        if let Some(controller) = &runtime.proxy_controller {
            controller
                .set_capture_friend_gids(settings.auto_sync && settings.sync_official_friends);
        }
        if runtime.protocol_session.is_none()
            && runtime.status.phase == "waiting_login"
            && let Some(arm) = runtime
                .current_arm
                .as_ref()
                .filter(|arm| arm.is_active())
                .cloned()
        {
            let status = StatusPayload::new(
                "waiting_login",
                "等待农场登录",
                waiting_login_detail(&arm.identity, hint),
                false,
            )
            .with_target_qq(&arm.identity.qq_number);
            emit_status(app, runtime, status);
        }
    }

    pub async fn set_protocol_capture(
        &self,
        app: &AppHandle,
        enabled: bool,
    ) -> Result<SettingsView, String> {
        let mut runtime = self.runtime.lock().await;
        if runtime.starting {
            return Err("正在启动代理，请稍候".to_owned());
        }
        if runtime.cancellation.is_none() {
            let view = self.settings.set_protocol_capture(enabled)?;
            if runtime.status.phase == "idle" {
                emit_status(app, &mut runtime, idle_status(enabled));
            }
            return Ok(view);
        }
        let Some(controller) = runtime.proxy_controller.clone() else {
            return Err("代理控制器状态异常，请停止后重试".to_owned());
        };
        let sender = runtime
            .capture_sender
            .clone()
            .ok_or_else(|| "代理捕获通道状态异常，请停止后重试".to_owned())?;
        let mut next_session = if enabled && runtime.protocol_session.is_none() {
            Some(ProtocolCaptureSession::create(&self.protocol_capture_root)?)
        } else {
            None
        };
        let view = self
            .settings
            .set_protocol_capture(enabled)
            .inspect_err(|_| {
                if let Some(session) = next_session.take().and_then(Arc::into_inner) {
                    session.cleanup_if_empty();
                }
            })?;
        if let Some(session) = next_session {
            if let Some(arm) = runtime.current_arm.take() {
                arm.try_cancel();
            }
            runtime.identity_warning = None;
            controller.set_mode(ActiveProxyMode::ProtocolCapture {
                session: session.clone(),
            });
            runtime.protocol_session = Some(session.clone());
            let port = runtime.proxy_port;
            let status = StatusPayload::new(
                "protocol_listening",
                "协议模式",
                protocol_listening_detail(port),
                false,
            )
            .with_capture_directory(session.directory());
            emit_status(app, &mut runtime, status);
        } else if !enabled && let Some(prev_session) = runtime.protocol_session.take() {
            let hint = capture_hint(view.settings.auto_sync, view.settings.sync_official_friends);
            runtime.capture_hint = hint;
            runtime.current_arm = None;
            runtime.identity_warning = None;
            controller.set_mode(ActiveProxyMode::CaptureCode {
                captured: sender,
                arm: None,
                capture_friend_gids: view.settings.auto_sync && view.settings.sync_official_friends,
                diagnostics_path: Some(self.diagnostics_path.clone()),
            });
            let summary = prev_session.summary();
            if summary.connection_count == 0 && summary.message_count == 0 {
                if let Ok(owned) = Arc::try_unwrap(prev_session) {
                    owned.cleanup_if_empty();
                }
                emit_status(app, &mut runtime, ready_status());
            } else {
                let detail = format!(
                    "已退出协议模式 · {} 个连接 · {} 条消息 · 选择账号继续同步",
                    summary.connection_count, summary.message_count
                );
                let status = StatusPayload::new("ready", "代理运行中", detail, false)
                    .with_capture_directory(&summary.directory);
                emit_status(app, &mut runtime, status);
            }
        }
        Ok(view)
    }

    pub fn stored_token(&self) -> Result<Option<String>, String> {
        self.settings.token()
    }

    pub fn set_update_proxy(&self, enabled: bool, proxy_url: &str) -> Result<String, String> {
        self.settings.set_update_proxy(enabled, proxy_url)
    }

    pub async fn start_capture(self: &Arc<Self>, app: AppHandle) -> Result<(), String> {
        let settings = self.settings.load()?;
        self.begin_start().await?;
        let result = self.start_proxy_session(app, settings, None).await;
        self.runtime.lock().await.starting = false;
        result
    }

    /// 为指定账号获取并同步 Code。代理未运行时先启动（仅此时安装证书），
    /// 已运行时只切换目标账号并重新布防，网络接管保持不变。
    pub async fn sync_account(
        self: &Arc<Self>,
        app: AppHandle,
        identity: LocalQqIdentity,
    ) -> Result<(), String> {
        let settings = self.settings.load()?;
        if settings.protocol_capture {
            return Err("协议模式已开启，请先关闭协议模式".to_owned());
        }
        self.validate_sync_settings(&settings)?;
        if self.arm_account(&app, &identity).await? {
            return Ok(());
        }
        self.begin_start().await?;
        let result = self
            .start_proxy_session(app, settings, Some(identity))
            .await;
        self.runtime.lock().await.starting = false;
        result
    }

    async fn arm_account(
        &self,
        app: &AppHandle,
        identity: &LocalQqIdentity,
    ) -> Result<bool, String> {
        let mut runtime = self.runtime.lock().await;
        if runtime.cancellation.is_none() {
            return Ok(false);
        }
        if runtime.protocol_session.is_some() {
            return Err("协议模式运行中，请先关闭协议模式".to_owned());
        }
        let Some(controller) = runtime.proxy_controller.clone() else {
            return Err("代理状态异常，请停止后重试".to_owned());
        };
        if let Some(previous_arm) = runtime.current_arm.take() {
            previous_arm.try_cancel();
        }
        let arm = CaptureArm::new(identity.clone());
        controller.set_arm(Some(arm.clone()))?;
        runtime.current_arm = Some(arm);
        runtime.dismissed_qqs.remove(&identity.qq_number);
        runtime.identity_warning = None;
        let status = StatusPayload::new(
            "waiting_login",
            "等待农场登录",
            waiting_login_detail(identity, runtime.capture_hint),
            false,
        )
        .with_target_qq(&identity.qq_number);
        emit_status(app, &mut runtime, status);
        Ok(true)
    }

    pub async fn cancel_account_sync(&self, app: &AppHandle) -> Result<(), String> {
        let mut runtime = self.runtime.lock().await;
        let Some(arm) = runtime.current_arm.as_ref() else {
            return Ok(());
        };
        if !arm.try_cancel() {
            return Err("已拦截到农场登录，正在处理，请稍候".to_owned());
        }
        let qq_number = arm.identity.qq_number.clone();
        runtime.current_arm = None;
        if let Some(controller) = &runtime.proxy_controller {
            let _ = controller.set_arm(None);
        }
        runtime.dismissed_qqs.insert(qq_number);
        runtime.identity_warning = None;
        emit_status(app, &mut runtime, ready_status());
        Ok(())
    }

    async fn start_proxy_session(
        self: &Arc<Self>,
        app: AppHandle,
        settings: AppSettings,
        mut initial_identity: Option<LocalQqIdentity>,
    ) -> Result<(), String> {
        let preparing_detail = format!("代理端口 {}", settings.proxy_port);
        let (tls, listener) = self
            .prepare_proxy_listener(&app, settings.proxy_port, &preparing_detail)
            .await?;

        let protocol_session = if settings.protocol_capture {
            match ProtocolCaptureSession::create(&self.protocol_capture_root) {
                Ok(session) => Some(session),
                Err(error) => {
                    let _ = self.cleanup_certificate().await;
                    return Err(self.record_failure(&app, error, false).await);
                }
            }
        } else {
            None
        };

        // 从「启动代理」进入普通模式时，若当前恰有 1 个在线 QQ 且满足自动同步条件，直接自动布防。
        if protocol_session.is_none()
            && initial_identity.is_none()
            && self.validate_sync_settings(&settings).is_ok()
            && let Ok(Ok(identities)) = tokio::time::timeout(
                Duration::from_millis(800),
                qq_identity::detect_all_async(self.qq_login_history.clone()),
            )
            .await
            && identities.len() == 1
        {
            initial_identity = identities.into_iter().next();
        }

        let cancellation = CancellationToken::new();
        let identity_cancellation = cancellation.clone();
        let (sender, receiver) = mpsc::channel(16);
        let _ = std::fs::remove_file(&self.diagnostics_path);

        let initial_arm = initial_identity.as_ref().cloned().map(CaptureArm::new);
        let initial_mode = if let Some(session) = protocol_session.clone() {
            ActiveProxyMode::ProtocolCapture { session }
        } else {
            ActiveProxyMode::CaptureCode {
                captured: sender.clone(),
                arm: initial_arm.clone(),
                capture_friend_gids: settings.auto_sync && settings.sync_official_friends,
                diagnostics_path: Some(self.diagnostics_path.clone()),
            }
        };
        let controller = ProxyController::new(initial_mode);
        let task = tokio::spawn(proxy::run(
            listener,
            tls,
            cancellation.clone(),
            controller.clone(),
        ));
        let hint = capture_hint(settings.auto_sync, settings.sync_official_friends);
        self.store_session_transport(
            cancellation,
            task,
            controller,
            sender,
            settings.proxy_port,
            initial_arm,
            hint,
            protocol_session.clone(),
        )
        .await;

        if let Err(error) = self.enable_system_proxy(settings.proxy_port).await {
            let _ = self.stop_transport().await;
            if let Some(session) = self.take_protocol_session().await
                && let Ok(owned) = Arc::try_unwrap(session)
            {
                owned.cleanup_if_empty();
            }
            return Err(self.record_failure(&app, error, false).await);
        }

        let status = if let Some(session) = protocol_session {
            StatusPayload::new(
                "protocol_listening",
                "协议模式",
                protocol_listening_detail(settings.proxy_port),
                false,
            )
            .with_capture_directory(session.directory())
        } else if let Some(identity) = initial_identity {
            StatusPayload::new(
                "waiting_login",
                "等待农场登录",
                waiting_login_detail(&identity, hint),
                false,
            )
            .with_target_qq(identity.qq_number)
        } else {
            ready_status()
        };
        self.publish(&app, status).await;
        self.start_identity_monitor(app.clone(), identity_cancellation)
            .await;
        self.spawn_capture_handler(app, receiver).await;
        Ok(())
    }

    async fn prepare_proxy_listener(
        &self,
        app: &AppHandle,
        proxy_port: u16,
        detail: &str,
    ) -> Result<(Arc<rustls::ServerConfig>, tokio::net::TcpListener), String> {
        self.publish(
            app,
            StatusPayload::new("preparing_proxy", "正在启动代理", detail, false),
        )
        .await;

        let tls = match self.prepare_certificate().await {
            Ok(tls) => tls,
            Err(error) => return Err(self.record_failure(app, error, false).await),
        };
        let listener = match proxy::bind(proxy_port).await {
            Ok(listener) => listener,
            Err(error) => {
                let _ = self.cleanup_certificate().await;
                return Err(self.record_failure(app, error, false).await);
            }
        };
        Ok((tls, listener))
    }

    pub async fn stop_capture(&self, app: &AppHandle) -> Result<(), String> {
        let protocol_session = self.take_protocol_session().await;
        self.stop_transport().await?;
        self.clear_session_identity().await;
        let status = if let Some(session) = protocol_session {
            let summary = session.summary();
            StatusPayload::new(
                "stopped",
                "协议模式已停止",
                protocol_capture_completion_detail(&summary, "网络已恢复"),
                false,
            )
            .with_capture_directory(&summary.directory)
        } else {
            StatusPayload::new("stopped", "已停止", "网络已恢复", self.has_code().await)
        };
        self.publish(app, status).await;
        Ok(())
    }

    pub async fn cleanup_network(&self, app: &AppHandle) -> Result<(), String> {
        let protocol_session = self.take_protocol_session().await;
        self.stop_transport().await?;
        self.clear_session_identity().await;
        let status = if let Some(session) = protocol_session {
            let summary = session.summary();
            StatusPayload::new(
                "idle",
                "协议模式已清理",
                protocol_capture_completion_detail(&summary, "网络已清理"),
                false,
            )
            .with_capture_directory(&summary.directory)
        } else {
            StatusPayload::new(
                "idle",
                "网络已清理",
                "代理已关闭，临时证书已移除",
                self.has_code().await,
            )
        };
        self.publish(app, status).await;
        Ok(())
    }

    pub async fn captured_code(&self, qq_number: Option<&str>) -> Result<String, String> {
        let runtime = self.runtime.lock().await;
        if let Some(qq) = qq_number.map(str::trim).filter(|value| !value.is_empty()) {
            return runtime
                .codes_by_qq
                .get(qq)
                .cloned()
                .ok_or_else(|| format!("QQ {qq} 当前没有可复制的 Code"));
        }
        runtime
            .last_code_qq
            .as_deref()
            .and_then(|qq| runtime.codes_by_qq.get(qq))
            .cloned()
            .ok_or_else(|| "当前没有可复制的 Code".to_owned())
    }

    pub async fn open_capture_directory(&self) -> Result<(), String> {
        let directory = self
            .runtime
            .lock()
            .await
            .status
            .capture_directory
            .clone()
            .ok_or_else(|| "当前没有可打开的日志目录".to_owned())?;
        tokio::task::spawn_blocking(move || crate::windows::open_directory(Path::new(&directory)))
            .await
            .map_err(|error| format!("打开日志目录失败: {error}"))?
    }

    pub async fn shutdown(&self) {
        let _ = self.stop_transport().await;
    }

    async fn spawn_capture_handler(
        self: &Arc<Self>,
        app: AppHandle,
        mut receiver: mpsc::Receiver<proxy::CaptureEvent>,
    ) {
        let core = self.clone();
        let mut runtime = self.runtime.lock().await;
        let task = tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                match event {
                    proxy::CaptureEvent::Intercepted { arm } => {
                        core.handle_intercepted(&app, arm).await;
                    }
                    proxy::CaptureEvent::Captured { arm, login } => {
                        let core_for_sync = core.clone();
                        let app_for_sync = app.clone();
                        let mut runtime = core.runtime.lock().await;
                        while runtime.sync_tasks.try_join_next().is_some() {}
                        runtime.sync_tasks.spawn(async move {
                            core_for_sync
                                .handle_captured_login(&app_for_sync, arm, login)
                                .await;
                        });
                    }
                }
            }
        });
        runtime.capture_task = Some(task);
    }

    async fn handle_intercepted(&self, app: &AppHandle, arm: Arc<CaptureArm>) {
        let mut runtime = self.runtime.lock().await;
        let qq = arm.identity.qq_number.clone();
        runtime.attempted_qqs.insert(qq.clone());
        if runtime
            .current_arm
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &arm))
        {
            runtime.current_arm = None;
        }
        let status = StatusPayload::new("code_captured", "已获取 Code", "正在读取登录信息", false)
            .with_target_qq(qq);
        emit_status(app, &mut runtime, status);
    }

    async fn handle_captured_login(
        &self,
        app: &AppHandle,
        arm: Arc<CaptureArm>,
        captured: proxy::CapturedLogin,
    ) {
        let qq = arm.identity.qq_number.clone();
        self.set_code(&qq, Some(captured.code.clone())).await;
        let captured_detail = if captured.friend_gids.is_empty() {
            "Code 已获取\n正在确认 QQ".to_owned()
        } else {
            format!(
                "Code 已获取 · 好友 GID {} 个\n正在确认 QQ",
                captured.friend_gids.len()
            )
        };
        self.publish(
            app,
            StatusPayload::new("code_captured", "已获取 Code", captured_detail, true)
                .with_target_qq(&qq),
        )
        .await;

        let verification = self.revalidate_identity_for(&arm.identity).await;
        let (settings, token) = match self.settings.load_for_sync() {
            Ok(settings_and_token) => settings_and_token,
            Err(error) => {
                self.record_failure_for(app, &qq, error, true).await;
                return;
            }
        };
        if settings.auto_sync {
            match verification {
                Ok(verified_identity) => {
                    self.sync_captured_login(app, &settings, token, &verified_identity, &captured)
                        .await;
                }
                Err(reason) => {
                    self.publish_identity_blocked(app, &qq, &reason).await;
                }
            }
        } else {
            self.publish(
                app,
                StatusPayload::new("completed", "Code 可复制", "可继续选择其他账号", true)
                    .with_target_qq(&qq),
            )
            .await;
        }
    }

    async fn sync_captured_login(
        &self,
        app: &AppHandle,
        settings: &AppSettings,
        token: Option<String>,
        locked_identity: &LocalQqIdentity,
        captured: &proxy::CapturedLogin,
    ) {
        let qq = &locked_identity.qq_number;
        self.publish(
            app,
            StatusPayload::new("syncing", "正在同步", "Code 已获取\n正在确认远程账号", true)
                .with_target_qq(qq),
        )
        .await;
        let result = async {
            let token = token.ok_or_else(|| "未配置服务器 Token".to_owned())?;
            let client = ServerClient::new()?;
            let profile = client
                .sync_code(
                    &settings.server_url,
                    &token,
                    SyncAccountInput {
                        account_name: &settings.account_name,
                        qq_number: qq,
                        nickname: &locked_identity.nickname,
                        avatar_url: &locked_identity.avatar_url,
                        code: &captured.code,
                    },
                )
                .await?;
            Ok::<_, String>((client, token, profile))
        }
        .await;
        match result {
            Ok((client, token, mut profile)) => {
                apply_captured_identity(&mut profile, captured);
                self.set_code(qq, None).await;
                let mut detail = sync_completion_detail(&profile, locked_identity);
                if settings.sync_official_friends {
                    detail.push_str(
                        &self
                            .sync_official_friend_gids(
                                app,
                                qq,
                                &profile,
                                captured,
                                &client,
                                (&settings.server_url, &token),
                            )
                            .await,
                    );
                }
                self.publish(
                    app,
                    StatusPayload::new("completed", "同步完成", detail, false)
                        .with_profile(profile)
                        .with_target_qq(qq),
                )
                .await;
            }
            Err(error) => {
                self.record_failure_for(app, qq, format!("同步失败：{error}\nCode 可复制"), true)
                    .await;
            }
        }
    }

    async fn sync_official_friend_gids(
        &self,
        app: &AppHandle,
        qq_number: &str,
        profile: &crate::server_sync::AccountProfile,
        captured: &proxy::CapturedLogin,
        client: &ServerClient,
        (server_url, token): (&str, &str),
    ) -> String {
        if captured.friend_gids.is_empty() {
            let warning = captured
                .friend_capture_warning
                .as_deref()
                .unwrap_or("官方登录期间没有返回可识别的好友 GID");
            return format!("\n好友未同步：{warning}");
        }

        let Some(own_gid) = captured_own_gid(captured) else {
            return "\n好友未同步：官方登录回包缺少自身 GID".to_owned();
        };

        let friend_gids = friend_gids_without_self(&captured.friend_gids, own_gid);
        let excluded_count = captured.friend_gids.len() - friend_gids.len();
        let progress_detail = format!("好友 GID {} 个\n正在更新远程账号", friend_gids.len());
        self.publish(
            app,
            StatusPayload::new("syncing", "正在同步好友", progress_detail, false)
                .with_profile(profile.clone())
                .with_target_qq(qq_number),
        )
        .await;

        let own_gids = [own_gid.to_owned()];
        let cleanup_result = client
            .batch_remove_friend_gids(server_url, token, &profile.account_id, &own_gids)
            .await;
        let add_result = if friend_gids.is_empty() {
            None
        } else {
            Some(
                client
                    .batch_add_friend_gids(server_url, token, &profile.account_id, &friend_gids)
                    .await,
            )
        };

        let mut details = Vec::new();
        match cleanup_result {
            Ok(_) => {}
            Err(error) => details.push(format!("清理自身 GID 失败：{error}")),
        }
        match add_result {
            Some(Ok(result)) => details.push(format!(
                "好友 GID {} 个 · 新增 {} 个",
                result.known_friend_gid_count, result.added_count
            )),
            Some(Err(error)) => details.push(format!("好友同步失败：{error}")),
            None => details.push("好友 GID 0 个".to_owned()),
        }
        if excluded_count > 0 {
            details.push(format!("已排除自身 GID {} 个", excluded_count));
        }
        if let Some(warning) = captured.friend_capture_warning.as_deref() {
            details.push(format!("捕获提示：{warning}"));
        }
        if details.is_empty() {
            String::new()
        } else {
            format!("\n{}", details.join("\n"))
        }
    }

    async fn publish_identity_blocked(&self, app: &AppHandle, qq_number: &str, reason: &str) {
        self.publish(
            app,
            StatusPayload::new(
                "identity_changed",
                "未同步",
                format!("QQ 已变化：{reason}\nCode 可复制"),
                true,
            )
            .with_target_qq(qq_number),
        )
        .await;
    }

    async fn begin_start(&self) -> Result<(), String> {
        let mut runtime = self.runtime.lock().await;
        if runtime.starting
            || runtime.cancellation.is_some()
            || runtime
                .capture_task
                .as_ref()
                .is_some_and(|task| !task.is_finished())
        {
            return Err("本地代理任务已经在运行".to_owned());
        }
        runtime.capture_task = None;
        runtime.starting = true;
        Ok(())
    }

    fn validate_sync_settings(&self, settings: &AppSettings) -> Result<(), String> {
        if settings.auto_sync {
            if settings.server_url.is_empty() {
                return Err("启用自动同步时必须填写服务器地址".to_owned());
            }
            if self.settings.token()?.is_none() {
                return Err("启用自动同步时必须填写后台登录 Token".to_owned());
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn store_session_transport(
        &self,
        cancellation: CancellationToken,
        task: JoinHandle<()>,
        proxy_controller: Arc<ProxyController>,
        capture_sender: mpsc::Sender<proxy::CaptureEvent>,
        proxy_port: u16,
        current_arm: Option<Arc<CaptureArm>>,
        capture_hint: &'static str,
        protocol_session: Option<Arc<ProtocolCaptureSession>>,
    ) {
        let mut runtime = self.runtime.lock().await;
        runtime.cancellation = Some(cancellation);
        runtime.task = Some(task);
        runtime.capture_task = None;
        runtime.identity_task = None;
        runtime.proxy_controller = Some(proxy_controller);
        runtime.capture_sender = Some(capture_sender);
        runtime.proxy_port = proxy_port;
        runtime.current_arm = current_arm;
        runtime.capture_hint = capture_hint;
        runtime.attempted_qqs.clear();
        runtime.dismissed_qqs.clear();
        runtime.codes_by_qq.clear();
        runtime.last_code_qq = None;
        runtime.identity_warning = None;
        runtime.protocol_session = protocol_session;
    }

    async fn take_protocol_session(&self) -> Option<Arc<ProtocolCaptureSession>> {
        self.runtime.lock().await.protocol_session.take()
    }

    async fn stop_transport(&self) -> Result<(), String> {
        let _stopping = self.transport_stop.lock().await;
        let (cancellation, task, capture_task, identity_task) = {
            let mut runtime = self.runtime.lock().await;
            if let Some(arm) = runtime.current_arm.take() {
                arm.try_cancel();
            }
            runtime.proxy_controller = None;
            runtime.capture_sender = None;
            (
                runtime.cancellation.take(),
                runtime.task.take(),
                runtime.capture_task.take(),
                runtime.identity_task.take(),
            )
        };
        if let Some(cancellation) = cancellation {
            cancellation.cancel();
        }
        if let Some(task) = identity_task {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = capture_task {
            task.abort();
            let _ = task.await;
        }
        let mut sync_tasks = std::mem::take(&mut self.runtime.lock().await.sync_tasks);
        sync_tasks.shutdown().await;
        if let Some(task) = task {
            let _ = task.await;
        }
        self.recover_network().await
    }

    async fn start_identity_monitor(
        self: &Arc<Self>,
        app: AppHandle,
        cancellation: CancellationToken,
    ) {
        let core = self.clone();
        let task = tokio::spawn(async move {
            loop {
                if cancellation.is_cancelled() {
                    break;
                }
                let skip_scan = {
                    let runtime = core.runtime.lock().await;
                    runtime.cancellation.is_none() || runtime.protocol_session.is_some()
                };
                if !skip_scan {
                    match qq_identity::detect_stable_all_async(core.qq_login_history.clone()).await
                    {
                        Ok(identities) => core.reconcile_identities(&app, identities).await,
                        Err(error) => core.handle_identity_monitor_error(&app, error).await,
                    }
                }
                tokio::select! {
                    _ = cancellation.cancelled() => break,
                    _ = sleep(Duration::from_millis(800)) => {}
                }
            }
        });
        self.runtime.lock().await.identity_task = Some(task);
    }

    async fn reconcile_identities(&self, app: &AppHandle, identities: Vec<LocalQqIdentity>) {
        let needs_auto_arm_check = {
            let runtime = self.runtime.lock().await;
            if runtime.cancellation.is_none() || runtime.protocol_session.is_some() {
                return;
            }
            should_check_auto_arm(
                &identities,
                runtime.current_arm.as_deref(),
                &runtime.attempted_qqs,
                &runtime.dismissed_qqs,
            )
        };
        let can_auto_arm = if needs_auto_arm_check {
            self.settings
                .load()
                .ok()
                .is_some_and(|settings| self.validate_sync_settings(&settings).is_ok())
        } else {
            false
        };

        let mut runtime = self.runtime.lock().await;
        if runtime.cancellation.is_none() || runtime.protocol_session.is_some() {
            return;
        }
        let Some(controller) = runtime.proxy_controller.clone() else {
            return;
        };

        if let Some(current_arm) = runtime.current_arm.clone() {
            if !current_arm.is_active() {
                return;
            }
            if let Some(refreshed) = identities
                .iter()
                .find(|id| id.qq_number == current_arm.identity.qq_number)
            {
                if refreshed.nickname != current_arm.identity.nickname
                    || refreshed.avatar_url != current_arm.identity.avatar_url
                {
                    current_arm.try_cancel();
                    let next_arm = CaptureArm::new(refreshed.clone());
                    if controller.set_arm(Some(next_arm.clone())).is_ok() {
                        runtime.current_arm = Some(next_arm);
                        let status = StatusPayload::new(
                            "waiting_login",
                            "等待农场登录",
                            waiting_login_detail(refreshed, runtime.capture_hint),
                            false,
                        )
                        .with_target_qq(&refreshed.qq_number);
                        emit_status(app, &mut runtime, status);
                    }
                }
                runtime.identity_warning = None;
                return;
            }

            current_arm.try_cancel();
            if can_auto_arm
                && let Some(next_identity) = next_single_window_candidate(
                    &identities,
                    &runtime.attempted_qqs,
                    &runtime.dismissed_qqs,
                    false,
                )
            {
                let next_arm = CaptureArm::new(next_identity.clone());
                if controller.set_arm(Some(next_arm.clone())).is_ok() {
                    runtime.current_arm = Some(next_arm);
                    runtime.identity_warning = None;
                    let status = StatusPayload::new(
                        "waiting_login",
                        "等待农场登录",
                        waiting_login_detail(&next_identity, runtime.capture_hint),
                        false,
                    )
                    .with_target_qq(&next_identity.qq_number);
                    emit_status(app, &mut runtime, status);
                    return;
                }
            }

            runtime.current_arm = None;
            let _ = controller.set_arm(None);
            runtime.identity_warning = None;
            let detail = format!(
                "QQ {} 已退出登录，请重新选择账号",
                current_arm.identity.qq_number
            );
            emit_status(
                app,
                &mut runtime,
                StatusPayload::new("ready", "代理运行中", detail, false),
            );
            return;
        }

        if can_auto_arm
            && let Some(candidate) = next_single_window_candidate(
                &identities,
                &runtime.attempted_qqs,
                &runtime.dismissed_qqs,
                true,
            )
        {
            let next_arm = CaptureArm::new(candidate.clone());
            if controller.set_arm(Some(next_arm.clone())).is_ok() {
                runtime.current_arm = Some(next_arm);
                runtime.identity_warning = None;
                let status = StatusPayload::new(
                    "waiting_login",
                    "等待农场登录",
                    waiting_login_detail(&candidate, runtime.capture_hint),
                    false,
                )
                .with_target_qq(&candidate.qq_number);
                emit_status(app, &mut runtime, status);
            }
        }
    }

    async fn handle_identity_monitor_error(&self, app: &AppHandle, error: String) {
        if !error.contains("未找到 QQ 主窗口") {
            return;
        }
        let mut runtime = self.runtime.lock().await;
        if runtime.cancellation.is_none() || runtime.protocol_session.is_some() {
            return;
        }
        let Some(current_arm) = runtime.current_arm.as_ref() else {
            return;
        };
        if !current_arm.is_active() {
            return;
        }
        let arm = runtime.current_arm.take().unwrap();
        arm.try_cancel();
        if let Some(controller) = &runtime.proxy_controller {
            let _ = controller.set_arm(None);
        }
        runtime.identity_warning = Some(error);
        emit_status(
            app,
            &mut runtime,
            StatusPayload::new(
                "ready",
                "代理运行中",
                "QQ 主窗口已关闭，请登录 QQ 后选择账号",
                false,
            ),
        );
    }

    async fn prepare_certificate(&self) -> Result<Arc<rustls::ServerConfig>, String> {
        let certificates = &self.certificates;
        tokio::task::block_in_place(|| certificates.prepare())
    }

    async fn cleanup_certificate(&self) -> Result<(), String> {
        let certificates = &self.certificates;
        tokio::task::block_in_place(|| certificates.cleanup())
    }

    async fn enable_system_proxy(&self, port: u16) -> Result<(), String> {
        let proxy_manager = &self.proxy_manager;
        tokio::task::block_in_place(|| proxy_manager.enable(port))
    }

    async fn recover_network(&self) -> Result<(), String> {
        let _recovery = self.network_recovery.lock().await;
        let proxy_manager = &self.proxy_manager;
        let certificates = &self.certificates;
        tokio::task::block_in_place(|| {
            let proxy_result = proxy_manager.restore();
            let certificate_result = certificates.cleanup();
            proxy_result.and(certificate_result)
        })
    }

    async fn set_code(&self, qq_number: &str, code: Option<String>) {
        let mut runtime = self.runtime.lock().await;
        match code {
            Some(code) => {
                runtime.codes_by_qq.insert(qq_number.to_owned(), code);
                runtime.last_code_qq = Some(qq_number.to_owned());
            }
            None => {
                runtime.codes_by_qq.remove(qq_number);
                if runtime.last_code_qq.as_deref() == Some(qq_number) {
                    runtime.last_code_qq = runtime.codes_by_qq.keys().next().cloned();
                }
            }
        }
    }

    async fn has_code(&self) -> bool {
        !self.runtime.lock().await.codes_by_qq.is_empty()
    }

    async fn clear_session_identity(&self) {
        let mut runtime = self.runtime.lock().await;
        runtime.current_arm = None;
        runtime.identity_warning = None;
    }

    async fn revalidate_identity_for(
        &self,
        locked_identity: &LocalQqIdentity,
    ) -> Result<LocalQqIdentity, String> {
        match detect_selected_qq_with_retry(
            self.qq_login_history.clone(),
            &locked_identity.qq_number,
            2,
        )
        .await
        {
            Ok(current) if current.qq_number == locked_identity.qq_number => Ok(current),
            Ok(current) => Err(format!(
                "捕获前锁定 QQ {}，捕获 Code 后检测到 QQ {}，账号已变化",
                locked_identity.qq_number, current.qq_number
            )),
            Err(error) => Err(format!(
                "捕获 Code 后无法再次确认 QQ {}：{}",
                locked_identity.qq_number, error
            )),
        }
    }

    async fn publish(&self, app: &AppHandle, status: StatusPayload) {
        let mut runtime = self.runtime.lock().await;
        emit_status(app, &mut runtime, status);
    }

    async fn record_failure(&self, app: &AppHandle, error: String, code_available: bool) -> String {
        self.publish(
            app,
            StatusPayload::new("error", "操作失败", &error, code_available),
        )
        .await;
        error
    }

    async fn record_failure_for(
        &self,
        app: &AppHandle,
        qq_number: &str,
        error: String,
        code_available: bool,
    ) -> String {
        self.publish(
            app,
            StatusPayload::new("error", "操作失败", &error, code_available)
                .with_target_qq(qq_number),
        )
        .await;
        error
    }

    pub fn mark_exiting(&self) -> bool {
        self.exiting.swap(true, Ordering::SeqCst)
    }
}

fn should_check_auto_arm(
    identities: &[LocalQqIdentity],
    current_arm: Option<&CaptureArm>,
    attempted_qqs: &HashSet<String>,
    dismissed_qqs: &HashSet<String>,
) -> bool {
    if identities.len() != 1 {
        return false;
    }
    let only_qq = &identities[0].qq_number;
    match current_arm {
        Some(arm) if arm.is_active() => {
            arm.identity.qq_number != *only_qq && !dismissed_qqs.contains(only_qq)
        }
        Some(_) => false,
        None => !attempted_qqs.contains(only_qq) && !dismissed_qqs.contains(only_qq),
    }
}

fn next_single_window_candidate(
    identities: &[LocalQqIdentity],
    attempted_qqs: &HashSet<String>,
    dismissed_qqs: &HashSet<String>,
    skip_attempted: bool,
) -> Option<LocalQqIdentity> {
    if identities.len() != 1 {
        return None;
    }
    let candidate = &identities[0];
    if dismissed_qqs.contains(&candidate.qq_number) {
        return None;
    }
    if skip_attempted && attempted_qqs.contains(&candidate.qq_number) {
        return None;
    }
    Some(candidate.clone())
}

fn emit_status(app: &AppHandle, runtime: &mut RuntimeState, mut status: StatusPayload) {
    status.session_active = runtime.cancellation.is_some();
    let protocol_session = runtime
        .protocol_session
        .as_ref()
        .filter(|_| status.session_active);
    status.protocol_mode = protocol_session.is_some();
    status.proxy_port = status.session_active.then_some(runtime.proxy_port);
    if let Some(session) = protocol_session {
        status = status.with_capture_directory(session.directory());
    }
    status.armed_qq = if status.session_active && !status.protocol_mode {
        runtime
            .current_arm
            .as_ref()
            .filter(|arm| arm.is_active())
            .map(|arm| arm.identity.qq_number.clone())
    } else {
        None
    };
    runtime.status = status.clone();
    let _ = app.emit("capture-status", status);
}

fn idle_status(protocol_mode: bool) -> StatusPayload {
    if protocol_mode {
        StatusPayload::new(
            "idle",
            "协议模式待启动",
            "点击“启动协议监听”，再打开 QQ 农场。",
            false,
        )
    } else {
        StatusPayload::idle()
    }
}

fn ready_status() -> StatusPayload {
    StatusPayload::new("ready", "代理运行中", "选择账号继续同步", false)
}

fn capture_hint(auto_sync: bool, sync_official_friends: bool) -> &'static str {
    if auto_sync && sync_official_friends {
        "在这个 QQ 里打开农场；获取 Code 和好友后自动同步"
    } else if auto_sync {
        "在这个 QQ 里打开农场；获取 Code 后自动同步"
    } else {
        "在这个 QQ 里打开农场；获取 Code"
    }
}

fn waiting_login_detail(identity: &LocalQqIdentity, hint: &str) -> String {
    let nickname = if identity.nickname.is_empty() {
        "未读取到昵称"
    } else {
        identity.nickname.as_str()
    };
    format!("QQ {}（{nickname}）\n{hint}", identity.qq_number)
}

fn protocol_listening_detail(proxy_port: u16) -> String {
    format!("代理端口 {proxy_port}")
}

fn protocol_capture_completion_detail(summary: &CaptureSummary, network_result: &str) -> String {
    format!(
        "{network_result} · {} 个连接 · {} 条消息 · {}",
        summary.connection_count,
        summary.message_count,
        readable_bytes(summary.total_bytes)
    )
}

fn readable_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    if bytes >= MIB {
        format!("{:.2} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

async fn detect_selected_qq_with_retry(
    history: Arc<QqLoginHistory>,
    qq_number: &str,
    max_attempts: usize,
) -> Result<LocalQqIdentity, String> {
    let mut last_error = "未能确认当前 QQ".to_owned();
    for attempt in 0..max_attempts {
        match qq_identity::detect_selected_stable_async(history.clone(), qq_number).await {
            Ok(identity) => return Ok(identity),
            Err(error) => last_error = error,
        }
        if attempt + 1 < max_attempts {
            sleep(Duration::from_millis(250)).await;
        }
    }
    Err(last_error)
}

fn friend_gids_without_self(captured_gids: &[String], own_gid: &str) -> Vec<String> {
    let own_gid = own_gid.trim();
    if own_gid.is_empty() {
        return captured_gids.to_vec();
    }
    captured_gids
        .iter()
        .filter(|gid| gid.trim() != own_gid)
        .cloned()
        .collect()
}

fn apply_captured_identity(
    profile: &mut crate::server_sync::AccountProfile,
    captured: &proxy::CapturedLogin,
) {
    if let Some(gid) = captured_own_gid(captured) {
        profile.gid = gid.to_owned();
        // Both values must come from this login, never mix an old remote OpenID
        // with the GID captured from the current official connection.
        profile.open_id = captured.own_identity.as_ref().unwrap().open_id.clone();
    }
}

fn captured_own_gid(captured: &proxy::CapturedLogin) -> Option<&str> {
    let gid = captured.own_identity.as_ref()?.gid.trim();
    (!gid.is_empty()
        && gid.len() <= 19
        && gid.bytes().all(|byte| byte.is_ascii_digit())
        && gid.bytes().any(|byte| byte != b'0'))
    .then_some(gid)
}

fn sync_completion_detail(
    profile: &crate::server_sync::AccountProfile,
    locked_identity: &LocalQqIdentity,
) -> String {
    let remote_name = if profile.nickname.is_empty() {
        profile.account_name.as_str()
    } else {
        profile.nickname.as_str()
    };
    let remote = if profile.has_game_identity() {
        let identity = if !profile.gid.is_empty() {
            format!("GID {}", profile.gid)
        } else {
            format!(
                "OpenID {}…",
                profile.open_id.chars().take(8).collect::<String>()
            )
        };
        format!("{remote_name} · {identity}")
    } else {
        format!("{remote_name} · 等待 GID/OpenID")
    };
    format!("{remote}\nQQ {}", locked_identity.qq_number)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(qq_number: &str, nickname: &str) -> LocalQqIdentity {
        LocalQqIdentity {
            qq_number: qq_number.to_owned(),
            nickname: nickname.to_owned(),
            avatar_url: String::new(),
            source: "test",
            verification_detail: "test",
        }
    }

    #[test]
    fn waiting_copy_names_the_account_and_next_step() {
        let detail = waiting_login_detail(&identity("3170105001", "落"), capture_hint(true, false));

        assert_eq!(
            detail,
            "QQ 3170105001（落）\n在这个 QQ 里打开农场；获取 Code 后自动同步"
        );
    }

    #[test]
    fn protocol_listening_copy_reports_the_port_and_session_directory() {
        let directory = Path::new("中文目录 with spaces")
            .join("protocol-captures")
            .join("session-123");
        let status = StatusPayload::new(
            "protocol_listening",
            "协议模式",
            protocol_listening_detail(8899),
            false,
        )
        .with_capture_directory(&directory);
        let payload = serde_json::to_value(status).unwrap();
        assert_eq!(payload["detail"], "代理端口 8899");
        assert_eq!(
            payload["capture_directory"],
            directory.to_string_lossy().as_ref()
        );
    }

    #[test]
    fn protocol_completion_copy_keeps_only_the_result_counts_and_directory() {
        let directory = Path::new("protocol-captures").join("session-123");
        let summary = CaptureSummary {
            directory: directory.clone(),
            connection_count: 2,
            message_count: 17,
            total_bytes: 2048,
        };

        let detail = protocol_capture_completion_detail(&summary, "网络已恢复");
        let status = StatusPayload::new("stopped", "协议模式已停止", detail, false)
            .with_capture_directory(&summary.directory);
        assert_eq!(status.detail, "网络已恢复 · 2 个连接 · 17 条消息 · 2.0 KiB");
        assert_eq!(
            status.capture_directory.as_deref(),
            Some(directory.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn capture_arm_allows_pipelined_switch_while_previous_account_is_syncing() {
        let (sender, _receiver) = mpsc::channel(4);
        let arm_a = CaptureArm::new(identity("1343475483", "账号一"));
        let controller = ProxyController::new(ActiveProxyMode::CaptureCode {
            captured: sender,
            arm: Some(arm_a.clone()),
            capture_friend_gids: false,
            diagnostics_path: None,
        });

        assert!(controller.claim_code(&arm_a, "synthetic-code-a"));
        assert!(!arm_a.is_active());
        assert!(controller.interception_mode().is_none());

        let arm_b = CaptureArm::new(identity("3170105001", "账号二"));
        controller.set_arm(Some(arm_b.clone())).unwrap();
        assert!(arm_b.is_active());
        assert!(controller.interception_mode().is_some());
        assert!(controller.claim_code(&arm_b, "synthetic-code-b"));
    }

    #[tokio::test]
    async fn codes_are_stored_and_cleared_per_qq_independently() {
        let core = AppCore::new(std::env::temp_dir().join("qq-farm-codes-per-qq-test"));
        core.set_code("100001", Some("code-a".to_owned())).await;
        core.set_code("100002", Some("code-b".to_owned())).await;

        assert_eq!(core.captured_code(Some("100001")).await.unwrap(), "code-a");
        assert_eq!(core.captured_code(Some("100002")).await.unwrap(), "code-b");
        assert_eq!(core.captured_code(None).await.unwrap(), "code-b");

        core.set_code("100002", None).await;
        assert!(core.captured_code(Some("100002")).await.is_err());
        assert_eq!(core.captured_code(Some("100001")).await.unwrap(), "code-a");
        assert_eq!(core.captured_code(None).await.unwrap(), "code-a");
    }

    #[test]
    fn single_window_switch_candidate_follows_new_account_unless_dismissed() {
        let qq1 = identity("100001", "账号一");
        let qq2 = identity("100002", "账号二");
        let mut attempted = HashSet::new();
        let mut dismissed = HashSet::new();

        // 单窗口切到新号 qq2，无论 qq2 之前是否在未触发前切走，只要未手动取消都可自动跟随
        let candidate =
            next_single_window_candidate(std::slice::from_ref(&qq2), &attempted, &dismissed, false);
        assert_eq!(candidate.unwrap().qq_number, "100002");

        // 多窗口时不自动猜测布防
        assert!(
            next_single_window_candidate(
                &[qq1.clone(), qq2.clone()],
                &attempted,
                &dismissed,
                true,
            )
            .is_none()
        );

        // 已尝试过的单窗口账号不自动重复布防；已取消的不自动恢复布防
        attempted.insert("100001".to_owned());
        assert!(
            next_single_window_candidate(std::slice::from_ref(&qq1), &attempted, &dismissed, true,)
                .is_none()
        );
        dismissed.insert("100002".to_owned());
        assert!(next_single_window_candidate(&[qq2], &attempted, &dismissed, false).is_none());
    }

    #[test]
    fn official_friend_gids_exclude_the_current_farm_account() {
        let captured = vec!["10001".to_owned(), "10002".to_owned(), "10003".to_owned()];

        let filtered = friend_gids_without_self(&captured, "10002");

        assert_eq!(filtered, vec!["10001".to_owned(), "10003".to_owned()]);
    }

    #[test]
    fn official_own_gid_comes_from_the_captured_login_reply() {
        let captured = proxy::CapturedLogin {
            code: "test-code".to_owned(),
            own_identity: Some(crate::friend_capture::CapturedLoginIdentity {
                gid: " 1027000001 ".to_owned(),
                open_id: "current-open-id".to_owned(),
            }),
            friend_gids: vec!["10001".to_owned()],
            friend_capture_warning: None,
        };

        assert_eq!(captured_own_gid(&captured), Some("1027000001"));
    }

    #[test]
    fn invalid_captured_own_gids_are_rejected() {
        for own_gid in [
            None,
            Some(String::new()),
            Some("0".to_owned()),
            Some("12ab".to_owned()),
        ] {
            let captured = proxy::CapturedLogin {
                code: "test-code".to_owned(),
                own_identity: own_gid.map(|gid| crate::friend_capture::CapturedLoginIdentity {
                    gid,
                    open_id: String::new(),
                }),
                friend_gids: Vec::new(),
                friend_capture_warning: None,
            };

            assert_eq!(captured_own_gid(&captured), None);
        }
    }

    #[test]
    fn current_login_replaces_remote_identity_as_a_pair() {
        let mut profile = crate::server_sync::AccountProfile {
            account_id: "remote-account".to_owned(),
            account_name: "测试账号".to_owned(),
            nickname: "测试农夫".to_owned(),
            qq_number: "123456".to_owned(),
            gid: "old-gid".to_owned(),
            open_id: "old-open-id".to_owned(),
            avatar_url: String::new(),
            running: true,
        };
        for open_id in ["current-open-id", ""] {
            let captured = proxy::CapturedLogin {
                code: "test-code".to_owned(),
                own_identity: Some(crate::friend_capture::CapturedLoginIdentity {
                    gid: "1027000001".to_owned(),
                    open_id: open_id.to_owned(),
                }),
                friend_gids: Vec::new(),
                friend_capture_warning: None,
            };
            apply_captured_identity(&mut profile, &captured);
            assert_eq!(profile.gid, "1027000001");
            assert_eq!(profile.open_id, open_id);
            assert_eq!(profile.account_id, "remote-account");
        }
    }

    #[test]
    fn official_friend_gids_are_unchanged_when_current_gid_is_unknown() {
        let captured = vec!["10001".to_owned(), "10002".to_owned()];

        let filtered = friend_gids_without_self(&captured, "  ");

        assert_eq!(filtered, captured);
    }

    #[test]
    fn official_friend_gids_can_be_empty_after_excluding_self() {
        let captured = vec!["10002".to_owned()];

        let filtered = friend_gids_without_self(&captured, "10002");

        assert!(filtered.is_empty());
    }

    #[tokio::test]
    async fn start_reservation_is_atomic() {
        let core = AppCore::new(std::env::temp_dir().join("qq-farm-start-reservation-test"));

        assert!(core.begin_start().await.is_ok());
        assert!(core.begin_start().await.is_err());
    }
}
