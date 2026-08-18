use crate::friend_proxy::open_target_websocket;
use rustls::ClientConfig;
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const CAPTURE_FORMAT_VERSION: u8 = 1;
const MAX_FRAME_PAYLOAD: usize = 64 * 1024 * 1024;
const MAX_MESSAGE_PAYLOAD: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CaptureDirection {
    ClientToServer,
    ServerToClient,
}

impl CaptureDirection {
    fn file_label(self) -> &'static str {
        match self {
            Self::ClientToServer => "send",
            Self::ServerToClient => "recv",
        }
    }

    fn expects_mask(self) -> bool {
        matches!(self, Self::ClientToServer)
    }

    fn description(self) -> &'static str {
        match self {
            Self::ClientToServer => "QQ 客户端到官方网关",
            Self::ServerToClient => "官方网关到 QQ 客户端",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CaptureSummary {
    pub directory: PathBuf,
    pub message_count: u64,
    pub total_bytes: u64,
}

pub(crate) struct ProtocolCaptureSession {
    directory: PathBuf,
    writer: Mutex<CaptureWriter>,
}

struct CaptureWriter {
    manifest: File,
    message_count: u64,
    total_bytes: u64,
}

#[derive(Serialize)]
struct SessionMetadata {
    format_version: u8,
    started_at_ms: u128,
    content: &'static str,
    privacy: &'static str,
}

#[derive(Serialize)]
struct ManifestRecord<'a> {
    sequence: u64,
    timestamp_ms: u128,
    direction: CaptureDirection,
    file: &'a str,
    size: usize,
}

impl ProtocolCaptureSession {
    pub(crate) fn create(capture_root: &Path) -> Result<Arc<Self>, String> {
        fs::create_dir_all(capture_root)
            .map_err(|error| format!("创建协议抓包根目录失败: {error}"))?;
        let started_at_ms = unix_timestamp_ms();
        let directory = create_unique_session_directory(capture_root, started_at_ms)?;
        let metadata = SessionMetadata {
            format_version: CAPTURE_FORMAT_VERSION,
            started_at_ms,
            content: "完整的 QQ 农场 WebSocket 二进制消息（已去除帧封装）",
            privacy: "不保存 HTTP/WebSocket 握手头、登录 URL 或 Code",
        };
        let metadata_bytes = serde_json::to_vec_pretty(&metadata)
            .map_err(|error| format!("生成协议抓包会话信息失败: {error}"))?;
        fs::write(directory.join("session.json"), metadata_bytes)
            .map_err(|error| format!("写入协议抓包会话信息失败: {error}"))?;
        let manifest = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(directory.join("manifest.jsonl"))
            .map_err(|error| format!("创建协议抓包清单失败: {error}"))?;

        Ok(Arc::new(Self {
            directory,
            writer: Mutex::new(CaptureWriter {
                manifest,
                message_count: 0,
                total_bytes: 0,
            }),
        }))
    }

    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    pub(crate) fn summary(&self) -> CaptureSummary {
        let writer = self
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        CaptureSummary {
            directory: self.directory.clone(),
            message_count: writer.message_count,
            total_bytes: writer.total_bytes,
        }
    }

    fn store_message(&self, direction: CaptureDirection, payload: &[u8]) -> Result<(), String> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| "协议抓包写入器状态异常".to_owned())?;
        let sequence = writer.message_count + 1;
        let file_name = format!("{sequence:06}-{}.bin", direction.file_label());
        let final_path = self.directory.join(&file_name);
        let temporary_path = self.directory.join(format!(".{file_name}.part"));
        fs::write(&temporary_path, payload)
            .map_err(|error| format!("写入协议抓包临时文件失败: {error}"))?;
        if let Err(error) = fs::rename(&temporary_path, &final_path) {
            let _ = fs::remove_file(&temporary_path);
            return Err(format!("提交协议抓包文件失败: {error}"));
        }

        let record = ManifestRecord {
            sequence,
            timestamp_ms: unix_timestamp_ms(),
            direction,
            file: &file_name,
            size: payload.len(),
        };
        serde_json::to_writer(&mut writer.manifest, &record)
            .map_err(|error| format!("写入协议抓包清单失败: {error}"))?;
        writer
            .manifest
            .write_all(b"\n")
            .and_then(|_| writer.manifest.flush())
            .map_err(|error| format!("刷新协议抓包清单失败: {error}"))?;
        writer.message_count = sequence;
        writer.total_bytes = writer.total_bytes.saturating_add(payload.len() as u64);
        Ok(())
    }
}

