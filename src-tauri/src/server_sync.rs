use crate::settings::normalize_server_url;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio::time::sleep;

#[derive(Debug, Serialize)]
struct AccountPayload<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a str>,
    name: &'a str,
    code: &'a str,
    platform: &'static str,
    #[serde(rename = "loginType")]
    login_type: &'static str,
    uin: &'a str,
    qq: &'a str,
    nick: &'a str,
    avatar: &'a str,
}

#[derive(Debug)]
struct ExistingAccount {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct ApiEnvelope {
    ok: bool,
    error: Option<String>,
    data: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct ConnectionInfo {
    pub username: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountProfile {
    pub account_id: String,
    pub account_name: String,
    pub nickname: String,
    pub qq_number: String,
    pub gid: String,
    pub open_id: String,
    pub avatar_url: String,
    pub running: bool,
}

pub struct SyncAccountInput<'a> {
    pub account_name: &'a str,
    pub qq_number: &'a str,
    pub nickname: &'a str,
    pub avatar_url: &'a str,
    pub code: &'a str,
}

#[derive(Debug, Serialize)]
struct FriendOpenIdsPayload<'a> {
    #[serde(rename = "openIds")]
    open_ids: &'a [String],
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FriendSyncResult {
    pub received_count: usize,
    pub friend_count: usize,
    #[serde(default)]
    pub cache_updated: bool,
    #[serde(default)]
    pub cached_count: usize,
    #[serde(default)]
    pub known_friend_gid_count: usize,
}

pub struct ServerClient {
    client: Client,
}

impl ServerClient {
    pub fn new() -> Result<Self, String> {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .no_proxy()
            .build()
            .map_err(|error| format!("创建服务器连接失败: {error}"))?;
        Ok(Self { client })
    }

    pub async fn test_connection(
        &self,
        server_url: &str,
        token: &str,
    ) -> Result<ConnectionInfo, String> {
        self.get_connection_info(server_url, token).await
    }

    async fn get_connection_info(
        &self,
        server_url: &str,
        token: &str,
    ) -> Result<ConnectionInfo, String> {
        let endpoint = endpoint(server_url, "/api/user/me")?;
        let response = self
            .client
            .get(endpoint)
            .header("x-admin-token", token)
            .send()
            .await
            .map_err(|error| format!("连接服务器失败: {error}"))?;
        let envelope = parse_response(response).await?;
        let data = envelope
            .data
            .ok_or_else(|| "服务器没有返回用户信息".to_owned())?;
        let username = first_value(&data, &["username"]);
        if username.is_empty() {
            return Err("服务器没有返回当前用户名，已停止同步以避免覆盖其他用户账号".to_owned());
        }
        Ok(ConnectionInfo {
            username,
            role: first_value(&data, &["role"]),
        })
    }

    pub async fn sync_code(
        &self,
        server_url: &str,
        token: &str,
        input: SyncAccountInput<'_>,
    ) -> Result<AccountProfile, String> {
        let qq_number = validated_qq_number(input.qq_number)?;
        let connection = self.get_connection_info(server_url, token).await?;
        let accounts = self.get_accounts_data(server_url, token).await?;
        let existing = existing_qq_account(&accounts, qq_number, &connection);
        let endpoint = endpoint(server_url, "/api/accounts")?;
        let fallback_avatar = qq_avatar_url(qq_number);
        let avatar = if input.avatar_url.trim().is_empty() {
            fallback_avatar.as_str()
        } else {
            input.avatar_url.trim()
        };
        let account_name = existing
            .as_ref()
            .map(|account| account.name.trim())
            .filter(|name| !name.is_empty())
            .unwrap_or(input.account_name);
        let payload = AccountPayload {
            id: existing.as_ref().map(|account| account.id.as_str()),
            name: account_name,
            code: input.code,
            platform: "qq",
            login_type: "manual",
            uin: qq_number,
            qq: qq_number,
            nick: input.nickname,
            avatar,
        };
        let response = self
            .client
            .post(endpoint)
            .header("x-admin-token", token)
            .json(&payload)
            .send()
            .await
            .map_err(|error| format!("同步 Code 失败: {error}"))?;
        let envelope = parse_response(response).await?;
        let data = envelope
            .data
            .ok_or_else(|| "服务器没有返回账号信息".to_owned())?;
        let mut profile = if let Some(existing) = existing {
            account_profile_by_id(&data, &existing.id)
                .ok_or_else(|| format!("服务器更新账号 {} 后没有返回对应账号信息", existing.id))?
        } else {
            newest_account_profile(&data)
                .ok_or_else(|| "服务器没有返回可识别的账号 ID".to_owned())?
        };

        self.start_account(server_url, token, &profile.account_id)
            .await?;

        for _ in 0..30 {
            if profile.running && profile.has_game_identity() {
                break;
            }
            sleep(Duration::from_millis(500)).await;
            if let Some(updated) = self
                .get_account_profile(server_url, token, &profile.account_id)
                .await?
            {
                profile = updated;
            }
        }
        Ok(profile)
    }

