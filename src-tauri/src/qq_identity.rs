use serde::Serialize;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use url::Url;

const STABLE_DETECTION_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalQqIdentity {
    pub qq_number: String,
    pub nickname: String,
    pub avatar_url: String,
    pub source: &'static str,
    pub verification_detail: &'static str,
}

#[derive(Debug)]
struct LoginEntry {
    qq_number: String,
    nickname: String,
    avatar_url: String,
}

pub fn detect() -> Result<LocalQqIdentity, String> {
    let app_data = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "无法定位 Windows QQ 配置目录".to_owned())?;
    detect_from_qq_root(&app_data.join("QQ"))
}

pub async fn detect_async() -> Result<LocalQqIdentity, String> {
    tokio::task::spawn_blocking(detect)
        .await
        .map_err(|error| format!("QQ 身份检测任务异常结束: {error}"))?
}

pub async fn detect_stable_async() -> Result<LocalQqIdentity, String> {
    tokio::time::timeout(STABLE_DETECTION_TIMEOUT, detect_stable_async_inner())
        .await
        .map_err(|_| "读取 QQ 主界面超过 2 秒，本次不会自动绑定 QQ 号".to_owned())?
}

async fn detect_stable_async_inner() -> Result<LocalQqIdentity, String> {
    let first = detect_async().await?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    let second = detect_async().await?;
    confirm_stable_identity(first, second)
}

fn confirm_stable_identity(
    first: LocalQqIdentity,
    mut second: LocalQqIdentity,
) -> Result<LocalQqIdentity, String> {
    if first.qq_number != second.qq_number || first.nickname != second.nickname {
        return Err(format!(
            "QQ 正在切换账号：连续两次检测分别为 {} / {}，本次不会绑定 QQ 号",
            first.qq_number, second.qq_number
        ));
    }
    second.source = "qq_ui_stable_unique_nickname";
    second.verification_detail = "QQ 前台昵称 + login.enc 唯一匹配 + 连续两次一致";
    Ok(second)
}

fn detect_from_qq_root(qq_root: &Path) -> Result<LocalQqIdentity, String> {
    let visible_nickname = crate::qq_window::current_nickname()?;
    let path = qq_root.join("auth").join("login.enc");
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("读取 Windows QQ 登录信息失败: {error}"))?;
    let entries = parse_login_entries(content.trim_start_matches('\u{feff}'))?;
    resolve_identity(entries, &visible_nickname)
}

