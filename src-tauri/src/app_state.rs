use crate::{
    certificates::CertificateManager,
    protocol_capture::{CaptureSummary, ProtocolCaptureSession},
    proxy::{self, ProxyMode},
    qq_identity::{self, LocalQqIdentity},
    qq_login_history::QqLoginHistory,
    server_sync::{FriendGidCleanupResult, FriendGidSyncResult, ServerClient, SyncAccountInput},
    settings::{AppSettings, SettingsStore, SettingsView},
    status::StatusPayload,
    system_proxy::SystemProxyManager,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::{AppHandle, Emitter};
use tokio::{
    sync::{Mutex, mpsc},
    task::JoinHandle,
    time::{Duration, sleep},
};
use tokio_util::sync::CancellationToken;

/// 这些阶段的状态描述的是某个具体账号，需要带上 target_qq。
const ACCOUNT_PHASES: [&str; 5] = [
    "waiting_login",
    "code_captured",
    "syncing",
    "completed",
    "identity_changed",
];

struct RuntimeState {
    starting: bool,
    cancellation: Option<CancellationToken>,
    task: Option<JoinHandle<()>>,
    capture_task: Option<JoinHandle<()>>,
    identity_task: Option<JoinHandle<()>>,
    armed: Option<Arc<AtomicBool>>,
    capture_hint: &'static str,
    capture_target: Option<String>,
    last_code: Option<String>,
    selected_qq_number: Option<String>,
    locked_identity: Option<LocalQqIdentity>,
    identity_warning: Option<String>,
    protocol_session: Option<Arc<ProtocolCaptureSession>>,
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
            armed: None,
            capture_hint: "",
            capture_target: None,
            last_code: None,
            selected_qq_number: None,
            locked_identity: None,
            identity_warning: None,
            protocol_session: None,
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
        let settings = self.settings.view()?;
        let runtime = self.runtime.lock().await;
        let warning = self.startup_warning.lock().await.clone();
        Ok((settings, runtime.status.clone(), warning))
    }

    pub fn save_settings(
        &self,
        settings: AppSettings,
        token: Option<String>,
    ) -> Result<SettingsView, String> {
        self.settings.save(settings, token)
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
        let result = if settings.protocol_capture {
            self.start_protocol_capture(app, settings).await
        } else {
            self.start_code_capture(app, settings, None).await
        };
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
        let result = self.start_code_capture(app, settings, Some(identity)).await;
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
            return Err("协议模式运行中，请先停止".to_owned());
        }
        let Some(armed) = runtime.armed.clone() else {
            return Err("代理状态异常，请停止后重试".to_owned());
        };
        if capture_in_progress(runtime.status.phase, &armed) {
            return Err("正在处理上一个账号的登录，请稍候".to_owned());
        }
        runtime.selected_qq_number = Some(identity.qq_number.clone());
        runtime.locked_identity = Some(identity.clone());
        runtime.identity_warning = None;
        runtime.capture_target = None;
        runtime.last_code = None;
        armed.store(true, Ordering::SeqCst);
        let status = StatusPayload::new(
            "waiting_login",
            "等待农场登录",
            waiting_login_detail(identity, runtime.capture_hint),
            false,
        );
        emit_status(app, &mut runtime, status);
        Ok(true)
    }

    pub async fn cancel_account_sync(&self, app: &AppHandle) -> Result<(), String> {
        let mut runtime = self.runtime.lock().await;
        if runtime.status.phase != "waiting_login" {
            return Ok(());
        }
        let Some(armed) = runtime.armed.clone() else {
            return Ok(());
        };
        if !armed.swap(false, Ordering::SeqCst) {
            return Err("已拦截到农场登录，正在处理，请稍候".to_owned());
        }
        runtime.locked_identity = None;
        runtime.identity_warning = None;
        emit_status(app, &mut runtime, ready_status());
        Ok(())
    }

    async fn start_code_capture(
        self: &Arc<Self>,
        app: AppHandle,
        settings: AppSettings,
        identity: Option<LocalQqIdentity>,
    ) -> Result<(), String> {
        let preparing_detail = format!("代理端口 {}", settings.proxy_port);
        let (tls, listener) = self
            .prepare_proxy_listener(&app, settings.proxy_port, &preparing_detail)
            .await?;
        let cancellation = CancellationToken::new();
        let identity_cancellation = cancellation.clone();
        let armed = Arc::new(AtomicBool::new(identity.is_some()));
        let (sender, receiver) = mpsc::channel(4);
        let _ = std::fs::remove_file(&self.diagnostics_path);
        let mode = ProxyMode::CaptureCode {
            captured: sender,
            armed: armed.clone(),
            capture_friend_gids: settings.auto_sync && settings.sync_official_friends,
            diagnostics_path: Some(self.diagnostics_path.clone()),
        };
        let task = tokio::spawn(proxy::run(listener, tls, cancellation.clone(), mode));
        let hint = capture_hint(settings.auto_sync, settings.sync_official_friends);
        self.store_transport(cancellation, task, identity.clone(), armed, hint)
            .await;

        if let Err(error) = self.enable_system_proxy(settings.proxy_port).await {
            let _ = self.stop_transport().await;
            return Err(self.record_failure(&app, error, false).await);
        }
        let status = match identity {
            Some(identity) => StatusPayload::new(
                "waiting_login",
                "等待农场登录",
                waiting_login_detail(&identity, hint),
                false,
            ),
            None => ready_status(),
        };
        self.publish(&app, status).await;
        self.start_identity_monitor(app.clone(), identity_cancellation)
            .await;
        self.spawn_capture_handler(app, receiver).await;
        Ok(())
    }

    async fn start_protocol_capture(
        self: &Arc<Self>,
        app: AppHandle,
        settings: AppSettings,
    ) -> Result<(), String> {
        let preparing_detail = format!("代理端口 {}", settings.proxy_port);
        let (tls, listener) = self
            .prepare_proxy_listener(&app, settings.proxy_port, &preparing_detail)
            .await?;
        let session = match ProtocolCaptureSession::create(&self.protocol_capture_root) {
            Ok(session) => session,
            Err(error) => {
                let _ = self.cleanup_certificate().await;
                return Err(self.record_failure(&app, error, false).await);
            }
        };
        let cancellation = CancellationToken::new();
        let mode = ProxyMode::ProtocolCapture {
            session: session.clone(),
        };
        let task = tokio::spawn(proxy::run(listener, tls, cancellation.clone(), mode));
        self.store_protocol_transport(cancellation, task, session.clone())
            .await;

        if let Err(error) = self.enable_system_proxy(settings.proxy_port).await {
            let _ = self.stop_transport().await;
            self.take_protocol_session().await;
            return Err(self.record_failure(&app, error, false).await);
        }
        self.publish(
            &app,
            StatusPayload::new(
                "protocol_listening",
                "协议模式",
                protocol_listening_detail(settings.proxy_port),
                false,
            )
            .with_capture_directory(session.directory()),
        )
        .await;
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
        self.clear_capture_identity().await;
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
        self.clear_capture_identity().await;
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

    pub async fn captured_code(&self) -> Result<String, String> {
        self.runtime
            .lock()
            .await
            .last_code
            .clone()
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
                    proxy::CaptureEvent::Intercepted => core.handle_intercepted(&app).await,
                    proxy::CaptureEvent::Captured(captured) => {
                        core.handle_captured_login(&app, captured).await;
                    }
                }
            }
        });
        runtime.capture_task = Some(task);
    }

    async fn send_friend_gids(
        &self,
        account_id: &str,
        gids: &[String],
    ) -> Result<FriendGidSyncResult, String> {
        let settings = self.settings.load()?;
        let token = self
            .settings
            .token()?
            .ok_or_else(|| "后台登录 Token 已不存在".to_owned())?;
        ServerClient::new()?
            .batch_add_friend_gids(&settings.server_url, &token, account_id, gids)
            .await
    }

    async fn remove_friend_gids(
        &self,
        account_id: &str,
        gids: &[String],
    ) -> Result<FriendGidCleanupResult, String> {
        let settings = self.settings.load()?;
        let token = self
            .settings
            .token()?
            .ok_or_else(|| "后台登录 Token 已不存在".to_owned())?;
        ServerClient::new()?
            .batch_remove_friend_gids(&settings.server_url, &token, account_id, gids)
            .await
    }

    async fn handle_intercepted(&self, app: &AppHandle) {
        let mut runtime = self.runtime.lock().await;
        runtime.capture_target = runtime.selected_qq_number.clone();
        let status = StatusPayload::new("code_captured", "已获取 Code", "正在读取登录信息", false);
        emit_status(app, &mut runtime, status);
    }

    async fn handle_captured_login(&self, app: &AppHandle, captured: proxy::CapturedLogin) {
        self.set_code(Some(captured.code.clone())).await;
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
            StatusPayload::new("code_captured", "已获取 Code", captured_detail, true),
        )
        .await;
        self.revalidate_capture_identity().await;
        let settings = match self.settings.load() {
            Ok(settings) => settings,
            Err(error) => {
                self.record_failure(app, error, true).await;
                return;
            }
        };
        if settings.auto_sync {
            self.sync_captured_login(app, &settings, &captured).await;
        } else {
            self.publish(
                app,
                StatusPayload::new("completed", "Code 可复制", "可继续选择其他账号", true),
            )
            .await;
        }
    }

    async fn sync_captured_login(
        &self,
        app: &AppHandle,
        settings: &AppSettings,
        captured: &proxy::CapturedLogin,
    ) {
        let (locked_identity, identity_warning) = self.capture_identity().await;
        let Some(locked_identity) = locked_identity else {
            self.publish_identity_blocked(
                app,
                identity_warning
                    .as_deref()
                    .unwrap_or("捕获 Code 时无法确认当前 QQ"),
            )
            .await;
            return;
        };
        self.publish(
            app,
            StatusPayload::new("syncing", "正在同步", "Code 已获取\n正在确认远程账号", true),
        )
        .await;
        let result = self
            .sync_code(settings, &captured.code, &locked_identity)
            .await;
        match result {
            Ok(mut profile) => {
                apply_captured_identity(&mut profile, captured);
                self.set_code(None).await;
                let mut detail = sync_completion_detail(&profile, &locked_identity);
                if settings.sync_official_friends {
                    detail.push_str(
                        &self
                            .sync_official_friend_gids(app, &profile, captured)
                            .await,
                    );
                }
                self.publish(
                    app,
                    StatusPayload::new("completed", "同步完成", detail, false)
                        .with_profile(profile),
                )
                .await;
            }
            Err(error) => {
                self.record_failure(app, format!("同步失败：{error}\nCode 可复制"), true)
                    .await;
            }
        }
    }

    async fn sync_official_friend_gids(
        &self,
        app: &AppHandle,
        profile: &crate::server_sync::AccountProfile,
        captured: &proxy::CapturedLogin,
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
                .with_profile(profile.clone()),
        )
        .await;

        let own_gids = [own_gid.to_owned()];
        let cleanup_result = self
            .remove_friend_gids(&profile.account_id, &own_gids)
            .await;
        let add_result = if friend_gids.is_empty() {
            None
        } else {
            Some(
                self.send_friend_gids(&profile.account_id, &friend_gids)
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

    async fn publish_identity_blocked(&self, app: &AppHandle, reason: &str) {
        self.publish(
            app,
            StatusPayload::new(
                "identity_changed",
                "未同步",
                format!("QQ 已变化：{reason}\nCode 可复制"),
                true,
            ),
        )
        .await;
    }

    async fn sync_code(
        &self,
        settings: &AppSettings,
        code: &str,
        locked_identity: &LocalQqIdentity,
    ) -> Result<crate::server_sync::AccountProfile, String> {
        let token = self
            .settings
            .token()?
            .ok_or_else(|| "未配置服务器 Token".to_owned())?;
        ServerClient::new()?
            .sync_code(
                &settings.server_url,
                &token,
                SyncAccountInput {
                    account_name: &settings.account_name,
                    qq_number: &locked_identity.qq_number,
                    nickname: &locked_identity.nickname,
                    avatar_url: &locked_identity.avatar_url,
                    code,
                },
            )
            .await
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

    async fn store_transport(
        &self,
        cancellation: CancellationToken,
        task: JoinHandle<()>,
        identity: Option<LocalQqIdentity>,
        armed: Arc<AtomicBool>,
        capture_hint: &'static str,
    ) {
        let mut runtime = self.runtime.lock().await;
        runtime.cancellation = Some(cancellation);
        runtime.task = Some(task);
        runtime.identity_task = None;
        runtime.armed = Some(armed);
        runtime.capture_hint = capture_hint;
        runtime.capture_target = None;
        runtime.last_code = None;
        runtime.selected_qq_number = identity.as_ref().map(|identity| identity.qq_number.clone());
        runtime.locked_identity = identity;
        runtime.identity_warning = None;
        runtime.protocol_session = None;
    }

    async fn store_protocol_transport(
        &self,
        cancellation: CancellationToken,
        task: JoinHandle<()>,
        session: Arc<ProtocolCaptureSession>,
    ) {
        let mut runtime = self.runtime.lock().await;
        runtime.cancellation = Some(cancellation);
        runtime.task = Some(task);
        runtime.capture_task = None;
        runtime.identity_task = None;
        runtime.armed = None;
        runtime.capture_target = None;
        runtime.last_code = None;
        runtime.selected_qq_number = None;
        runtime.locked_identity = None;
        runtime.identity_warning = None;
        runtime.protocol_session = Some(session);
    }

    async fn take_protocol_session(&self) -> Option<Arc<ProtocolCaptureSession>> {
        self.runtime.lock().await.protocol_session.take()
    }

    async fn stop_transport(&self) -> Result<(), String> {
        let _stopping = self.transport_stop.lock().await;
        let (cancellation, task, capture_task, identity_task) = {
            let mut runtime = self.runtime.lock().await;
            runtime.armed = None;
            runtime.capture_target = None;
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
                match qq_identity::detect_stable_all_async(core.qq_login_history.clone()).await {
                    Ok(identities) => core.adopt_waiting_identities(&app, identities).await,
                    Err(error) => {
                        core.mark_waiting_identity_unconfirmed(&app, error).await;
                    }
                }
                tokio::select! {
                    _ = cancellation.cancelled() => break,
                    _ = sleep(Duration::from_millis(500)) => {}
                }
            }
        });
        self.runtime.lock().await.identity_task = Some(task);
    }

    async fn adopt_waiting_identities(&self, app: &AppHandle, identities: Vec<LocalQqIdentity>) {
        let selected_qq_number = self.runtime.lock().await.selected_qq_number.clone();
        match choose_waiting_identity(&identities, selected_qq_number.as_deref()) {
            Ok(identity) => self.adopt_waiting_identity(app, identity).await,
            Err(error) => self.mark_waiting_identity_unconfirmed(app, error).await,
        }
    }

    async fn adopt_waiting_identity(&self, app: &AppHandle, identity: LocalQqIdentity) {
        let mut runtime = self.runtime.lock().await;
        if runtime.status.phase != "waiting_login" {
            return;
        }
        let (changed, _) = replace_locked_identity(&mut runtime.locked_identity, &identity);
        runtime.selected_qq_number = Some(identity.qq_number.clone());
        runtime.identity_warning = None;
        if !changed {
            return;
        }
        let detail = waiting_login_detail(&identity, runtime.capture_hint);
        let status = StatusPayload::new("waiting_login", "等待农场登录", detail, false);
        emit_status(app, &mut runtime, status);
    }

    async fn mark_waiting_identity_unconfirmed(&self, app: &AppHandle, error: String) {
        let mut runtime = self.runtime.lock().await;
        if runtime.status.phase != "waiting_login" {
            return;
        }
        let changed = runtime.locked_identity.take().is_some()
            || runtime.identity_warning.as_deref() != Some(error.as_str());
        runtime.identity_warning = Some(error.clone());
        if !changed {
            return;
        }
        let status = StatusPayload::new(
            "waiting_login",
            "等待确认 QQ",
            format!("代理仍在运行\n{error}"),
            false,
        );
        emit_status(app, &mut runtime, status);
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

    async fn set_code(&self, code: Option<String>) {
        self.runtime.lock().await.last_code = code;
    }

    async fn has_code(&self) -> bool {
        self.runtime.lock().await.last_code.is_some()
    }

    async fn capture_identity(&self) -> (Option<LocalQqIdentity>, Option<String>) {
        let runtime = self.runtime.lock().await;
        (
            runtime.locked_identity.clone(),
            runtime.identity_warning.clone(),
        )
    }

    async fn clear_capture_identity(&self) {
        let mut runtime = self.runtime.lock().await;
        runtime.locked_identity = None;
        runtime.identity_warning = None;
    }

    async fn revalidate_capture_identity(&self) {
        let (locked_identity, _) = self.capture_identity().await;
        let Some(locked_identity) = locked_identity else {
            return;
        };
        let result = detect_selected_qq_with_retry(
            self.qq_login_history.clone(),
            &locked_identity.qq_number,
            2,
        )
        .await;
        let mut runtime = self.runtime.lock().await;
        match result {
            Ok(current) if current.qq_number == locked_identity.qq_number => {
                runtime.locked_identity = Some(current);
                runtime.identity_warning = None;
            }
            Ok(current) => {
                runtime.locked_identity = None;
                runtime.identity_warning = Some(format!(
                    "捕获前锁定 QQ {}，捕获 Code 后检测到 QQ {}，账号已变化",
                    locked_identity.qq_number, current.qq_number
                ));
            }
            Err(error) => {
                runtime.locked_identity = None;
                runtime.identity_warning = Some(format!(
                    "捕获 Code 后无法再次确认 QQ {}：{}",
                    locked_identity.qq_number, error
                ));
            }
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

    pub fn mark_exiting(&self) -> bool {
        self.exiting.swap(true, Ordering::SeqCst)
    }
}

fn replace_locked_identity(
    locked_identity: &mut Option<LocalQqIdentity>,
    identity: &LocalQqIdentity,
) -> (bool, Option<String>) {
    let previous = locked_identity
        .as_ref()
        .map(|current| current.qq_number.clone());
    let changed = previous.as_deref() != Some(identity.qq_number.as_str());
    *locked_identity = Some(identity.clone());
    (changed, previous)
}

fn emit_status(app: &AppHandle, runtime: &mut RuntimeState, mut status: StatusPayload) {
    status.session_active = runtime.cancellation.is_some();
    if status.target_qq.is_none() {
        status.target_qq = if ACCOUNT_PHASES.contains(&status.phase) {
            runtime
                .capture_target
                .clone()
                .or_else(|| runtime.selected_qq_number.clone())
        } else if status.phase == "error" {
            // 只有拦截后的失败属于某个账号；启动阶段的失败不归到任何账号。
            runtime.capture_target.clone()
        } else {
            None
        };
    }
    runtime.status = status.clone();
    let _ = app.emit("capture-status", status);
}

/// 已拦截登录但结果尚未发布时禁止切换账号，否则 Code 会被记到新选的账号上。
fn capture_in_progress(phase: &str, armed: &AtomicBool) -> bool {
    matches!(phase, "code_captured" | "syncing")
        || (phase == "waiting_login" && !armed.load(Ordering::SeqCst))
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

fn choose_waiting_identity(
    identities: &[LocalQqIdentity],
    selected_qq_number: Option<&str>,
) -> Result<LocalQqIdentity, String> {
    if identities.is_empty() {
        return Err("未检测到可确认的 Windows QQ 账号".to_owned());
    }
    if let Some(selected_qq_number) = selected_qq_number {
        return identities
            .iter()
            .find(|identity| identity.qq_number == selected_qq_number)
            .cloned()
            .ok_or_else(|| {
                format!("所选 QQ {selected_qq_number} 已不在当前登录窗口中，请重新选择")
            });
    }
    if identities.len() == 1 {
        return Ok(identities[0].clone());
    }
    Err(format!(
        "检测到 {} 个 Windows QQ 账号，请选择右侧下拉列表中的本次登录账号",
        identities.len()
    ))
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
    fn waiting_identity_lock_adopts_a_stable_account_switch() {
        let mut locked = Some(identity("1343475483", "芜"));
        let next = identity("3170105001", "落");
        let (changed, previous) = replace_locked_identity(&mut locked, &next);

        assert!(changed);
        assert_eq!(previous.as_deref(), Some("1343475483"));
        assert_eq!(locked.unwrap().qq_number, "3170105001");
    }

    #[test]
    fn waiting_identity_lock_refreshes_without_reporting_a_switch() {
        let mut locked = Some(identity("3170105001", "落"));
        let refreshed = identity("3170105001", "落");
        let (changed, previous) = replace_locked_identity(&mut locked, &refreshed);

        assert!(!changed);
        assert_eq!(previous.as_deref(), Some("3170105001"));
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
    fn switching_accounts_is_blocked_while_a_login_is_being_processed() {
        let armed = AtomicBool::new(true);
        assert!(!capture_in_progress("waiting_login", &armed));
        assert!(!capture_in_progress("completed", &armed));
        assert!(capture_in_progress("syncing", &armed));

        armed.store(false, Ordering::SeqCst);
        assert!(capture_in_progress("waiting_login", &armed));
        assert!(!capture_in_progress("ready", &armed));
        assert!(!capture_in_progress("error", &armed));
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
    fn one_visible_account_is_selected_automatically() {
        let candidates = vec![identity("3170105001", "账号一")];

        let selected = choose_waiting_identity(&candidates, None).unwrap();

        assert_eq!(selected.qq_number, "3170105001");
    }

    #[test]
    fn multiple_visible_accounts_require_an_explicit_selection() {
        let candidates = vec![
            identity("1343475483", "账号一"),
            identity("3170105001", "账号二"),
        ];

        let error = choose_waiting_identity(&candidates, None).unwrap_err();

        assert!(error.contains("请选择"));
    }

    #[test]
    fn multiple_visible_accounts_use_the_selected_qq_number() {
        let candidates = vec![
            identity("1343475483", "账号一"),
            identity("3170105001", "账号二"),
        ];

        let selected = choose_waiting_identity(&candidates, Some("3170105001")).unwrap();

        assert_eq!(selected.nickname, "账号二");
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
