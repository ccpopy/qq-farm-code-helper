use crate::{
    certificates::TARGET_HOST,
    friend_capture::{
        CapturedFriendReply, FriendReplyKind, ServerFriendInspector, encode_empty_sync_all_request,
    },
    proxy::read_headers,
};
use rustls::ClientConfig;
use std::{
    collections::HashSet,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, copy_bidirectional},
    net::{TcpStream, lookup_host},
    sync::mpsc,
    time::{sleep, timeout},
};
use tokio_rustls::TlsConnector;

const TARGET_PATH: &str = "/prod/ws";
const PASSIVE_CAPTURE_WAIT: Duration = Duration::from_secs(5);
const ACTIVE_CAPTURE_WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FriendCaptureOutcome {
    pub own_gid: Option<String>,
    pub gids: Vec<String>,
    pub warning: Option<String>,
}

pub(crate) async fn relay_target_websocket<S>(
    client: S,
    request_headers: Vec<u8>,
    upstream_tls: Arc<ClientConfig>,
) -> Result<FriendCaptureOutcome, String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (client, upstream) = open_target_websocket(client, request_headers, upstream_tls).await?;
    relay_websocket(client, upstream).await
}

pub(crate) async fn open_target_websocket<S>(
    mut client: S,
    request_headers: Vec<u8>,
    upstream_tls: Arc<ClientConfig>,
) -> Result<(S, tokio_rustls::client::TlsStream<TcpStream>), String>
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
        let _ = copy_bidirectional(&mut client, &mut upstream).await;
        return Err(format!("QQ 农场网关拒绝 WebSocket 登录 (HTTP {status})"));
    }

    Ok((client, upstream))
}

pub(crate) fn target_websocket_url(headers: &[u8]) -> Result<String, String> {
    validate_target_websocket_request(headers)?;
    let text =
        std::str::from_utf8(headers).map_err(|_| "QQ WebSocket 请求头不是有效 UTF-8".to_owned())?;
    let target = text
        .split("\r\n")
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| "QQ WebSocket 请求目标缺失".to_owned())?;
    let host = header_value(text, "host").ok_or_else(|| "QQ WebSocket Host 缺失".to_owned())?;
    Ok(format!("wss://{host}{target}"))
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

