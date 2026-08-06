use serde::Serialize;
use serde_json::Value;
use std::{fs, path::PathBuf};
use url::Url;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalQqIdentity {
    pub qq_number: String,
    pub nickname: String,
    pub avatar_url: String,
    pub source: &'static str,
}

pub fn detect() -> Result<LocalQqIdentity, String> {
    let app_data = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "无法定位 Windows QQ 配置目录".to_owned())?;
    let path = app_data.join("QQ").join("auth").join("login.enc");
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("读取 Windows QQ 登录信息失败: {error}"))?;
    parse_identity(content.trim_start_matches('\u{feff}'))
}

fn parse_identity(content: &str) -> Result<LocalQqIdentity, String> {
    let data: Value =
        serde_json::from_str(content).map_err(|_| "Windows QQ 登录信息格式无法识别".to_owned())?;
    let current = data
        .get("account")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| is_qq_number(value))
        .ok_or_else(|| "Windows QQ 当前账号为空，请先登录 QQ".to_owned())?;
    let entry = data
        .get("loginList")
        .and_then(Value::as_array)
        .and_then(|entries| {
            entries.iter().find(|entry| {
                entry.get("uin").and_then(Value::as_str).map(str::trim) == Some(current)
            })
        });
    let nickname = entry
        .and_then(|entry| entry.get("nickName"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned();
    let face_url = entry
        .and_then(|entry| entry.get("faceUrl"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let avatar_url = if is_safe_avatar_url(face_url) {
        face_url.to_owned()
    } else {
        qq_avatar_url(current)
    };
    Ok(LocalQqIdentity {
        qq_number: current.to_owned(),
        nickname,
        avatar_url,
        source: "qqnt_login_config",
    })
}

fn is_qq_number(value: &str) -> bool {
    (5..=12).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_safe_avatar_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| url.scheme() == "https" && url.host_str().is_some())
}

fn qq_avatar_url(qq_number: &str) -> String {
    format!("https://q1.qlogo.cn/g?b=qq&nk={qq_number}&s=100")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_current_qq_identity() {
        let identity = parse_identity(
            r#"{
              "account":"12345678",
              "loginList":[
                {"uin":"87654321","nickName":"旧账号","faceUrl":""},
                {"uin":"12345678","nickName":"小农夫","faceUrl":"https://example.com/avatar.png"}
              ]
            }"#,
        )
        .unwrap();
        assert_eq!(identity.qq_number, "12345678");
        assert_eq!(identity.nickname, "小农夫");
        assert_eq!(identity.avatar_url, "https://example.com/avatar.png");
    }

    #[test]
    fn rejects_missing_current_account() {
        assert!(parse_identity(r#"{"account":"","loginList":[]}"#).is_err());
    }

    #[test]
    fn keeps_new_account_before_login_list_refreshes() {
        let identity = parse_identity(r#"{"account":"12345678","loginList":[]}"#).unwrap();
        assert_eq!(identity.qq_number, "12345678");
        assert!(identity.nickname.is_empty());
        assert!(identity.avatar_url.contains("nk=12345678"));
    }
}
