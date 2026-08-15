use crate::{
    app_state::AppCore,
    qq_identity::{self, LocalQqIdentity},
    server_sync::{ConnectionInfo, ServerClient},
    settings::{AppSettings, SettingsView},
    status::StatusPayload,
    updater::{self, UpdateInfo},
};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, State};

#[derive(Serialize)]
pub struct BootstrapPayload {
    pub settings: AppSettings,
    pub token_configured: bool,
    pub status: StatusPayload,
    pub startup_warning: Option<String>,
}

#[tauri::command]
pub async fn get_bootstrap(core: State<'_, Arc<AppCore>>) -> Result<BootstrapPayload, String> {
    let (settings, status, startup_warning) = core.bootstrap().await?;
    Ok(BootstrapPayload {
        settings: settings.settings,
        token_configured: settings.token_configured,
        status,
        startup_warning,
    })
}

#[tauri::command]
pub fn save_settings(
    core: State<'_, Arc<AppCore>>,
    settings: AppSettings,
    token: Option<String>,
) -> Result<SettingsView, String> {
    core.save_settings(settings, token)
}

#[tauri::command]
pub async fn test_connection(
    core: State<'_, Arc<AppCore>>,
    server_url: String,
    token: Option<String>,
) -> Result<ConnectionInfo, String> {
    let token = match token.filter(|value| !value.trim().is_empty()) {
        Some(token) => token,
        None => core
            .stored_token()?
            .ok_or_else(|| "请填写后台登录 Token".to_owned())?,
    };
    ServerClient::new()?
        .test_connection(&server_url, &token)
        .await
}

#[tauri::command]
pub async fn start_capture(core: State<'_, Arc<AppCore>>, app: AppHandle) -> Result<(), String> {
    core.inner().clone().start_capture(app).await
}

#[tauri::command]
pub async fn stop_capture(core: State<'_, Arc<AppCore>>, app: AppHandle) -> Result<(), String> {
    core.stop_capture(&app).await
}

#[tauri::command]
pub async fn cleanup_network(core: State<'_, Arc<AppCore>>, app: AppHandle) -> Result<(), String> {
    core.cleanup_network(&app).await
}

#[tauri::command]
pub async fn get_captured_code(core: State<'_, Arc<AppCore>>) -> Result<String, String> {
    core.captured_code().await
}

#[tauri::command]
pub async fn detect_local_qqs() -> Result<Vec<LocalQqIdentity>, String> {
    qq_identity::detect_stable_all_async().await
}

#[tauri::command]
pub async fn select_local_qq(
    core: State<'_, Arc<AppCore>>,
    app: AppHandle,
    qq_number: String,
) -> Result<LocalQqIdentity, String> {
    let qq_number = qq_number.trim();
    if qq_number.is_empty() {
        return Err("请选择本次登录的 QQ 账号".to_owned());
    }
    let identity = qq_identity::detect_selected_stable_async(qq_number).await?;
    core.select_local_qq(&app, identity).await
}

#[tauri::command]
pub async fn check_for_update() -> Result<UpdateInfo, String> {
    updater::check_for_update().await
}

#[tauri::command]
pub fn save_update_proxy(core: State<'_, Arc<AppCore>>, enabled: bool) -> Result<(), String> {
    core.set_update_proxy(enabled)
}

#[tauri::command]
pub async fn install_update(
    core: State<'_, Arc<AppCore>>,
    app: AppHandle,
    use_proxy: bool,
) -> Result<(), String> {
    let prepared = updater::prepare_update(use_proxy).await?;
    core.shutdown().await;
    updater::launch_prepared_update(prepared)?;
    core.mark_exiting();
    app.exit(0);
    Ok(())
}
