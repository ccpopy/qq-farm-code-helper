use crate::friend_capture::inspect_client_gate_request;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha1::{Digest, Sha1};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    time::{Duration, timeout},
};

const WEBSOCKET_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
const FRIEND_REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

enum FragmentedMessage {
    Binary(Vec<u8>),
    Text,
}

struct ClientFrame {
    fin: bool,
    opcode: u8,
    payload: Vec<u8>,
}

pub async fn capture_sync_all_request<S>(
    stream: &mut S,
    request_headers: &[u8],
) -> Result<Vec<String>, String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    write_websocket_handshake(stream, request_headers).await?;
    timeout(FRIEND_REQUEST_TIMEOUT, capture_request_loop(stream))
        .await
        .map_err(|_| "等待 QQ 客户端发送 SyncAll 超时".to_owned())?
}

async fn capture_request_loop<S>(stream: &mut S) -> Result<Vec<String>, String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut fragmented: Option<FragmentedMessage> = None;
    loop {
        let frame = read_client_frame(stream).await?;
        match frame.opcode {
            0x0 => {
                let Some(message) = fragmented.as_mut() else {
                    return Err("QQ WebSocket continuation 缺少起始帧".to_owned());
                };
                if let FragmentedMessage::Binary(payload) = message {
                    append_payload(payload, &frame.payload)?;
                }
                if frame.fin {
                    if let Some(FragmentedMessage::Binary(payload)) = fragmented.take()
                        && let Some(open_ids) = handle_gate_request(stream, &payload).await?
                    {
                        return Ok(open_ids);
                    }
                    fragmented = None;
                }
            }
            0x1 | 0x2 => {
                if fragmented.is_some() {
                    return Err("QQ WebSocket 在分片消息完成前发送了新消息".to_owned());
                }
                if frame.fin {
                    if frame.opcode == 0x2
                        && let Some(open_ids) = handle_gate_request(stream, &frame.payload).await?
                    {
                        return Ok(open_ids);
                    }
                } else {
                    fragmented = Some(if frame.opcode == 0x2 {
                        FragmentedMessage::Binary(frame.payload)
                    } else {
                        FragmentedMessage::Text
                    });
                }
            }
            0x8 => {
                write_server_frame(stream, 0x8, &frame.payload).await?;
                return Err("QQ WebSocket 在发送 SyncAll 前已关闭".to_owned());
            }
            0x9 => write_server_frame(stream, 0xA, &frame.payload).await?,
            0xA => {}
            _ => return Err("QQ WebSocket 帧类型无效".to_owned()),
        }
    }
}

async fn handle_gate_request<S>(
    stream: &mut S,
    payload: &[u8],
) -> Result<Option<Vec<String>>, String>
where
    S: AsyncWrite + Unpin,
{
    let Some(request) = inspect_client_gate_request(payload) else {
        return Ok(None);
    };
    write_server_frame(stream, 0x2, &request.response).await?;
    request.sync_open_ids.transpose()
}

fn append_payload(message: &mut Vec<u8>, payload: &[u8]) -> Result<(), String> {
    if message.len().saturating_add(payload.len()) > MAX_MESSAGE_BYTES {
        return Err("QQ WebSocket 消息超过安全限制".to_owned());
    }
    message.extend_from_slice(payload);
    Ok(())
}

async fn write_websocket_handshake<S>(stream: &mut S, request_headers: &[u8]) -> Result<(), String>
where
    S: AsyncWrite + Unpin,
{
    let key = websocket_header(request_headers, "sec-websocket-key")
        .ok_or_else(|| "QQ WebSocket 请求缺少 Sec-WebSocket-Key".to_owned())?;
    let decoded_key = STANDARD
        .decode(key)
        .map_err(|_| "QQ WebSocket Key 格式无效".to_owned())?;
    if decoded_key.len() != 16 {
        return Err("QQ WebSocket Key 长度无效".to_owned());
    }
    if !websocket_header_contains_token(request_headers, "upgrade", "websocket")
        || !websocket_header_contains_token(request_headers, "connection", "upgrade")
        || websocket_header(request_headers, "sec-websocket-version") != Some("13")
    {
        return Err("QQ 请求不是 WebSocket v13 Upgrade".to_owned());
    }
    let mut digest = Sha1::new();
    digest.update(key.as_bytes());
    digest.update(WEBSOCKET_GUID.as_bytes());
    let accept = STANDARD.encode(digest.finalize());
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|error| format!("写入本地 WebSocket 握手失败: {error}"))?;
    stream
        .flush()
        .await
        .map_err(|error| format!("刷新本地 WebSocket 握手失败: {error}"))
}