pub(crate) async fn relay_target_websocket<S>(
    client: S,
    request_headers: Vec<u8>,
    upstream_tls: Arc<ClientConfig>,
    session: Arc<ProtocolCaptureSession>,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (client, upstream) = open_target_websocket(client, request_headers, upstream_tls).await?;
    let (client_reader, client_writer) = tokio::io::split(client);
    let (upstream_reader, upstream_writer) = tokio::io::split(upstream);

    tokio::try_join!(
        relay_direction(
            client_reader,
            upstream_writer,
            CaptureDirection::ClientToServer,
            session.clone(),
        ),
        relay_direction(
            upstream_reader,
            client_writer,
            CaptureDirection::ServerToClient,
            session,
        ),
    )?;
    Ok(())
}

async fn relay_direction<R, W>(
    mut reader: R,
    mut writer: W,
    direction: CaptureDirection,
    session: Arc<ProtocolCaptureSession>,
) -> Result<(), String>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut inspector = WebSocketInspector::new(direction.expects_mask());
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .await
            .map_err(|error| format!("读取{}协议流量失败: {error}", direction.description()))?;
        if read == 0 {
            inspector.finish()?;
            let _ = writer.shutdown().await;
            return Ok(());
        }
        writer
            .write_all(&buffer[..read])
            .await
            .map_err(|error| format!("转发{}协议流量失败: {error}", direction.description()))?;
        for payload in inspector.feed(&buffer[..read])? {
            session.store_message(direction, &payload)?;
        }
    }
}

fn create_unique_session_directory(root: &Path, started_at_ms: u128) -> Result<PathBuf, String> {
    for suffix in 0..1000_u16 {
        let name = if suffix == 0 {
            format!("session-{started_at_ms}")
        } else {
            format!("session-{started_at_ms}-{suffix:03}")
        };
        let directory = root.join(name);
        match fs::create_dir(&directory) {
            Ok(()) => return Ok(directory),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("创建协议抓包会话目录失败: {error}")),
        }
    }
    Err("同一时刻创建的协议抓包会话过多".to_owned())
}

fn unix_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FragmentKind {
    Text,
    Binary,
}

struct FragmentedMessage {
    kind: FragmentKind,
    payload: Vec<u8>,
}

struct WebSocketInspector {
    buffer: Vec<u8>,
    expected_mask: bool,
    fragmented: Option<FragmentedMessage>,
}

impl WebSocketInspector {
    fn new(expected_mask: bool) -> Self {
        Self {
            buffer: Vec::new(),
            expected_mask,
            fragmented: None,
        }
    }

    fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        self.buffer.extend_from_slice(bytes);
        let mut consumed = 0;
        let mut messages = Vec::new();
        while let Some((frame, frame_size)) =
            parse_frame(&self.buffer[consumed..], self.expected_mask)?
        {
            consumed += frame_size;
            self.accept_frame(frame, &mut messages)?;
        }
        if consumed > 0 {
            self.buffer.drain(..consumed);
        }
        if self.buffer.len() > MAX_FRAME_PAYLOAD + 14 {
            return Err("WebSocket 帧缓冲超过安全限制".to_owned());
        }
        Ok(messages)
    }

    fn finish(&self) -> Result<(), String> {
        if !self.buffer.is_empty() {
            return Err("WebSocket 连接结束时仍有不完整帧".to_owned());
        }
        if self.fragmented.is_some() {
            return Err("WebSocket 连接结束时仍有未完成的分片消息".to_owned());
        }
        Ok(())
    }

    fn accept_frame(
        &mut self,
        frame: WebSocketFrame,
        output: &mut Vec<Vec<u8>>,
    ) -> Result<(), String> {
        match frame.opcode {
            0x0 => {
                let fragmented = self
                    .fragmented
                    .as_mut()
                    .ok_or_else(|| "收到没有起始帧的 WebSocket continuation".to_owned())?;
                extend_message(&mut fragmented.payload, &frame.payload)?;
                if frame.fin {
                    let complete = self.fragmented.take().expect("fragment checked above");
                    if complete.kind == FragmentKind::Binary {
                        output.push(complete.payload);
                    }
                }
            }
            0x1 | 0x2 => {
                if self.fragmented.is_some() {
                    return Err("上一条 WebSocket 分片消息尚未结束".to_owned());
                }
                let kind = if frame.opcode == 0x2 {
                    FragmentKind::Binary
                } else {
                    FragmentKind::Text
                };
                if frame.fin {
                    if kind == FragmentKind::Binary {
                        output.push(frame.payload);
                    }
                } else {
                    self.fragmented = Some(FragmentedMessage {
                        kind,
                        payload: frame.payload,
                    });
                }
            }
            0x8..=0xA => {}
            _ => return Err(format!("不支持的 WebSocket opcode: {}", frame.opcode)),
        }
        Ok(())
    }
}

struct WebSocketFrame {
    fin: bool,
    opcode: u8,
    payload: Vec<u8>,
}

fn parse_frame(
    bytes: &[u8],
    expected_mask: bool,
) -> Result<Option<(WebSocketFrame, usize)>, String> {
    if bytes.len() < 2 {
        return Ok(None);
    }
    let fin = bytes[0] & 0x80 != 0;
    if bytes[0] & 0x70 != 0 {
        return Err("协议模式不支持带 RSV 扩展位的 WebSocket 帧".to_owned());
    }
    let opcode = bytes[0] & 0x0F;
    let masked = bytes[1] & 0x80 != 0;
    if masked != expected_mask {
        let expected = if expected_mask {
            "带掩码"
        } else {
            "不带掩码"
        };
        return Err(format!("WebSocket 帧方向异常，预期{expected}"));
    }

    let mut cursor = 2;
    let payload_length = match bytes[1] & 0x7F {
        length @ 0..=125 => usize::from(length),
        126 => {
            if bytes.len() < cursor + 2 {
                return Ok(None);
            }
            let length = u16::from_be_bytes([bytes[cursor], bytes[cursor + 1]]);
            cursor += 2;
            usize::from(length)
        }
        127 => {
            if bytes.len() < cursor + 8 {
                return Ok(None);
            }
            let length = u64::from_be_bytes(
                bytes[cursor..cursor + 8]
                    .try_into()
                    .expect("eight-byte websocket length"),
            );
            cursor += 8;
            usize::try_from(length).map_err(|_| "WebSocket 帧长度超出平台限制".to_owned())?
        }
        _ => unreachable!(),
    };
    if payload_length > MAX_FRAME_PAYLOAD {
        return Err("WebSocket 单帧超过 64 MiB 安全限制".to_owned());
    }
    if opcode >= 0x8 && (!fin || payload_length > 125) {
        return Err("WebSocket 控制帧格式无效".to_owned());
    }

    let mask = if masked {
        if bytes.len() < cursor + 4 {
            return Ok(None);
        }
        let mask: [u8; 4] = bytes[cursor..cursor + 4]
            .try_into()
            .expect("four-byte websocket mask");
        cursor += 4;
        Some(mask)
    } else {
        None
    };
    let frame_size = cursor
        .checked_add(payload_length)
        .ok_or_else(|| "WebSocket 帧长度溢出".to_owned())?;
    if bytes.len() < frame_size {
        return Ok(None);
    }
    let mut payload = bytes[cursor..frame_size].to_vec();
    if let Some(mask) = mask {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % mask.len()];
        }
    }
    Ok(Some((
        WebSocketFrame {
            fin,
            opcode,
            payload,
        },
        frame_size,
    )))
}