    pub async fn sync_friend_open_ids(
        &self,
        server_url: &str,
        token: &str,
        account_id: &str,
        open_ids: &[String],
    ) -> Result<FriendSyncResult, String> {
        validate_friend_open_ids(open_ids)?;
        let endpoint = endpoint(server_url, "/api/friends/sync-open-ids")?;
        let response = self
            .client
            .post(endpoint)
            .header("x-admin-token", token)
            .header("x-account-id", account_id)
            .json(&FriendOpenIdsPayload { open_ids })
            .send()
            .await
            .map_err(|error| format!("同步好友 OpenID 失败: {error}"))?;
        let envelope = parse_response(response).await?;
        let data = envelope
            .data
            .ok_or_else(|| "服务器没有返回好友同步结果".to_owned())?;
        serde_json::from_value(data).map_err(|_| "服务器返回的好友同步结果无效".to_owned())
    }

    async fn start_account(
        &self,
        server_url: &str,
        token: &str,
        account_id: &str,
    ) -> Result<(), String> {
        let path = format!("/api/accounts/{account_id}/start");
        let endpoint = endpoint(server_url, &path)
            .map_err(|error| format!("Code 已同步，但自动启动远程账号失败: {error}"))?;
        let response = self
            .client
            .post(endpoint)
            .header("x-admin-token", token)
            .send()
            .await
            .map_err(|error| format!("Code 已同步，但自动启动远程账号失败: {error}"))?;
        parse_response(response)
            .await
            .map(|_| ())
            .map_err(|error| format!("Code 已同步，但自动启动远程账号失败: {error}"))
    }

    async fn get_account_profile(
        &self,
        server_url: &str,
        token: &str,
        account_id: &str,
    ) -> Result<Option<AccountProfile>, String> {
        let data = self.get_accounts_data(server_url, token).await?;
        Ok(account_profile_by_id(&data, account_id))
    }