async fn relay_websocket<C, U>(client: C, upstream: U) -> Result<FriendCaptureOutcome, String>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: AsyncRead + AsyncWrite + Unpin,
{
    let (client_reader, client_writer) = tokio::io::split(client);
    let (upstream_reader, upstream_writer) = tokio::io::split(upstream);
    let (injection_sender, injection_receiver) = mpsc::channel(1);
    let (reply_sender, mut reply_receiver) = mpsc::channel(16);
    let latest_server_seq = Arc::new(AtomicI64::new(0));
    let own_gid = Arc::new(OnceLock::new());
    let client_to_server =
        relay_client_websocket(client_reader, upstream_writer, injection_receiver);
    let server_to_client = relay_server_websocket(
        upstream_reader,
        client_writer,
        reply_sender,
        latest_server_seq.clone(),
        own_gid.clone(),
    );
    tokio::pin!(client_to_server, server_to_client);

    let passive_wait = sleep(PASSIVE_CAPTURE_WAIT);
    tokio::pin!(passive_wait);
    let mut fallback_gids = Vec::new();
    let mut fallback_seen = HashSet::new();
    loop {
        tokio::select! {
            result = &mut client_to_server => {
                return transport_ended("QQ 客户端到官方网关", result, fallback_gids, own_gid.get().cloned());
            }
            result = &mut server_to_client => {
                return transport_ended("官方网关到 QQ 客户端", result, fallback_gids, own_gid.get().cloned());
            }
            reply = reply_receiver.recv() => {
                let Some(reply) = reply else {
                    return Err("官方好友响应捕获通道已关闭".to_owned());
                };
                if reply.kind == FriendReplyKind::SyncAll && !reply.gids.is_empty() {
                    return Ok(capture_outcome(&own_gid, reply.gids, None));
                }
                merge_gids(&mut fallback_gids, &mut fallback_seen, &reply.gids);
            }
            _ = &mut passive_wait => break,
        }
    }

    if !fallback_gids.is_empty() {
        return Ok(capture_outcome(
            &own_gid,
            fallback_gids,
            Some("QQ 客户端未返回 SyncAll，已采用官方 GetGameFriends 响应中的 GID".to_owned()),
        ));
    }

    let server_seq = latest_server_seq.load(Ordering::Relaxed);
    let (active_client_seq, active_frame) = build_active_sync_all_frame(server_seq)?;
    injection_sender
        .send(active_frame)
        .await
        .map_err(|_| "QQ 官方连接已结束，无法主动发起 SyncAll".to_owned())?;

    let active_wait = sleep(ACTIVE_CAPTURE_WAIT);
    tokio::pin!(active_wait);
    loop {
        tokio::select! {
            result = &mut client_to_server => {
                return transport_ended("QQ 客户端到官方网关", result, fallback_gids, own_gid.get().cloned());
            }
            result = &mut server_to_client => {
                return transport_ended("官方网关到 QQ 客户端", result, fallback_gids, own_gid.get().cloned());
            }
            reply = reply_receiver.recv() => {
                let Some(reply) = reply else {
                    return Err("官方好友响应捕获通道已关闭".to_owned());
                };
                if reply.kind == FriendReplyKind::SyncAll {
                    if !reply.gids.is_empty() {
                        let warning = (reply.client_seq == active_client_seq).then(|| {
                            "QQ 客户端未主动触发 SyncAll，已由 Helper 通过当前官方会话发起"
                                .to_owned()
                        });
                        return Ok(capture_outcome(&own_gid, reply.gids, warning));
                    }
                    if reply.client_seq == active_client_seq {
                        return Ok(capture_outcome(
                            &own_gid,
                            Vec::new(),
                            Some(
                                "QQ 客户端未主动触发 SyncAll；Helper 主动请求成功，但官方返回 0 位好友"
                                    .to_owned(),
                            ),
                        ));
                    }
                }
                merge_gids(&mut fallback_gids, &mut fallback_seen, &reply.gids);
            }
            _ = &mut active_wait => {
                if fallback_gids.is_empty() {
                    return Ok(capture_outcome(
                        &own_gid,
                        Vec::new(),
                        Some(
                            "QQ 客户端未主动触发 SyncAll，Helper 主动请求后仍未收到可识别的官方好友响应"
                                .to_owned(),
                        ),
                    ));
                }
                return Ok(capture_outcome(
                    &own_gid,
                    fallback_gids,
                    Some(
                        "主动 SyncAll 未返回好友，已采用官方 GetGameFriends 响应中的 GID"
                            .to_owned(),
                    ),
                ));
            }
        }
    }
}

fn capture_outcome(
    own_gid: &OnceLock<String>,
    gids: Vec<String>,
    warning: Option<String>,
) -> FriendCaptureOutcome {
    FriendCaptureOutcome {
        own_gid: own_gid.get().cloned(),
        gids,
        warning,
    }
}

fn transport_ended(
    direction: &str,
    result: Result<(), String>,
    fallback_gids: Vec<String>,
    own_gid: Option<String>,
) -> Result<FriendCaptureOutcome, String> {
    if !fallback_gids.is_empty() {
        return Ok(FriendCaptureOutcome {
            own_gid,
            gids: fallback_gids,
            warning: Some(format!(
                "{direction}连接提前结束，已采用此前官方响应中的部分好友 GID"
            )),
        });
    }
    match result {
        Ok(()) => Err(format!("{direction}连接在返回好友前已关闭")),
        Err(error) => Err(error),
    }
}

fn merge_gids(target: &mut Vec<String>, seen: &mut HashSet<String>, values: &[String]) {
    for gid in values {
        if seen.insert(gid.clone()) {
            target.push(gid.clone());
        }
    }
}

async fn relay_client_websocket<R, W>(
    mut reader: R,
    mut writer: W,
    mut injected: mpsc::Receiver<Vec<u8>>,
) -> Result<(), String>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        tokio::select! {
            read = reader.read(&mut buffer) => {
                let read = read.map_err(|error| format!("读取 QQ WebSocket 流量失败: {error}"))?;
                if read == 0 {
                    let _ = writer.shutdown().await;
                    return Ok(());
                }
                writer
                    .write_all(&buffer[..read])
                    .await
                    .map_err(|error| format!("转发 QQ WebSocket 流量失败: {error}"))?;
            }
            Some(frame) = injected.recv() => {
                writer
                    .write_all(&frame)
                    .await
                    .map_err(|error| format!("主动发送 SyncAll 失败: {error}"))?;
                writer
                    .flush()
                    .await
                    .map_err(|error| format!("刷新主动 SyncAll 请求失败: {error}"))?;
            }
        }
    }
}

