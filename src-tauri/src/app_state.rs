use crate::{
    certificates::CertificateManager,
    proxy, qq_identity,
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

struct RuntimeState {
    cancellation: Option<CancellationToken>,
    task: Option<JoinHandle<()>>,
    last_code: Option<String>,
    status: StatusPayload,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            cancellation: None,
            task: None,
            last_code: None,
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
        let (sender, receiver) = mpsc::channel(1);
        let _ = std::fs::remove_file(&self.diagnostics_path);
        let task = tokio::spawn(proxy::run(
            listener,
            tls,
            cancellation.clone(),
            sender,
            Some(self.diagnostics_path.clone()),
        ));
        self.store_transport(cancellation, task).await;

        if let Err(error) = self.enable_system_proxy(settings.proxy_port).await {
            let _ = self.stop_transport().await;
            return Err(self.record_failure(&app, error, false).await);
        }
        self.publish(
            &app,
            StatusPayload::new(
                "waiting_login",
                "等待 Windows QQ 登录",
                "现在完全退出并重新打开 QQ，然后进入 QQ 农场。",
                false,
            ),
        )
        .await;
        self.spawn_capture_handler(app, receiver);
        Ok(())
    }

    pub async fn stop_capture(&self, app: &AppHandle) -> Result<(), String> {
        self.stop_transport().await?;
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
        self.publish(
            app,
            StatusPayload::new(
                "syncing",
                "正在同步服务器",
                "向 qq-farm-bot 新增 QQ 账号并自动启动…",
                true,
            ),
        )
        .await;
        let result = self.sync_code(settings, code).await;
        match result {
            Ok(profile) => {
                self.set_code(None).await;
                self.publish(
                    app,
                    StatusPayload::new(
                        "completed",
                        "同步完成",
                        format!(
                            "已同步到 {}。",
                            if profile.nickname.is_empty() {
                                profile.account_name.as_str()
                            } else {
                                profile.nickname.as_str()
                            }
                        ),
                        false,
                    )
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

    async fn sync_code(
        &self,
        settings: &AppSettings,
        code: &str,
    ) -> Result<crate::server_sync::AccountProfile, String> {
        let token = self
            .settings
            .token()?
            .ok_or_else(|| "未配置服务器 Token".to_owned())?;
        let detected = detect_current_qq_with_retry().await;
        let qq_number = detected
            .as_ref()
            .map(|identity| identity.qq_number.as_str())
            .filter(|value| !value.is_empty())
            .unwrap_or(settings.qq_number.as_str());
        let matching_identity = detected.as_ref();
        ServerClient::new()?
            .sync_code(
                &settings.server_url,
                &token,
                SyncAccountInput {
                    account_name: &settings.account_name,
                    qq_number,
                    nickname: matching_identity
                        .map(|identity| identity.nickname.as_str())
                        .unwrap_or_default(),
                    avatar_url: matching_identity
                        .map(|identity| identity.avatar_url.as_str())
                        .unwrap_or_default(),
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

    async fn store_transport(&self, cancellation: CancellationToken, task: JoinHandle<()>) {
        let mut runtime = self.runtime.lock().await;
        runtime.cancellation = Some(cancellation);
        runtime.task = Some(task);
        runtime.last_code = None;
    }

    async fn stop_transport(&self) -> Result<(), String> {
        let (cancellation, task) = {
            let mut runtime = self.runtime.lock().await;
            (runtime.cancellation.take(), runtime.task.take())
        };
        if let Some(cancellation) = cancellation {
            cancellation.cancel();
        }
        if let Some(task) = task {
            let _ = task.await;
        }
        self.recover_network().await
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

async fn detect_current_qq_with_retry() -> Option<qq_identity::LocalQqIdentity> {
    for attempt in 0..4 {
        if let Ok(identity) = qq_identity::detect() {
            return Some(identity);
        }
        if attempt < 3 {
            sleep(Duration::from_millis(250)).await;
        }
    }
    None
}