fn websocket_header_contains_token(headers: &[u8], name: &str, expected: &str) -> bool {
    websocket_header(headers, name).is_some_and(|value| {
        value
            .split(',')
            .any(|token| token.trim().eq_ignore_ascii_case(expected))
    })
}

fn websocket_header<'a>(headers: &'a [u8], expected: &str) -> Option<&'a str> {
    let text = std::str::from_utf8(headers).ok()?;
    text.split("\r\n").skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case(expected)
            .then_some(value.trim())
    })
}

async fn read_client_frame<S>(stream: &mut S) -> Result<ClientFrame, String>
where
    S: AsyncRead + Unpin,
{
    let mut head = [0_u8; 2];
    stream
        .read_exact(&mut head)
        .await
        .map_err(|error| format!("读取 QQ WebSocket 帧失败: {error}"))?;
    let fin = head[0] & 0x80 != 0;
    let opcode = head[0] & 0x0f;
    if head[0] & 0x70 != 0 || !matches!(opcode, 0x0 | 0x1 | 0x2 | 0x8 | 0x9 | 0xA) {
        return Err("QQ WebSocket 帧头无效".to_owned());
    }
    if head[1] & 0x80 == 0 {
        return Err("QQ 客户端 WebSocket 帧未使用掩码".to_owned());
    }

    let payload_len = match head[1] & 0x7f {
        length @ 0..=125 => length as usize,
        126 => read_u16(stream).await? as usize,
        127 => usize::try_from(read_u64(stream).await?)
            .map_err(|_| "QQ WebSocket 帧长度无效".to_owned())?,
        _ => unreachable!(),
    };
    if payload_len > MAX_MESSAGE_BYTES || (opcode >= 0x8 && (!fin || payload_len > 125)) {
        return Err("QQ WebSocket 帧超过安全限制".to_owned());
    }

    let mut mask = [0_u8; 4];
    stream
        .read_exact(&mut mask)
        .await
        .map_err(|error| format!("读取 QQ WebSocket 掩码失败: {error}"))?;
    let mut payload = vec![0_u8; payload_len];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(|error| format!("读取 QQ WebSocket 负载失败: {error}"))?;
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[index % mask.len()];
    }
    Ok(ClientFrame {
        fin,
        opcode,
        payload,
    })
}

async fn read_u16<S>(stream: &mut S) -> Result<u16, String>
where
    S: AsyncRead + Unpin,
{
    let mut bytes = [0_u8; 2];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|error| format!("读取 QQ WebSocket 长度失败: {error}"))?;
    Ok(u16::from_be_bytes(bytes))
}

async fn read_u64<S>(stream: &mut S) -> Result<u64, String>
where
    S: AsyncRead + Unpin,
{
    let mut bytes = [0_u8; 8];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|error| format!("读取 QQ WebSocket 长度失败: {error}"))?;
    if bytes[0] & 0x80 != 0 {
        return Err("QQ WebSocket 帧长度无效".to_owned());
    }
    Ok(u64::from_be_bytes(bytes))
}