fn parse_login_entries(content: &str) -> Result<Vec<LoginEntry>, String> {
    let data: Value =
        serde_json::from_str(content).map_err(|_| "Windows QQ 登录信息格式无法识别".to_owned())?;
    let entries = data
        .get("loginList")
        .and_then(Value::as_array)
        .ok_or_else(|| "Windows QQ 登录列表为空，请先登录 QQ".to_owned())?;
    let entries = entries
        .iter()
        .filter_map(|entry| {
            let qq_number = value_text(entry.get("uin"));
            if !is_qq_number(&qq_number) {
                return None;
            }
            let nickname = entry
                .get("nickName")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned();
            let face_url = entry
                .get("faceUrl")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            Some(LoginEntry {
                avatar_url: if is_safe_avatar_url(face_url) {
                    face_url.to_owned()
                } else {
                    qq_avatar_url(&qq_number)
                },
                qq_number,
                nickname,
            })
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return Err("Windows QQ 登录列表中没有可识别的账号".to_owned());
    }
    Ok(entries)
}

fn resolve_identity(
    entries: Vec<LoginEntry>,
    visible_nickname: &str,
) -> Result<LocalQqIdentity, String> {
    let visible_nickname = visible_nickname.trim();
    if visible_nickname.is_empty() {
        return Err("QQ 主界面昵称为空，本次不会自动绑定 QQ 号".to_owned());
    }
    let mut matches = entries
        .iter()
        .filter(|entry| entry.nickname == visible_nickname);
    let matched_entry = matches.next().ok_or_else(|| {
        format!(
            "QQ 主界面显示“{visible_nickname}”，但登录列表中没有对应账号，本次不会自动绑定 QQ 号"
        )
    })?;
    if matches.next().is_some() {
        return Err(format!(
            "QQ 主界面显示“{visible_nickname}”，但该昵称对应多个账号，无法唯一确认 QQ 号"
        ));
    }
    Ok(LocalQqIdentity {
        qq_number: matched_entry.qq_number.clone(),
        nickname: matched_entry.nickname.clone(),
        avatar_url: matched_entry.avatar_url.clone(),
        source: "qq_ui_unique_nickname",
        verification_detail: "QQ 前台昵称 + login.enc 唯一匹配",
    })
}

fn value_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.trim().to_owned(),
        Some(Value::Number(value)) => value.to_string(),
        _ => String::new(),
    }
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

    fn entries(content: &str) -> Vec<LoginEntry> {
        parse_login_entries(content).unwrap()
    }

    #[test]
    fn matches_the_unique_login_entry_for_the_visible_qq_nickname() {
        let identity = resolve_identity(
            entries(
                r#"{
                  "account":"3170105001",
                  "loginList":[
                    {"uin":"1343475483","nickName":"新账号","faceUrl":"https://example.com/new.png","isUserLogin":true},
                    {"uin":"3170105001","nickName":"旧账号","faceUrl":"","isUserLogin":false}
                  ]
                }"#,
            ),
            "旧账号",
        )
        .unwrap();
        assert_eq!(identity.qq_number, "3170105001");
        assert_eq!(identity.nickname, "旧账号");
        assert_eq!(identity.source, "qq_ui_unique_nickname");
    }

    #[test]
    fn rejects_a_visible_nickname_missing_from_the_login_list() {
        let result = resolve_identity(
            entries(
                r#"{
                  "loginList":[
                    {"uin":"1343475483","nickName":"新账号","isUserLogin":true},
                    {"uin":"3170105001","nickName":"旧账号","isUserLogin":false}
                  ]
                }"#,
            ),
            "另一个账号",
        );
        assert!(result.unwrap_err().contains("没有对应账号"));
    }

    #[test]
    fn rejects_duplicate_nicknames_in_the_login_list() {
        let result = resolve_identity(
            entries(
                r#"{"loginList":[
                    {"uin":"12345678","nickName":"同名账号"},
                    {"uin":"87654321","nickName":"同名账号"}
                ]}"#,
            ),
            "同名账号",
        );
        assert!(result.unwrap_err().contains("对应多个账号"));
    }

    #[test]
    fn supports_numeric_uin_and_uses_qlogo_fallback() {
        let identity = resolve_identity(
            entries(
                r#"{"loginList":[{"uin":12345678,"nickName":"账号","faceUrl":"","isUserLogin":true}]}"#,
            ),
            "账号",
        )
        .unwrap();
        assert!(identity.avatar_url.contains("nk=12345678"));
    }

    #[test]
    fn rejects_identity_that_changes_between_two_samples() {
        let first = LocalQqIdentity {
            qq_number: "12345678".to_owned(),
            nickname: "账号一".to_owned(),
            avatar_url: String::new(),
            source: "test",
            verification_detail: "test",
        };
        let second = LocalQqIdentity {
            qq_number: "87654321".to_owned(),
            nickname: "账号二".to_owned(),
            avatar_url: String::new(),
            source: "test",
            verification_detail: "test",
        };
        assert!(confirm_stable_identity(first, second).is_err());
    }

    #[tokio::test]
    #[ignore = "requires a running Windows QQ main window and local login.enc"]
    async fn detects_the_live_windows_qq_identity() {
        let identity = detect_stable_async().await.unwrap();
        eprintln!(
            "live QQ identity: {} / {} ({})",
            identity.nickname, identity.qq_number, identity.verification_detail
        );
        assert!(!identity.qq_number.is_empty());
    }
}