fn extend_message(target: &mut Vec<u8>, value: &[u8]) -> Result<(), String> {
    let next_length = target
        .len()
        .checked_add(value.len())
        .ok_or_else(|| "WebSocket 分片消息长度溢出".to_owned())?;
    if next_length > MAX_MESSAGE_PAYLOAD {
        return Err("WebSocket 分片消息超过 64 MiB 安全限制".to_owned());
    }
    target.extend_from_slice(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn websocket_frame(payload: &[u8], fin: bool, opcode: u8, masked: bool) -> Vec<u8> {
        assert!(payload.len() <= 125);
        let mask = [0x12, 0x34, 0x56, 0x78];
        let mut frame = vec![(if fin { 0x80 } else { 0 }) | opcode];
        frame.push((if masked { 0x80 } else { 0 }) | payload.len() as u8);
        if masked {
            frame.extend_from_slice(&mask);
            frame.extend(
                payload
                    .iter()
                    .enumerate()
                    .map(|(index, byte)| byte ^ mask[index % mask.len()]),
            );
        } else {
            frame.extend_from_slice(payload);
        }
        frame
    }

    #[test]
    fn reconstructs_a_masked_client_message_across_tcp_chunks() {
        let frame = websocket_frame(b"gate-request", true, 0x2, true);
        let mut inspector = WebSocketInspector::new(true);

        assert!(inspector.feed(&frame[..3]).unwrap().is_empty());
        assert_eq!(
            inspector.feed(&frame[3..]).unwrap(),
            vec![b"gate-request".to_vec()]
        );
        inspector.finish().unwrap();
    }

    #[test]
    fn reconstructs_fragmented_server_binary_messages_with_control_frames() {
        let mut bytes = websocket_frame(b"gate-", false, 0x2, false);
        bytes.extend(websocket_frame(b"ping", true, 0x9, false));
        bytes.extend(websocket_frame(b"reply", true, 0x0, false));
        let mut inspector = WebSocketInspector::new(false);

        assert_eq!(
            inspector.feed(&bytes).unwrap(),
            vec![b"gate-reply".to_vec()]
        );
    }

    #[test]
    fn rejects_frames_with_the_wrong_mask_direction() {
        let frame = websocket_frame(b"unexpected", true, 0x2, false);
        let mut inspector = WebSocketInspector::new(true);

        assert!(inspector.feed(&frame).unwrap_err().contains("预期带掩码"));
    }

    #[test]
    fn stores_globally_ordered_binary_files_without_login_headers() {
        let root = std::env::temp_dir().join(format!(
            "qq-farm-protocol-capture-{}-{}",
            std::process::id(),
            unix_timestamp_ms()
        ));
        let session = ProtocolCaptureSession::create(&root).unwrap();
        session
            .store_message(CaptureDirection::ClientToServer, b"request")
            .unwrap();
        session
            .store_message(CaptureDirection::ServerToClient, b"reply")
            .unwrap();

        let summary = session.summary();
        assert_eq!(summary.message_count, 2);
        assert_eq!(summary.total_bytes, 12);
        assert_eq!(
            fs::read(summary.directory.join("000001-send.bin")).unwrap(),
            b"request"
        );
        assert_eq!(
            fs::read(summary.directory.join("000002-recv.bin")).unwrap(),
            b"reply"
        );
        let session_metadata = fs::read_to_string(summary.directory.join("session.json")).unwrap();
        let manifest = fs::read_to_string(summary.directory.join("manifest.jsonl")).unwrap();
        assert!(!session_metadata.to_ascii_lowercase().contains("code="));
        assert!(!manifest.to_ascii_lowercase().contains("code="));
        assert!(manifest.contains("client_to_server"));
        assert!(manifest.contains("server_to_client"));

        drop(session);
        fs::remove_dir_all(root).unwrap();
    }
}