    async fn get_accounts_data(&self, server_url: &str, token: &str) -> Result<Value, String> {
        let endpoint = endpoint(server_url, "/api/accounts")?;
        let response = self
            .client
            .get(endpoint)
            .header("x-admin-token", token)
            .send()
            .await
            .map_err(|error| format!("读取远程账号信息失败: {error}"))?;
        let envelope = parse_response(response).await?;
        envelope
            .data
            .ok_or_else(|| "服务器没有返回账号列表".to_owned())
    }
}

fn validated_qq_number(value: &str) -> Result<&str, String> {
    let value = value.trim();
    if (5..=12).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit()) {
        Ok(value)
    } else {
        Err("拒绝同步：当前 QQ 号未通过本地身份确认，服务器不会创建账号".to_owned())
    }
}

fn validate_friend_open_ids(open_ids: &[String]) -> Result<(), String> {
    if open_ids.is_empty() || open_ids.len() > 200 {
        return Err("好友 OpenID 数量必须在 1 到 200 之间".to_owned());
    }
    if open_ids.iter().any(|open_id| {
        let open_id = open_id.trim();
        open_id.is_empty() || open_id.len() > 128 || open_id.chars().any(char::is_control)
    }) {
        return Err("好友 OpenID 格式无效".to_owned());
    }
    Ok(())
}

impl AccountProfile {
    pub(crate) fn has_game_identity(&self) -> bool {
        !self.nickname.is_empty() && (!self.gid.is_empty() || !self.open_id.is_empty())
    }
}

fn newest_account_profile(data: &Value) -> Option<AccountProfile> {
    data.get("accounts")
        .and_then(Value::as_array)?
        .last()
        .and_then(account_profile_from_value)
}

fn existing_qq_account(
    data: &Value,
    qq_number: &str,
    connection: &ConnectionInfo,
) -> Option<ExistingAccount> {
    data.get("accounts")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(|account| {
            let platform = first_value(account, &["platform"]);
            if !platform.is_empty() && !platform.eq_ignore_ascii_case("qq") {
                return None;
            }
            if first_value(account, &["uin", "qq"]) != qq_number {
                return None;
            }

            let username = first_value(account, &["username"]);
            let owned_by_current_user = username == connection.username;
            let legacy_admin_account =
                username.is_empty() && connection.role.eq_ignore_ascii_case("admin");
            if !owned_by_current_user && !legacy_admin_account {
                return None;
            }

            let id = value_string(account.get("id"));
            if id.is_empty() {
                return None;
            }
            Some(ExistingAccount {
                id,
                name: first_value(account, &["name"]),
            })
        })
        // qq-farm-bot 的数字 ID 按创建顺序递增；已有重复项时优先复用最早账号，
        // 以最大概率保留用户最初绑定在该 ID 下的策略与配置。
        .min_by_key(|account| account.id.parse::<u64>().unwrap_or(u64::MAX))
}

fn account_profile_by_id(data: &Value, account_id: &str) -> Option<AccountProfile> {
    data.get("accounts")
        .and_then(Value::as_array)?
        .iter()
        .find(|account| value_string(account.get("id")) == account_id)
        .and_then(account_profile_from_value)
}

fn account_profile_from_value(account: &Value) -> Option<AccountProfile> {
    let account_id = value_string(account.get("id"));
    if account_id.is_empty() {
        return None;
    }
    let qq_number = first_value(account, &["uin", "qq"]);
    let avatar = first_value(account, &["avatar", "avatarUrl", "avatar_url"]);
    Some(AccountProfile {
        account_id,
        account_name: first_value(account, &["name"]),
        nickname: first_value(account, &["nick", "nickname"]),
        qq_number: qq_number.clone(),
        gid: first_value(account, &["gid"]),
        open_id: first_value(account, &["openId", "open_id"]),
        avatar_url: if avatar.is_empty() {
            qq_avatar_url(&qq_number)
        } else {
            avatar
        },
        running: account
            .get("running")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn first_value(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| {
            let value = value_string(value.get(*key));
            (!value.is_empty()).then_some(value)
        })
        .unwrap_or_default()
}

fn value_string(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.trim().to_owned(),
        Some(Value::Number(value)) => value.to_string(),
        _ => String::new(),
    }
}

fn qq_avatar_url(qq_number: &str) -> String {
    let qq_number = qq_number.trim();
    if qq_number.is_empty() {
        String::new()
    } else {
        format!("https://q1.qlogo.cn/g?b=qq&nk={qq_number}&s=100")
    }
}

async fn parse_response(response: reqwest::Response) -> Result<ApiEnvelope, String> {
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("读取服务器响应失败: {error}"))?;
    let envelope: ApiEnvelope = serde_json::from_str(&text)
        .map_err(|_| format!("服务器返回了非 JSON 响应 (HTTP {})", status.as_u16()))?;
    if status == StatusCode::UNAUTHORIZED {
        return Err("Token 已失效，请从 qq-farm-bot 侧边栏重新复制".to_owned());
    }
    if !status.is_success() || !envelope.ok {
        return Err(envelope
            .error
            .unwrap_or_else(|| format!("服务器请求失败 (HTTP {})", status.as_u16())));
    }
    Ok(envelope)
}

fn endpoint(server_url: &str, path: &str) -> Result<String, String> {
    let base = normalize_server_url(server_url)?;
    if base.is_empty() {
        return Err("请先填写服务器地址".to_owned());
    }
    Ok(format!("{base}{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::oneshot,
    };

    #[test]
    fn builds_api_endpoint() {
        assert_eq!(
            endpoint("https://farm.example.com/", "/api/accounts").unwrap(),
            "https://farm.example.com/api/accounts"
        );
    }

    #[test]
    fn admin_can_reuse_a_legacy_unowned_qq_account() {
        let data: Value = serde_json::from_str(
            r#"{"accounts":[{"id":"3","name":"旧版账号","username":"","platform":"qq","uin":"12345678"}]}"#,
        )
        .unwrap();
        let connection = ConnectionInfo {
            username: "admin".to_owned(),
            role: "admin".to_owned(),
        };

        let account = existing_qq_account(&data, "12345678", &connection).unwrap();
        assert_eq!(account.id, "3");
    }

    #[tokio::test]
    async fn sync_creates_account_when_qq_is_not_present() {
        let (server_url, request_receiver) = serve_http_responses(vec![
            r#"{"ok":true,"data":{"username":"admin","role":"admin"}}"#,
            r#"{"ok":true,"data":{"accounts":[],"nextId":7}}"#,
            r#"{"ok":true,"data":{"accounts":[{"id":"7","name":"Windows QQ","username":"admin","nick":"测试农夫","uin":"12345678","gid":1027000001,"openId":"openid-test","avatar":"https://example.com/avatar.png","running":true}]}}"#,
            r#"{"ok":true}"#,
        ])
        .await;

        let profile = ServerClient::new()
            .unwrap()
            .sync_code(
                &server_url,
                "synthetic-token",
                SyncAccountInput {
                    account_name: "Windows QQ",
                    qq_number: "12345678",
                    nickname: "本机昵称",
                    avatar_url: "https://example.com/local-avatar.png",
                    code: "testcode0123456789abcdef0123456789",
                },
            )
            .await
            .unwrap();
        assert_eq!(profile.nickname, "测试农夫");
        assert_eq!(profile.gid, "1027000001");
        assert_eq!(profile.qq_number, "12345678");
        let requests = request_receiver.await.unwrap();
        assert_eq!(requests.len(), 4);
        assert!(requests[0].starts_with("GET /api/user/me HTTP/1.1"));
        assert!(requests[1].starts_with("GET /api/accounts HTTP/1.1"));
        let request = &requests[2];
        assert!(request.starts_with("POST /api/accounts HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("x-admin-token: synthetic-token")
        );
        assert!(request.contains("\"platform\":\"qq\""));
        assert!(request.contains("\"loginType\":\"manual\""));
        assert!(request.contains("\"uin\":\"12345678\""));
        assert!(request.contains("\"nick\":\"本机昵称\""));
        assert!(request.contains("example.com/local-avatar.png"));
        assert!(!request.contains("\"id\":"));
        assert!(requests[3].starts_with("POST /api/accounts/7/start HTTP/1.1"));
        assert!(
            requests[3]
                .to_ascii_lowercase()
                .contains("x-admin-token: synthetic-token")
        );
    }

    #[tokio::test]
    async fn sync_updates_oldest_matching_qq_account_and_preserves_its_name() {
        let (server_url, request_receiver) = serve_http_responses(vec![
            r#"{"ok":true,"data":{"username":"admin","role":"admin"}}"#,
            r#"{"ok":true,"data":{"accounts":[{"id":"2","name":"主号原配置","username":"admin","platform":"qq","uin":"12345678","running":false},{"id":"7","name":"重复账号","username":"admin","platform":"qq","uin":"12345678","running":true},{"id":"8","name":"其他用户账号","username":"alice","platform":"qq","uin":"12345678","running":true}]}}"#,
            r#"{"ok":true,"data":{"accounts":[{"id":"2","name":"主号原配置","username":"admin","platform":"qq","nick":"测试农夫","uin":"12345678","gid":1027000001,"openId":"openid-test","running":false},{"id":"7","name":"重复账号","username":"admin","platform":"qq","uin":"12345678","running":true}]}}"#,
            r#"{"ok":true}"#,
            r#"{"ok":true,"data":{"accounts":[{"id":"2","name":"主号原配置","username":"admin","platform":"qq","nick":"测试农夫","uin":"12345678","gid":1027000001,"openId":"openid-test","running":true},{"id":"7","name":"重复账号","username":"admin","platform":"qq","uin":"12345678","running":true}]}}"#,
        ])
        .await;

        let profile = ServerClient::new()
            .unwrap()
            .sync_code(
                &server_url,
                "synthetic-token",
                SyncAccountInput {
                    account_name: "Windows QQ",
                    qq_number: "12345678",
                    nickname: "本机昵称",
                    avatar_url: "",
                    code: "newcode0123456789abcdef0123456789",
                },
            )
            .await
            .unwrap();

        assert_eq!(profile.account_id, "2");
        assert_eq!(profile.account_name, "主号原配置");
        assert!(profile.running);
        let requests = request_receiver.await.unwrap();
        let request = &requests[2];
        assert!(request.contains("\"id\":\"2\""));
        assert!(request.contains("\"name\":\"主号原配置\""));
        assert!(request.contains("\"code\":\"newcode0123456789abcdef0123456789\""));
        assert!(!request.contains("\"id\":\"7\""));
        assert!(requests[3].starts_with("POST /api/accounts/2/start HTTP/1.1"));
        assert!(requests[4].starts_with("GET /api/accounts HTTP/1.1"));
    }

    #[tokio::test]
    async fn sync_does_not_update_same_qq_owned_by_another_user() {
        let (server_url, request_receiver) = serve_http_responses(vec![
            r#"{"ok":true,"data":{"username":"admin","role":"admin"}}"#,
            r#"{"ok":true,"data":{"accounts":[{"id":"5","name":"其他用户账号","username":"alice","platform":"qq","uin":"12345678","running":true}]}}"#,
            r#"{"ok":true,"data":{"accounts":[{"id":"5","name":"其他用户账号","username":"alice","platform":"qq","uin":"12345678","running":true},{"id":"6","name":"Windows QQ","username":"admin","platform":"qq","nick":"测试农夫","uin":"12345678","gid":1027000001,"openId":"openid-new","running":true}]}}"#,
            r#"{"ok":true}"#,
        ])
        .await;

        let profile = ServerClient::new()
            .unwrap()
            .sync_code(
                &server_url,
                "synthetic-token",
                SyncAccountInput {
                    account_name: "Windows QQ",
                    qq_number: "12345678",
                    nickname: "本机昵称",
                    avatar_url: "",
                    code: "newcode0123456789abcdef0123456789",
                },
            )
            .await
            .unwrap();

        assert_eq!(profile.account_id, "6");
        let requests = request_receiver.await.unwrap();
        assert!(!requests[2].contains("\"id\":"));
        assert!(requests[3].starts_with("POST /api/accounts/6/start HTTP/1.1"));
    }

    #[tokio::test]
    async fn sync_reports_when_account_was_saved_but_auto_start_failed() {
        let (server_url, _request_receiver) = serve_http_responses(vec![
            r#"{"ok":true,"data":{"username":"admin","role":"admin"}}"#,
            r#"{"ok":true,"data":{"accounts":[],"nextId":7}}"#,
            r#"{"ok":true,"data":{"accounts":[{"id":"7","name":"Windows QQ","username":"admin","uin":"12345678","running":false}]}}"#,
            r#"{"ok":false,"error":"Account not found"}"#,
        ])
        .await;

        let error = ServerClient::new()
            .unwrap()
            .sync_code(
                &server_url,
                "synthetic-token",
                SyncAccountInput {
                    account_name: "Windows QQ",
                    qq_number: "12345678",
                    nickname: "本机昵称",
                    avatar_url: "",
                    code: "newcode0123456789abcdef0123456789",
                },
            )
            .await
            .unwrap_err();

        assert_eq!(
            error,
            "Code 已同步，但自动启动远程账号失败: Account not found"
        );
    }

    #[tokio::test]
    async fn rejects_sync_without_a_confirmed_qq_number() {
        let result = ServerClient::new()
            .unwrap()
            .sync_code(
                "http://127.0.0.1:9",
                "synthetic-token",
                SyncAccountInput {
                    account_name: "Windows QQ",
                    qq_number: "",
                    nickname: "未确认账号",
                    avatar_url: "",
                    code: "testcode0123456789abcdef0123456789",
                },
            )
            .await;
        assert!(result.unwrap_err().contains("服务器不会创建账号"));
    }

    #[tokio::test]
    async fn friend_sync_posts_only_open_ids_to_the_selected_account() {
        let (server_url, request_receiver) = serve_http_responses(vec![
            r#"{"ok":true,"data":{"receivedCount":2,"friendCount":20,"cacheUpdated":true,"cachedCount":20,"knownFriendGidCount":20}}"#,
        ])
        .await;
        let open_ids = vec!["open-a".to_owned(), "open-b".to_owned()];

        let result = ServerClient::new()
            .unwrap()
            .sync_friend_open_ids(&server_url, "synthetic-token", "7", &open_ids)
            .await
            .unwrap();

        assert_eq!(
            result,
            FriendSyncResult {
                received_count: 2,
                friend_count: 20,
                cache_updated: true,
                cached_count: 20,
                known_friend_gid_count: 20,
            }
        );
        let requests = request_receiver.await.unwrap();
        let request = &requests[0];
        assert!(request.starts_with("POST /api/friends/sync-open-ids HTTP/1.1"));
        assert!(request.to_ascii_lowercase().contains("x-account-id: 7"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("x-admin-token: synthetic-token")
        );
        assert!(request.contains(r#"{"openIds":["open-a","open-b"]}"#));
        assert!(!request.contains("nickname"));
        assert!(!request.contains("gid"));
    }

    #[tokio::test]
    async fn friend_sync_rejects_invalid_open_ids_before_connecting() {
        let result = ServerClient::new()
            .unwrap()
            .sync_friend_open_ids("http://127.0.0.1:9", "token", "7", &[])
            .await;

        assert!(result.unwrap_err().contains("OpenID"));
    }

    async fn read_http_request(socket: &mut tokio::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 1024];
        let mut expected_length = None;
        loop {
            let read = socket.read(&mut buffer).await.unwrap();
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
            if expected_length.is_none() {
                expected_length = total_request_length(&bytes);
            }
            if expected_length.is_some_and(|length| bytes.len() >= length) {
                break;
            }
        }
        String::from_utf8(bytes).unwrap()
    }

    async fn serve_http_responses(
        bodies: Vec<&'static str>,
    ) -> (String, oneshot::Receiver<Vec<String>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (request_sender, request_receiver) = oneshot::channel();
        tokio::spawn(async move {
            let mut requests = Vec::with_capacity(bodies.len());
            for body in bodies {
                let (mut socket, _) = listener.accept().await.unwrap();
                requests.push(read_http_request(&mut socket).await);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
            request_sender.send(requests).unwrap();
        });
        (format!("http://127.0.0.1:{port}"), request_receiver)
    }

    fn total_request_length(bytes: &[u8]) -> Option<usize> {
        let header_end = bytes.windows(4).position(|window| window == b"\r\n\r\n")? + 4;
        let headers = std::str::from_utf8(&bytes[..header_end]).ok()?;
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())?
            })
            .unwrap_or(0);
        Some(header_end + content_length)
    }
}
