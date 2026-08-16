use crate::qq_login_history::{ObservedQqAccount, QqLoginHistory};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
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

pub fn detect_all(history: &QqLoginHistory) -> Result<Vec<LocalQqIdentity>, String> {
    let app_data = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "无法定位 Windows QQ 配置目录".to_owned())?;
    detect_all_from_qq_root(&app_data.join("QQ"), history)
}

pub async fn detect_stable_all_async(
    history: Arc<QqLoginHistory>,
) -> Result<Vec<LocalQqIdentity>, String> {
    tokio::time::timeout(
        STABLE_DETECTION_TIMEOUT,
        detect_stable_all_async_inner(history),
    )
    .await
    .map_err(|_| "读取 QQ 主界面超过 2 秒，本次不会自动绑定 QQ 号".to_owned())?
}

pub async fn detect_selected_stable_async(
    history: Arc<QqLoginHistory>,
    qq_number: &str,
) -> Result<LocalQqIdentity, String> {
    let identities = detect_stable_all_async(history).await?;
    identities
        .into_iter()
        .find(|identity| identity.qq_number == qq_number)
        .ok_or_else(|| format!("当前 QQ 主窗口中未找到所选账号 {qq_number}，请重新选择"))
}

async fn detect_stable_all_async_inner(
    history: Arc<QqLoginHistory>,
) -> Result<Vec<LocalQqIdentity>, String> {
    let first = detect_all_async(history.clone()).await?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    let second = detect_all_async(history).await?;
    confirm_stable_identities(first, second)
}

async fn detect_all_async(history: Arc<QqLoginHistory>) -> Result<Vec<LocalQqIdentity>, String> {
    tokio::task::spawn_blocking(move || detect_all(&history))
        .await
        .map_err(|error| format!("QQ 身份检测任务异常结束: {error}"))?
}

fn confirm_stable_identities(
    first: Vec<LocalQqIdentity>,
    mut second: Vec<LocalQqIdentity>,
) -> Result<Vec<LocalQqIdentity>, String> {
    let first_keys = first
        .iter()
        .map(|identity| (identity.qq_number.clone(), identity.nickname.clone()))
        .collect::<Vec<_>>();
    let second_keys = second
        .iter()
        .map(|identity| (identity.qq_number.clone(), identity.nickname.clone()))
        .collect::<Vec<_>>();
    if first_keys != second_keys {
        return Err(format!(
            "QQ 登录实例正在变化：连续两次检测到的账号数量为 {} / {}，请稍后重试",
            first.len(),
            second.len()
        ));
    }
    for identity in &mut second {
        identity.source = "qq_ui_stable_visible_accounts";
        identity.verification_detail = "QQ 主界面昵称 + 本机登录记录匹配 + 连续两次一致";
    }
    Ok(second)
}

fn detect_all_from_qq_root(
    qq_root: &Path,
    history: &QqLoginHistory,
) -> Result<Vec<LocalQqIdentity>, String> {
    let visible_nicknames = crate::qq_window::visible_nicknames()?;
    let path = qq_root.join("auth").join("login.enc");
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("读取 Windows QQ 登录信息失败: {error}"))?;
    let entries = parse_login_entries(content.trim_start_matches('\u{feff}'))?;
    let entries = history.remember_and_merge(entries);
    resolve_identities(entries, &visible_nicknames)
}

