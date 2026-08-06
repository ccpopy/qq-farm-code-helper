use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use url::Url;

const KEYRING_SERVICE: &str = "qq-farm-code-helper";
const KEYRING_USER: &str = "qq-farm-bot-server-token";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub server_url: String,
    pub account_name: String,
    pub auto_sync: bool,
    pub proxy_port: u16,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            account_name: "Windows QQ".to_owned(),
            auto_sync: true,
            proxy_port: 8899,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingsView {
    pub settings: AppSettings,
    pub token_configured: bool,
}

pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            path: data_dir.join("settings.json"),
        }
    }

    pub fn load(&self) -> Result<AppSettings, String> {
        if !self.path.exists() {
            return Ok(AppSettings::default());
        }
        let content =
            fs::read_to_string(&self.path).map_err(|error| format!("读取设置失败: {error}"))?;
        let mut settings: AppSettings =
            serde_json::from_str(&content).map_err(|error| format!("设置文件格式无效: {error}"))?;
        normalize_settings(&mut settings)?;
        Ok(settings)
    }

    pub fn save(
        &self,
        mut settings: AppSettings,
        token: Option<String>,
    ) -> Result<SettingsView, String> {
        normalize_settings(&mut settings)?;
        if let Some(token) = token {
            let token = token.trim();
            if !token.is_empty() {
                token_entry()?.set_password(token).map_err(keyring_error)?;
            }
        }
        write_settings_atomic(&self.path, &settings)?;
        self.view_with(settings)
    }

    pub fn view(&self) -> Result<SettingsView, String> {
        self.view_with(self.load()?)
    }

    pub fn token(&self) -> Result<Option<String>, String> {
        let entry = token_entry()?;
        match entry.get_password() {
            Ok(value) if !value.trim().is_empty() => Ok(Some(value)),
            Ok(_) | Err(KeyringError::NoEntry) => Ok(None),
            Err(error) => Err(keyring_error(error)),
        }
    }

    fn view_with(&self, settings: AppSettings) -> Result<SettingsView, String> {
        Ok(SettingsView {
            settings,
            token_configured: self.token()?.is_some(),
        })
    }
}

pub fn normalize_server_url(value: &str) -> Result<String, String> {
    let value = value.trim().trim_end_matches('/');
    if value.is_empty() {
        return Ok(String::new());
    }
    let parsed = Url::parse(value).map_err(|_| "服务器地址格式无效".to_owned())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("服务器地址必须是有效的 http 或 https 地址".to_owned());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() || !parsed.username().is_empty() {
        return Err("服务器地址不能包含账号、查询参数或片段".to_owned());
    }
    Ok(value.to_owned())
}

fn normalize_settings(settings: &mut AppSettings) -> Result<(), String> {
    settings.server_url = normalize_server_url(&settings.server_url)?;
    settings.account_name = settings.account_name.trim().to_owned();
    if settings.account_name.is_empty() {
        settings.account_name = "Windows QQ".to_owned();
    }
    if !(1024..=65535).contains(&settings.proxy_port) {
        return Err("本地代理端口必须在 1024 到 65535 之间".to_owned());
    }
    Ok(())
}

fn token_entry() -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(keyring_error)
}

fn keyring_error(error: KeyringError) -> String {
    format!("访问 Windows 凭据管理器失败: {error}")
}

fn write_settings_atomic(path: &PathBuf, settings: &AppSettings) -> Result<(), String> {
    let content =
        serde_json::to_vec_pretty(settings).map_err(|error| format!("序列化设置失败: {error}"))?;
    fs::write(path, content).map_err(|error| format!("保存设置失败: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_server_url() {
        assert_eq!(
            normalize_server_url(" https://farm.example.com/ ").unwrap(),
            "https://farm.example.com"
        );
    }

    #[test]
    fn rejects_server_url_with_query() {
        assert!(normalize_server_url("https://farm.example.com/?token=x").is_err());
    }
}
