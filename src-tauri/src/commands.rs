use crate::{
    app_state::AppCore,
    qq_identity::{self, LocalQqIdentity},
    server_sync::{ConnectionInfo, ServerClient},
    settings::{AppSettings, SettingsView},
    status::StatusPayload,
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
pub fn detect_local_qq() -> Result<LocalQqIdentity, String> {
    qq_identity::detect()
}
