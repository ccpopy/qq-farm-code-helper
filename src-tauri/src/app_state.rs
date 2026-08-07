use crate::{
    certificates::CertificateManager,
    proxy,
    qq_identity::{self, LocalQqIdentity},
    server_sync::{ServerClient, SyncAccountInput},
    settings::{AppSettings, SettingsStore, SettingsView},
    status::StatusPayload,
    system_proxy::SystemProxyManager,
};
use std::{
    path::PathBuf,
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

const INITIAL_IDENTITY_WARNING: &str = "代理已启动，但捕获 Code 前尚未稳定确认 Windows QQ";

struct RuntimeState {
    cancellation: Option<CancellationToken>,
    task: Option<JoinHandle<()>>,
    identity_task: Option<JoinHandle<()>>,
    last_code: Option<String>,
    locked_identity: Option<LocalQqIdentity>,
    identity_warning: Option<String>,
    status: StatusPayload,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            cancellation: None,
            task: None,
            identity_task: None,
            last_code: None,
            locked_identity: None,
            identity_warning: None,
            status: StatusPayload::idle(),
        }
    }
}

pub struct AppCore {
    certificates: CertificateManager,
    diagnostics_path: PathBuf,
    proxy_manager: SystemProxyManager,
    settings: SettingsStore,
    runtime: Mutex<RuntimeState>,
    startup_warning: Mutex<Option<String>>,
    pub exiting: AtomicBool,
}

