mod app_state;
mod certificates;
mod commands;
mod proxy;
mod server_sync;
mod settings;
mod status;
mod system_proxy;
mod windows;

use app_state::AppCore;
use std::sync::Arc;
use tauri::{Manager, RunEvent};

fn main() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let app = tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let core = Arc::new(AppCore::new(data_dir));
            core.recover_stale();
            app.manage(core);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_bootstrap,
            commands::save_settings,
            commands::test_connection,
            commands::start_capture,
            commands::stop_capture,
            commands::cleanup_network,
            commands::get_captured_code,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build QQ Farm Code Helper");

    app.run(|app_handle, event| {
        if let RunEvent::ExitRequested { api, .. } = event {
            let core = app_handle.state::<Arc<AppCore>>().inner().clone();
            if !core.mark_exiting() {
                api.prevent_exit();
                let handle = app_handle.clone();
                tauri::async_runtime::spawn(async move {
                    core.shutdown().await;
                    handle.exit(0);
                });
            }
        }
    });
}
