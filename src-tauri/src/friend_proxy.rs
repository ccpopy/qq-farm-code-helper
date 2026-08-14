use crate::{certificates::TARGET_HOST, friend_capture::FriendSyncInspector, proxy::read_headers};
use rustls::ClientConfig;
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, copy_bidirectional},
    net::{TcpStream, lookup_host},
    sync::mpsc,
    time::timeout,
};
use tokio_rustls::TlsConnector;

const TARGET_PATH: &str = "/prod/ws";

pub async fn relay_target_websocket<S>(
    mut client: S,
    request_headers: Vec<u8>,
    upstream_tls: Arc<ClientConfig>,
    captured: mpsc::Sender<Vec<String>>,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    validate_target_websocket_request(&request_headers)?;
    let upstream_request = rewrite_target_websocket_request(&request_headers)?;
    drop(request_headers);

    let mut upstream = connect_target_tls(upstream_tls).await?;
    upstream
        .write_all(&upstream_request)
        .await
        .map_err(|error| format!("转发 QQ WebSocket 握手失败: {error}"))?;
    drop(upstream_request);

    let response_headers = read_headers(&mut upstream).await?;
    let status = response_status(&response_headers).unwrap_or_default();
    if has_header(&response_headers, "sec-websocket-extensions") {
        return Err("QQ WebSocket 返回了不支持的压缩扩展".to_owned());
    }
    client
        .write_all(&response_headers)
        .await
        .map_err(|error| format!("转发 QQ WebSocket 握手响应失败: {error}"))?;

    if status != 101 || !is_websocket_upgrade(&response_headers) {
        copy_bidirectional(&mut client, &mut upstream)
            .await
            .map_err(|error| format!("转发 QQ HTTPS 响应失败: {error}"))?;
        return Ok(());
    }

    relay_websocket(client, upstream, captured).await
}

async fn connect_target_tls(
    config: Arc<ClientConfig>,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, String> {
    let addresses = timeout(Duration::from_secs(10), lookup_host((TARGET_HOST, 443)))
        .await
        .map_err(|_| "解析 QQ 农场网关地址超时".to_owned())?
        .map_err(|error| format!("解析 QQ 农场网关地址失败: {error}"))?;
    let mut last_error = None;
    let mut stream = None;
    for address in addresses.filter(|address| {
        let ip = address.ip();
        !ip.is_loopback() && !ip.is_unspecified() && !ip.is_multicast()
    }) {
        match timeout(Duration::from_secs(8), TcpStream::connect(address)).await {
            Ok(Ok(candidate)) => {
                stream = Some(candidate);
                break;
            }
            Ok(Err(error)) => last_error = Some(error.to_string()),
            Err(_) => last_error = Some("连接超时".to_owned()),
        }
    }
    let stream = stream.ok_or_else(|| {
        format!(
            "连接 QQ 农场网关失败: {}",
            last_error.unwrap_or_else(|| "没有可用的公网地址".to_owned())
        )
    })?;
    let server_name = rustls_pki_types::ServerName::try_from(TARGET_HOST.to_owned())
        .map_err(|_| "QQ 农场网关域名无效".to_owned())?;
    timeout(
        Duration::from_secs(15),
        TlsConnector::from(config).connect(server_name, stream),
    )
    .await
    .map_err(|_| "连接 QQ 农场网关 TLS 超时".to_owned())?
    .map_err(|error| format!("验证 QQ 农场网关 TLS 失败: {error}"))
}

async fn relay_websocket<C, U>(
    client: C,
    upstream: U,
    captured: mpsc::Sender<Vec<String>>,
) -> Result<(), String>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: AsyncRead + AsyncWrite + Unpin,
{
    let (client_reader, client_writer) = tokio::io::split(client);
    let (upstream_reader, upstream_writer) = tokio::io::split(upstream);
    let client_to_server = relay_websocket_passthrough(client_reader, upstream_writer);
    let server_to_client = relay_server_websocket(
        upstream_reader,
        client_writer,
        FriendSyncInspector::new(),
        captured,
    );
    tokio::pin!(client_to_server, server_to_client);
    tokio::select! {
        result = &mut client_to_server => result,
        result = &mut server_to_client => result,
    }
}

async fn relay_websocket_passthrough<R, W>(mut reader: R, mut writer: W) -> Result<(), String>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|error| format!("读取 QQ WebSocket 流量失败: {error}"))?;
        if read == 0 {
            let _ = writer.shutdown().await;
            return Ok(());
        }
        writer
            .write_all(&buffer[..read])
            .await
            .map_err(|error| format!("转发 QQ WebSocket 流量失败: {error}"))?;
    }
}

async fn relay_server_websocket<R, W>(
    mut reader: R,
    mut writer: W,
    mut inspector: FriendSyncInspector,
    captured: mpsc::Sender<Vec<String>>,
) -> Result<(), String>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|error| format!("读取 QQ WebSocket 流量失败: {error}"))?;
        if read == 0 {
            let _ = writer.shutdown().await;
            return Ok(());
        }
        writer
            .write_all(&buffer[..read])
            .await
            .map_err(|error| format!("转发 QQ WebSocket 流量失败: {error}"))?;
        if let Some(open_ids) = inspector.feed(&buffer[..read]) {
            writer
                .flush()
                .await
                .map_err(|error| format!("刷新 QQ WebSocket 响应失败: {error}"))?;
            let _ = captured.try_send(open_ids);
            return Ok(());
        }
    }
}