async fn write_server_frame<S>(stream: &mut S, opcode: u8, payload: &[u8]) -> Result<(), String>
where
    S: AsyncWrite + Unpin,
{
    let mut frame = Vec::with_capacity(payload.len() + 10);
    frame.push(0x80 | opcode);
    match payload.len() {
        length @ 0..=125 => frame.push(length as u8),
        length @ 126..=65535 => {
            frame.push(126);
            frame.extend_from_slice(&(length as u16).to_be_bytes());
        }
        length => {
            frame.push(127);
            frame.extend_from_slice(&(length as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(payload);
    stream
        .write_all(&frame)
        .await
        .map_err(|error| format!("写入本地 WebSocket 响应失败: {error}"))?;
    stream
        .flush()
        .await
        .map_err(|error| format!("刷新本地 WebSocket 响应失败: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;
    use std::collections::HashMap;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};
    use tokio::sync::oneshot;

    const REQUEST_HEADERS: &[u8] = b"GET /prod/ws?code=synthetic HTTP/1.1\r\nHost: gate-obt.nqf.qq.com\r\nConnection: keep-alive, Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n";

    #[derive(Clone, PartialEq, Message)]
    struct TestEnvelope {
        #[prost(message, optional, tag = "1")]
        meta: Option<TestMeta>,
        #[prost(bytes = "vec", tag = "2")]
        body: Vec<u8>,
        #[prost(string, tag = "3")]
        token: String,
    }

    #[derive(Clone, PartialEq, Message)]
    struct TestMeta {
        #[prost(string, tag = "1")]
        service_name: String,
        #[prost(string, tag = "2")]
        method_name: String,
        #[prost(int32, tag = "3")]
        message_type: i32,
        #[prost(int64, tag = "4")]
        client_seq: i64,
        #[prost(int64, tag = "5")]
        server_seq: i64,
        #[prost(int64, tag = "6")]
        error_code: i64,
        #[prost(string, tag = "7")]
        error_message: String,
        #[prost(map = "string, bytes", tag = "8")]
        metadata: HashMap<String, Vec<u8>>,
    }

    #[derive(Clone, PartialEq, Message)]
    struct TestSyncAllRequest {
        #[prost(string, repeated, tag = "2")]
        open_ids: Vec<String>,
    }

    fn gate_request(method_name: &str, body: Vec<u8>) -> Vec<u8> {
        TestEnvelope {
            meta: Some(TestMeta {
                service_name: "gamepb.friendpb.FriendService".to_owned(),
                method_name: method_name.to_owned(),
                message_type: 1,
                client_seq: 7,
                server_seq: 11,
                error_code: 0,
                error_message: String::new(),
                metadata: HashMap::new(),
            }),
            body,
            token: "token".to_owned(),
        }
        .encode_to_vec()
    }

    fn masked_client_frame(payload: &[u8]) -> Vec<u8> {
        let mask = [0x12, 0x34, 0x56, 0x78];
        let mut frame = vec![0x82];
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
        frame
    }

    #[tokio::test]
    async fn writes_the_rfc_websocket_accept_value() {
        let (mut server, mut client) = duplex(1024);
        let server_task = tokio::spawn(async move {
            write_websocket_handshake(&mut server, REQUEST_HEADERS)
                .await
                .unwrap();
        });
        let mut response = vec![0_u8; 256];
        let read = client.read(&mut response).await.unwrap();
        server_task.await.unwrap();
        let response = std::str::from_utf8(&response[..read]).unwrap();

        assert!(response.starts_with("HTTP/1.1 101 Switching Protocols\r\n"));
        assert!(response.contains("Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n"));
    }

    #[tokio::test]
    async fn reads_and_unmasks_extended_client_frames() {
        let payload = vec![0xA5; 130];
        let frame = masked_client_frame(&payload);
        let (mut server, mut client) = duplex(1024);
        client.write_all(&frame).await.unwrap();

        let parsed = read_client_frame(&mut server).await.unwrap();

        assert!(parsed.fin);
        assert_eq!(parsed.opcode, 0x2);
        assert_eq!(parsed.payload, payload);
    }

    #[tokio::test]
    async fn captures_sync_all_from_the_same_local_login_session() {
        let first = masked_client_frame(&gate_request("GetShareKey", Vec::new()));
        let sync_body = TestSyncAllRequest {
            open_ids: vec!["open-a".to_owned(), "open-b".to_owned()],
        }
        .encode_to_vec();
        let second = masked_client_frame(&gate_request("SyncAll", sync_body));
        let (mut server, mut client) = duplex(64 * 1024);
        let (release_client, keep_client_open) = oneshot::channel();

        let client_task = tokio::spawn(async move {
            client.write_all(&first).await.unwrap();
            client.write_all(&second).await.unwrap();
            let _ = keep_client_open.await;
        });
        let open_ids = capture_sync_all_request(&mut server, REQUEST_HEADERS)
            .await
            .unwrap();
        let _ = release_client.send(());
        client_task.await.unwrap();

        assert_eq!(open_ids, vec!["open-a".to_owned(), "open-b".to_owned()]);
    }
}