fn parse_login_entries(content: &str) -> Result<Vec<ObservedQqAccount>, String> {
    let data: Value =
        serde_json::from_str(content).map_err(|_| "Windows QQ 登录信息格式无法识别".to_owned())?;
    // 不同版本的 Windows QQ 写出的 login.enc 结构不同：顶层直接是账号数组，或包在 {"loginList": [...]} 里。
    let entries = match &data {
        Value::Array(entries) => entries,
        _ => data
            .get("loginList")
            .and_then(Value::as_array)
            .ok_or_else(|| "Windows QQ 登录列表为空，请先登录 QQ".to_owned())?,
    };
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
            Some(ObservedQqAccount {
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

fn resolve_identities(
    entries: Vec<ObservedQqAccount>,
    visible_nicknames: &[String],
) -> Result<Vec<LocalQqIdentity>, String> {
    let visible_nicknames = visible_nicknames
        .iter()
        .map(|nickname| nickname.trim())
        .filter(|nickname| !nickname.is_empty())
        .collect::<BTreeSet<_>>();
    if visible_nicknames.is_empty() {
        return Err("QQ 主界面昵称为空，本次不会自动绑定 QQ 号".to_owned());
    }

    let identities = entries
        .into_iter()
        .filter(|entry| visible_nicknames.contains(entry.nickname.as_str()))
        .map(|entry| LocalQqIdentity {
            qq_number: entry.qq_number,
            nickname: entry.nickname,
            avatar_url: entry.avatar_url,
            source: "qq_ui_visible_account",
            verification_detail: "QQ 主界面昵称 + 本机登录记录匹配",
        })
        .collect::<Vec<_>>();
    if identities.is_empty() {
        return Err(format!(
            "QQ 主界面显示的账号（{}）均未在本机登录记录中找到，请用该账号重新登录一次 QQ；本次不会自动绑定 QQ 号",
            visible_nicknames.into_iter().collect::<Vec<_>>().join("、")
        ));
    }
    Ok(identities)
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

    fn entries(content: &str) -> Vec<ObservedQqAccount> {
        parse_login_entries(content).unwrap()
    }

    fn temp_history() -> (PathBuf, QqLoginHistory) {
        let directory = std::env::temp_dir().join(format!(
            "qq-farm-code-helper-identity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let history = QqLoginHistory::new(directory.clone());
        (directory, history)
    }

    #[test]
    fn matches_the_unique_login_entry_for_the_visible_qq_nickname() {
        let identities = resolve_identities(
            entries(
                r#"{
                  "account":"3170105001",
                  "loginList":[
                    {"uin":"1343475483","nickName":"新账号","faceUrl":"https://example.com/new.png","isUserLogin":true},
                    {"uin":"3170105001","nickName":"旧账号","faceUrl":"","isUserLogin":false}
                  ]
                }"#,
            ),
            &["旧账号".to_owned()],
        )
        .unwrap();
        let identity = &identities[0];
        assert_eq!(identity.qq_number, "3170105001");
        assert_eq!(identity.nickname, "旧账号");
        assert_eq!(identity.source, "qq_ui_visible_account");
    }

    #[test]
    fn lists_each_visible_windows_qq_account_for_explicit_selection() {
        let identities = resolve_identities(
            entries(
                r#"{
                  "loginList":[
                    {"uin":"1343475483","nickName":"账号一","faceUrl":""},
                    {"uin":"3170105001","nickName":"账号二","faceUrl":""},
                    {"uin":"87654321","nickName":"未打开账号","faceUrl":""}
                  ]
                }"#,
            ),
            &["账号一".to_owned(), "账号二".to_owned()],
        )
        .unwrap();

        assert_eq!(
            identities
                .iter()
                .map(|identity| identity.qq_number.as_str())
                .collect::<Vec<_>>(),
            vec!["1343475483", "3170105001"]
        );
    }

    #[test]
    fn keeps_duplicate_nicknames_as_separate_selectable_accounts() {
        let identities = resolve_identities(
            entries(
                r#"{"loginList":[
                    {"uin":"12345678","nickName":"同名账号"},
                    {"uin":"87654321","nickName":"同名账号"}
                ]}"#,
            ),
            &["同名账号".to_owned()],
        )
        .unwrap();

        assert_eq!(identities.len(), 2);
        assert_eq!(identities[0].qq_number, "12345678");
        assert_eq!(identities[1].qq_number, "87654321");
    }

    #[test]
    fn rejects_a_visible_nickname_missing_from_the_login_list() {
        let result = resolve_identities(
            entries(
                r#"{
                  "loginList":[
                    {"uin":"1343475483","nickName":"新账号","isUserLogin":true},
                    {"uin":"3170105001","nickName":"旧账号","isUserLogin":false}
                  ]
                }"#,
            ),
            &["另一个账号".to_owned()],
        );
        assert!(result.unwrap_err().contains("均未在本机登录记录中找到"));
    }

    #[test]
    fn offers_the_remembered_account_when_the_login_file_only_keeps_the_latest() {
        let (directory, history) = temp_history();
        history.remember_and_merge(entries(
            r#"[{"uin":"1343475483","nickName":"芜","faceUrl":"","isUserLogin":true}]"#,
        ));

        let merged = history.remember_and_merge(entries(
            r#"[{"uin":"3170105001","nickName":"落","faceUrl":"","isUserLogin":true}]"#,
        ));
        let identities = resolve_identities(merged, &["芜".to_owned(), "落".to_owned()]).unwrap();

        assert_eq!(
            identities
                .iter()
                .map(|identity| identity.qq_number.as_str())
                .collect::<Vec<_>>(),
            vec!["3170105001", "1343475483"]
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn parses_a_login_file_whose_top_level_is_an_entry_array() {
        let identities = resolve_identities(
            entries(
                r#"[{"uin":"1343475483","nickName":"账号","faceUrl":"","facePath":"","isUserLogin":true,"isAutoLogin":true,"isQuickLogin":true,"loginType":1}]"#,
            ),
            &["账号".to_owned()],
        )
        .unwrap();
        let identity = &identities[0];
        assert_eq!(identity.qq_number, "1343475483");
        assert_eq!(identity.nickname, "账号");
    }

    #[test]
    fn supports_numeric_uin_and_uses_qlogo_fallback() {
        let identities = resolve_identities(
            entries(
                r#"{"loginList":[{"uin":12345678,"nickName":"账号","faceUrl":"","isUserLogin":true}]}"#,
            ),
            &["账号".to_owned()],
        )
        .unwrap();
        let identity = &identities[0];
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
        assert!(confirm_stable_identities(vec![first], vec![second]).is_err());
    }

    #[test]
    #[ignore = "requires a local Windows QQ login.enc"]
    fn parses_the_live_login_file() {
        let app_data = std::env::var_os("APPDATA").map(PathBuf::from).unwrap();
        let content =
            fs::read_to_string(app_data.join("QQ").join("auth").join("login.enc")).unwrap();
        let entries = parse_login_entries(content.trim_start_matches('\u{feff}')).unwrap();
        eprintln!("live login entry count: {}", entries.len());
        assert!(!entries.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires a running Windows QQ main window and local login.enc"]
    async fn detects_the_live_windows_qq_identities() {
        let (directory, history) = temp_history();
        let identities = detect_stable_all_async(Arc::new(history)).await.unwrap();
        eprintln!("live QQ account count: {}", identities.len());
        assert!(!identities.is_empty());
        assert!(
            identities
                .iter()
                .all(|identity| !identity.qq_number.is_empty())
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