impl AppCore {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            certificates: CertificateManager::new(data_dir.clone()),
            diagnostics_path: data_dir.join("traffic-diagnostics.log"),
            proxy_manager: SystemProxyManager::new(data_dir.clone()),
            settings: SettingsStore::new(data_dir),
            runtime: Mutex::new(RuntimeState::default()),
            startup_warning: Mutex::new(None),
            exiting: AtomicBool::new(false),
        }
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

    pub fn set_update_proxy(&self, enabled: bool) -> Result<(), String> {
        self.settings.set_update_proxy(enabled)
    }

    pub async fn start_capture(self: &Arc<Self>, app: AppHandle) -> Result<(), String> {
        self.ensure_not_running().await?;
        let settings = self.settings.load()?;
        self.validate_sync_settings(&settings)?;
        self.publish(
            &app,
            StatusPayload::new(
                "preparing_proxy",
                "正在启动代理",
                "生成临时证书并保存当前系统代理设置…",
                false,
            ),
        )
        .await;

        let tls = self.prepare_certificate().await?;
        let listener = match proxy::bind(settings.proxy_port).await {
            Ok(listener) => listener,
            Err(error) => {
                let _ = self.cleanup_certificate().await;
                return Err(self.record_failure(&app, error, false).await);
            }
        };
        let cancellation = CancellationToken::new();
        let identity_cancellation = cancellation.clone();
        let (sender, receiver) = mpsc::channel(1);
        let _ = std::fs::remove_file(&self.diagnostics_path);
        let task = tokio::spawn(proxy::run(
            listener,
            tls,
            cancellation.clone(),
            sender,
            Some(self.diagnostics_path.clone()),
        ));
        self.store_transport(
            cancellation,
            task,
            None,
            Some(INITIAL_IDENTITY_WARNING.to_owned()),
        )
        .await;

        if let Err(error) = self.enable_system_proxy(settings.proxy_port).await {
            let _ = self.stop_transport().await;
            return Err(self.record_failure(&app, error, false).await);
        }
        self.publish(
            &app,
            StatusPayload::new(
                "waiting_login",
                "等待 Windows QQ 登录",
                waiting_login_detail(settings.auto_sync),
                false,
            ),
        )
        .await;
        self.start_identity_monitor(app.clone(), identity_cancellation)
            .await;
        self.spawn_capture_handler(app, receiver);
        Ok(())
    }

    pub async fn stop_capture(&self, app: &AppHandle) -> Result<(), String> {
        self.stop_transport().await?;
        self.clear_capture_identity().await;
        self.publish(
            app,
            StatusPayload::new(
                "stopped",
                "已停止",
                "系统代理和临时证书已恢复。",
                self.has_code().await,
            ),
        )
        .await;
        Ok(())
    }

    pub async fn cleanup_network(&self, app: &AppHandle) -> Result<(), String> {
        let _ = self.stop_transport().await;
        self.clear_capture_identity().await;
        self.recover_network().await?;
        self.publish(
            app,
            StatusPayload::new(
                "idle",
                "清理完成",
                "系统代理与临时证书均已清理。",
                self.has_code().await,
            ),
        )
        .await;
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

    pub async fn shutdown(&self) {
        let _ = self.stop_transport().await;
        let _ = self.recover_network().await;
    }

    fn spawn_capture_handler(
        self: &Arc<Self>,
        app: AppHandle,
        mut receiver: mpsc::Receiver<String>,
    ) {
        let core = self.clone();
        tokio::spawn(async move {
            if let Some(code) = receiver.recv().await {
                core.handle_captured_code(app, code).await;
            }
        });
    }

    async fn handle_captured_code(&self, app: AppHandle, code: String) {
        self.set_code(Some(code.clone())).await;
        self.publish(
            &app,
            StatusPayload::new(
                "code_captured",
                "已获取 Code",
                "官方登录请求已阻断，正在恢复系统网络设置。",
                true,
            ),
        )
        .await;

        if let Err(error) = self.stop_transport().await {
            self.record_failure(&app, error, false).await;
            return;
        }
        self.publish(
            &app,
            StatusPayload::new(
                "code_captured",
                "已获取 Code",
                "系统网络已恢复，正在确认捕获账号…",
                true,
            ),
        )
        .await;
        self.revalidate_capture_identity().await;
        let settings = match self.settings.load() {
            Ok(settings) => settings,
            Err(error) => {
                self.record_failure(&app, error, true).await;
                return;
            }
        };
        if settings.auto_sync {
            self.sync_captured_code(&app, &settings, &code).await;
        } else {
            self.publish(
                &app,
                StatusPayload::new(
                    "completed",
                    "Code 已就绪",
                    "自动同步已关闭，可点击“复制 Code”手动使用。",
                    true,
                ),
            )
            .await;
        }
    }

    async fn sync_captured_code(&self, app: &AppHandle, settings: &AppSettings, code: &str) {
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
            StatusPayload::new(
                "syncing",
                "正在同步服务器",
                "同步 Code，并等待远程 GID/OpenID 确认身份…",
                true,
            ),
        )
        .await;
        let result = self.sync_code(settings, code, &locked_identity).await;
        match result {
            Ok(profile) => {
                self.set_code(None).await;
                let detail = sync_completion_detail(&profile, &locked_identity);
                self.publish(
                    app,
                    StatusPayload::new("completed", "同步完成", detail, false)
                        .with_profile(profile),
                )
                .await;
            }
            Err(error) => {
                self.record_failure(app, format!("同步失败，Code 已保留在内存中: {error}"), true)
                    .await;
            }
        }
    }

    async fn publish_identity_blocked(&self, app: &AppHandle, reason: &str) {
        self.publish(
            app,
            StatusPayload::new(
                "identity_changed",
                "未同步：QQ 账号已变化",
                format!(
                    "Code 已保留在内存中，且没有向服务器创建账号。{reason}。请确认目标 QQ 后重新启动获取。"
                ),
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

    async fn ensure_not_running(&self) -> Result<(), String> {
        if self.runtime.lock().await.cancellation.is_some() {
            Err("获取任务已经在运行".to_owned())
        } else {
            Ok(())
        }
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
        locked_identity: Option<LocalQqIdentity>,
        identity_warning: Option<String>,
    ) {
        let mut runtime = self.runtime.lock().await;
        runtime.cancellation = Some(cancellation);
        runtime.task = Some(task);
        runtime.identity_task = None;
        runtime.last_code = None;
        runtime.locked_identity = locked_identity;
        runtime.identity_warning = identity_warning;
    }

    async fn stop_transport(&self) -> Result<(), String> {
        let (cancellation, task, identity_task) = {
            let mut runtime = self.runtime.lock().await;
            (
                runtime.cancellation.take(),
                runtime.task.take(),
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
                match qq_identity::detect_stable_async().await {
                    Ok(identity) => core.adopt_waiting_identity(&app, identity).await,
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

    async fn adopt_waiting_identity(&self, app: &AppHandle, identity: LocalQqIdentity) {
        let mut runtime = self.runtime.lock().await;
        if runtime.status.phase != "waiting_login" {
            return;
        }
        let (changed, previous) = replace_locked_identity(&mut runtime.locked_identity, &identity);
        runtime.identity_warning = None;
        if !changed {
            return;
        }
        let detail = waiting_identity_detail(previous.as_deref(), &identity);
        let status = StatusPayload::new("waiting_login", "QQ 已确认，等待农场登录", detail, false);
        runtime.status = status.clone();
        let _ = app.emit("capture-status", status);
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
            "等待 Windows QQ 登录",
            format!(
                "本地代理仍在运行，但尚未确认当前 QQ：{error}。若要自动同步，请确认成功后再进入农场；未确认时 Code 不会提交服务器。"
            ),
            false,
        );
        runtime.status = status.clone();
        let _ = app.emit("capture-status", status);
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
        let result = detect_current_qq_with_retry(2).await;
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
        self.runtime.lock().await.status = status.clone();
        let _ = app.emit("capture-status", status);
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

fn waiting_login_detail(auto_sync: bool) -> &'static str {
    if auto_sync {
        "本地代理已启动。现在可以打开或登录 Windows QQ；检测并锁定当前账号后，再进入 QQ 农场。未确认账号前捕获到 Code 也不会提交服务器。"
    } else {
        "本地代理已启动。现在可以打开或登录 Windows QQ，再进入 QQ 农场；自动同步已关闭，本次只获取 Code。"
    }
}

fn waiting_identity_detail(previous: Option<&str>, identity: &LocalQqIdentity) -> String {
    let nickname = if identity.nickname.is_empty() {
        "未读取到昵称"
    } else {
        identity.nickname.as_str()
    };
    match previous {
        Some(previous) => format!(
            "已将锁定账号从 QQ {previous} 切换为 QQ {}（{nickname}）。请用当前账号进入 QQ 农场。",
            identity.qq_number
        ),
        None => format!(
            "已稳定确认并锁定 QQ {}（{nickname}）。现在可以进入 QQ 农场。",
            identity.qq_number
        ),
    }
}

async fn detect_current_qq_with_retry(max_attempts: usize) -> Result<LocalQqIdentity, String> {
    let mut last_error = "未能确认当前 QQ".to_owned();
    for attempt in 0..max_attempts {
        match qq_identity::detect_stable_async().await {
            Ok(identity) => return Ok(identity),
            Err(error) => last_error = error,
        }
        if attempt + 1 < max_attempts {
            sleep(Duration::from_millis(250)).await;
        }
    }
    Err(last_error)
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
        format!("远程已确认 {remote_name}（{identity}）")
    } else {
        format!("已同步到 {remote_name}，远程 GID/OpenID 仍在等待回填")
    };
    format!(
        "{remote}；本机前台身份确认并绑定 QQ {}。",
        locked_identity.qq_number
    )
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
    fn auto_sync_waiting_copy_allows_proxy_before_qq_but_keeps_upload_guard() {
        let detail = waiting_login_detail(true);

        assert!(detail.contains("本地代理已启动"));
        assert!(detail.contains("打开或登录 Windows QQ"));
        assert!(detail.contains("不会提交服务器"));
    }

    #[test]
    fn first_stable_identity_is_treated_as_an_initial_lock() {
        let identity = identity("3170105001", "落");
        let detail = waiting_identity_detail(None, &identity);

        assert!(detail.contains("已稳定确认并锁定"));
        assert!(!detail.contains("切换"));
    }
}
