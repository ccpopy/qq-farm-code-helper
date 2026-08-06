use crate::settings::normalize_server_url;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Serialize)]
struct AccountPayload<'a> {
    name: &'a str,
    code: &'a str,
    platform: &'static str,
    #[serde(rename = "loginType")]
    login_type: &'static str,
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
        account_name: &str,
        code: &str,
    ) -> Result<(), String> {
        let endpoint = endpoint(server_url, "/api/accounts")?;
        let payload = AccountPayload {
            name: account_name,
            code,
            platform: "qq",
            login_type: "manual",
        };
        let response = self
            .client
            .post(endpoint)
            .header("x-admin-token", token)
            .json(&payload)
            .send()
            .await
            .map_err(|error| format!("同步 Code 失败: {error}"))?;
        parse_response(response).await.map(|_| ())
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
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 21\r\nConnection: close\r\n\r\n{\"ok\":true,\"data\":{}}",
                )
                .await
                .unwrap();
        });

        ServerClient::new()
            .unwrap()
            .sync_code(
                &format!("http://127.0.0.1:{port}"),
                "synthetic-token",
                "Windows QQ",
                "testcode0123456789abcdef0123456789",
            )
            .await
            .unwrap();
        let request = request_receiver.await.unwrap();
        assert!(request.starts_with("POST /api/accounts HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("x-admin-token: synthetic-token")
        );
        assert!(request.contains("\"platform\":\"qq\""));
        assert!(request.contains("\"loginType\":\"manual\""));
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
