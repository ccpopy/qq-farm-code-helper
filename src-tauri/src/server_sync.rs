use crate::settings::normalize_server_url;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio::time::sleep;

#[derive(Debug, Serialize)]
struct AccountPayload<'a> {
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
        Ok(ConnectionInfo {
            username: data
                .get("username")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            role: data
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
        })
    }

    pub async fn sync_code(
        &self,
        server_url: &str,
        token: &str,
        input: SyncAccountInput<'_>,
    ) -> Result<AccountProfile, String> {
        let qq_number = validated_qq_number(input.qq_number)?;
        let endpoint = endpoint(server_url, "/api/accounts")?;
        let fallback_avatar = qq_avatar_url(qq_number);
        let avatar = if input.avatar_url.trim().is_empty() {
            fallback_avatar.as_str()
        } else {
            input.avatar_url.trim()
        };
        let payload = AccountPayload {
            name: input.account_name,
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
            .ok_or_else(|| "服务器没有返回新建账号信息".to_owned())?;
        let mut profile = newest_account_profile(&data)
            .ok_or_else(|| "服务器没有返回可识别的账号 ID".to_owned())?;

        for _ in 0..30 {
            if profile.has_game_identity() {
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

    async fn get_account_profile(
        &self,
        server_url: &str,
        token: &str,
        account_id: &str,
    ) -> Result<Option<AccountProfile>, String> {
        let endpoint = endpoint(server_url, "/api/accounts")?;
        let response = self
            .client
            .get(endpoint)
            .header("x-admin-token", token)
            .send()
            .await
            .map_err(|error| format!("读取远程账号信息失败: {error}"))?;
        let envelope = parse_response(response).await?;
        Ok(envelope
            .data
            .as_ref()
            .and_then(|data| account_profile_by_id(data, account_id)))
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

    #[tokio::test]
    async fn sync_sends_expected_auth_and_account_payload() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (request_sender, request_receiver) = oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut socket).await;
            request_sender.send(request).unwrap();
            let body = r#"{"ok":true,"data":{"accounts":[{"id":"7","name":"Windows QQ","nick":"测试农夫","uin":"12345678","gid":1027000001,"openId":"openid-test","avatar":"https://example.com/avatar.png","running":true}]}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });

        let profile = ServerClient::new()
            .unwrap()
            .sync_code(
                &format!("http://127.0.0.1:{port}"),
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
        let request = request_receiver.await.unwrap();
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

    fn total_request_length(bytes: &[u8]) -> Option<usize> {
        let header_end = bytes.windows(4).position(|window| window == b"\r\n\r\n")? + 4;
        let headers = std::str::from_utf8(&bytes[..header_end]).ok()?;
        let content_length = headers.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })?;
        Some(header_end + content_length)
    }
}