async fn relay_server_websocket<R, W>(
    mut reader: R,
    mut writer: W,
    replies: mpsc::Sender<CapturedFriendReply>,
    latest_server_seq: Arc<AtomicI64>,
    own_gid: Arc<OnceLock<String>>,
) -> Result<(), String>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut inspector = ServerFriendInspector::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|error| format!("读取 QQ WebSocket 响应失败: {error}"))?;
        if read == 0 {
            let _ = writer.shutdown().await;
            return Ok(());
        }
        writer
            .write_all(&buffer[..read])
            .await
            .map_err(|error| format!("转发 QQ WebSocket 响应失败: {error}"))?;
        let inspection = inspector.feed(&buffer[..read]);
        if let Some(captured_own_gid) = inspection.own_gid {
            let _ = own_gid.set(captured_own_gid);
        }
        if inspection.latest_server_seq > latest_server_seq.load(Ordering::Relaxed) {
            latest_server_seq.store(inspection.latest_server_seq, Ordering::Relaxed);
        }
        for reply in inspection.friend_replies {
            let _ = replies.try_send(reply);
        }
    }
}

fn build_active_sync_all_frame(server_seq: i64) -> Result<(i64, Vec<u8>), String> {
    let mut random = [0_u8; 4];
    getrandom::fill(&mut random).map_err(|error| format!("生成 SyncAll 请求序号失败: {error}"))?;
    let client_seq = i64::from(u32::from_be_bytes(random) & 0x3FFF_FFFF) + 1_000_000_000;
    let payload = encode_empty_sync_all_request(client_seq, server_seq, random_gateway_token()?);
    Ok((client_seq, masked_client_binary_frame(&payload)?))
}

fn random_gateway_token() -> Result<String, String> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut random = [0_u8; 96];
    getrandom::fill(&mut random).map_err(|error| format!("生成网关请求 Token 失败: {error}"))?;
    let mut token = String::with_capacity(random.len() + 1);
    for byte in random {
        token.push(ALPHABET[usize::from(byte) % ALPHABET.len()] as char);
    }
    token.push('=');
    Ok(token)
}

fn masked_client_binary_frame(payload: &[u8]) -> Result<Vec<u8>, String> {
    let mut mask = [0_u8; 4];
    getrandom::fill(&mut mask).map_err(|error| format!("生成 WebSocket 掩码失败: {error}"))?;
    let mut frame = Vec::with_capacity(payload.len() + 14);
    frame.push(0x82);
    match payload.len() {
        length @ 0..=125 => frame.push(0x80 | length as u8),
        length @ 126..=65535 => {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(length as u16).to_be_bytes());
        }
        length => {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(length as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % mask.len()]),
    );
    Ok(frame)
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
        return Err("QQ WebSocket 请求包含折叠请求头".to_owned());
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
        return Err("QQ WebSocket 请求 Host 数量无效".to_owned());
    }
    if !hosts[0]
        .split(':')
        .next()
        .is_some_and(|host| host.eq_ignore_ascii_case(TARGET_HOST))
    {
        return Err("QQ WebSocket 请求主机不匹配".to_owned());
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
        let skip = line
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
    fn transparent_handshake_preserves_target_but_disables_compression() {
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
    fn preserves_the_complete_websocket_url_for_strict_comparison() {
        let headers = format!(
            "GET /prod/ws?platform=qq&code={TEST_CODE}&device=Windows HTTP/1.1\r\nHost: {TARGET_HOST}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );

        assert_eq!(
            target_websocket_url(headers.as_bytes()).unwrap(),
            format!("wss://{TARGET_HOST}/prod/ws?platform=qq&code={TEST_CODE}&device=Windows")
        );
    }

    #[test]
    fn active_sync_all_uses_a_masked_client_binary_frame() {
        let payload = b"synthetic-gate-message";
        let frame = masked_client_binary_frame(payload).unwrap();

        assert_eq!(frame[0], 0x82);
        assert_ne!(frame[1] & 0x80, 0);
        let length = usize::from(frame[1] & 0x7f);
        assert_eq!(length, payload.len());
        let mask = &frame[2..6];
        let decoded = frame[6..]
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4])
            .collect::<Vec<_>>();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn rejects_duplicate_host_headers() {
        let headers = format!(
            "GET /prod/ws?code={TEST_CODE} HTTP/1.1\r\nHost: {TARGET_HOST}\r\nHost: example.com\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );

        assert!(validate_target_websocket_request(headers.as_bytes()).is_err());
    }
}
