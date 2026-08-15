#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_state;
mod certificates;
mod commands;
mod friend_capture;
mod local_friend_capture;
mod proxy;
mod qq_identity;
mod qq_window;
mod server_sync;
mod settings;
mod status;
mod system_proxy;
mod update_installer;
mod updater;
mod windows;

use app_state::AppCore;
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::{Manager, RunEvent};

const RUNTIME_FILE_NAMES: [&str; 4] = [
    "settings.json",
    "system-proxy-backup.json",
    "temporary-ca.cer",
    "traffic-diagnostics.log",
];

fn data_dir_from_executable(executable: &Path) -> io::Result<PathBuf> {
    executable
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .ok_or_else(|| io::Error::other("无法确定程序所在目录"))
}

fn executable_data_dir() -> io::Result<PathBuf> {
    data_dir_from_executable(&std::env::current_exe()?)
}

fn migrate_legacy_data_dir(legacy_dir: &Path, data_dir: &Path) -> io::Result<()> {
    if legacy_dir == data_dir || !legacy_dir.exists() {
        return Ok(());
    }
    for name in RUNTIME_FILE_NAMES {
        let source = legacy_dir.join(name);
        if !source.exists() {
            continue;
        }
        let destination = data_dir.join(name);
        if destination.exists() {
            std::fs::remove_file(source)?;
            continue;
        }
        if std::fs::rename(&source, &destination).is_err() {
            std::fs::copy(&source, &destination)?;
            std::fs::remove_file(source)?;
        }
    }
    if legacy_dir.read_dir()?.next().is_none() {
        std::fs::remove_dir(legacy_dir)?;
    }
    Ok(())
}

fn main() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            let data_dir = executable_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            migrate_legacy_data_dir(&app.path().app_data_dir()?, &data_dir)?;
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
            commands::detect_local_qq,
            commands::check_for_update,
            commands::save_update_proxy,
            commands::install_update,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_runtime_files_next_to_the_executable() {
        let executable = Path::new("portable/QQFarmCodeHelper.exe");

        assert_eq!(
            data_dir_from_executable(executable).unwrap(),
            PathBuf::from("portable")
        );
    }

    #[test]
    fn migrates_legacy_settings_into_the_executable_directory() {
        let root = std::env::temp_dir().join(format!(
            "qq-farm-code-helper-migration-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let legacy = root.join("legacy");
        let portable = root.join("portable");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&portable).unwrap();
        std::fs::write(legacy.join("settings.json"), b"{}").unwrap();

        migrate_legacy_data_dir(&legacy, &portable).unwrap();

        assert_eq!(
            std::fs::read(portable.join("settings.json")).unwrap(),
            b"{}"
        );
        assert!(!legacy.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