fn validate_target_websocket_request(headers: &[u8]) -> Result<(), String> {
    let text =
        std::str::from_utf8(headers).map_err(|_| "QQ WebSocket 请求头不是有效 UTF-8".to_owned())?;
    let mut lines = text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| "QQ WebSocket 请求行为空".to_owned())?;
    let mut parts = request_line.split_whitespace();
    if !parts
        .next()
        .is_some_and(|method| method.eq_ignore_ascii_case("GET"))
    {
        return Err("QQ 农场好友同步请求不是 GET".to_owned());
    }
    let target = parts
        .next()
        .ok_or_else(|| "QQ WebSocket 请求路径缺失".to_owned())?;
    if target.split('?').next() != Some(TARGET_PATH) {
        return Err("QQ 农场好友同步请求路径不匹配".to_owned());
    }
    if text
        .split("\r\n")
        .skip(1)
        .any(|line| line.starts_with([' ', '\t']))
    {
        return Err("QQ 农场好友同步请求包含折叠请求头".to_owned());
    }
    let hosts = text
        .split("\r\n")
        .skip(1)
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("host")
                .then(|| value.trim())
        })
        .collect::<Vec<_>>();
    if hosts.len() != 1 {
        return Err("QQ 农场好友同步请求 Host 数量无效".to_owned());
    }
    let host = hosts[0];
    if !host
        .split(':')
        .next()
        .is_some_and(|host| host.eq_ignore_ascii_case(TARGET_HOST))
    {
        return Err("QQ 农场好友同步请求主机不匹配".to_owned());
    }
    if !header_contains_token(text, "upgrade", "websocket")
        || !header_contains_token(text, "connection", "upgrade")
        || header_value(text, "sec-websocket-version") != Some("13")
    {
        return Err("QQ 农场请求不是 WebSocket v13 升级".to_owned());
    }
    Ok(())
}

fn rewrite_target_websocket_request(headers: &[u8]) -> Result<Vec<u8>, String> {
    let text =
        std::str::from_utf8(headers).map_err(|_| "QQ WebSocket 请求头不是有效 UTF-8".to_owned())?;
    let mut rewritten = Vec::with_capacity(headers.len());
    for (index, line) in text.split("\r\n").enumerate() {
        if line.is_empty() {
            continue;
        }
        if index == 0 {
            rewritten.extend_from_slice(line.as_bytes());
            rewritten.extend_from_slice(b"\r\nHost: ");
            rewritten.extend_from_slice(TARGET_HOST.as_bytes());
            rewritten.extend_from_slice(b"\r\n");
            continue;
        }
        let skip = index > 0
            && line
                .split_once(':')
                .map(|(name, _)| {
                    matches!(
                        name.trim().to_ascii_lowercase().as_str(),
                        "host"
                            | "proxy-authorization"
                            | "proxy-connection"
                            | "sec-websocket-extensions"
                    )
                })
                .unwrap_or(false);
        if skip {
            continue;
        }
        rewritten.extend_from_slice(line.as_bytes());
        rewritten.extend_from_slice(b"\r\n");
    }
    rewritten.extend_from_slice(b"\r\n");
    Ok(rewritten)
}

fn response_status(headers: &[u8]) -> Option<u16> {
    std::str::from_utf8(headers)
        .ok()?
        .split("\r\n")
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

fn is_websocket_upgrade(headers: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(headers) else {
        return false;
    };
    header_contains_token(text, "upgrade", "websocket")
        && header_contains_token(text, "connection", "upgrade")
}

fn has_header(headers: &[u8], name: &str) -> bool {
    std::str::from_utf8(headers)
        .ok()
        .and_then(|text| header_value(text, name))
        .is_some()
}

fn header_value<'a>(headers: &'a str, expected_name: &str) -> Option<&'a str> {
    headers.split("\r\n").skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case(expected_name)
            .then(|| value.trim())
    })
}

fn header_contains_token(headers: &str, name: &str, expected_token: &str) -> bool {
    header_value(headers, name).is_some_and(|value| {
        value
            .split(',')
            .any(|token| token.trim().eq_ignore_ascii_case(expected_token))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_CODE: &str = "abcdefghijklmnopqrstuvwxyz123456";

    #[test]
    fn friend_sync_handshake_preserves_target_but_disables_compression() {
        let headers = format!(
            "GET /prod/ws?platform=qq&code={TEST_CODE} HTTP/1.1\r\nHost: {TARGET_HOST}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: test-key\r\nSec-WebSocket-Extensions: permessage-deflate\r\nProxy-Authorization: sentinel\r\n\r\n"
        );

        validate_target_websocket_request(headers.as_bytes()).unwrap();
        let rewritten =
            String::from_utf8(rewrite_target_websocket_request(headers.as_bytes()).unwrap())
                .unwrap();

        assert!(rewritten.starts_with(&format!(
            "GET /prod/ws?platform=qq&code={TEST_CODE} HTTP/1.1"
        )));
        assert!(!rewritten.contains("Sec-WebSocket-Extensions"));
        assert!(!rewritten.contains("Proxy-Authorization"));
        assert_eq!(rewritten.matches("Host:").count(), 1);
        assert!(rewritten.contains(&format!("Host: {TARGET_HOST}\r\n")));
        assert!(rewritten.ends_with("\r\n\r\n"));
    }

    #[test]
    fn friend_sync_rejects_duplicate_host_headers() {
        let headers = format!(
            "GET /prod/ws?code={TEST_CODE} HTTP/1.1\r\nHost: {TARGET_HOST}\r\nHost: example.com\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );

        assert!(validate_target_websocket_request(headers.as_bytes()).is_err());
    }
}
