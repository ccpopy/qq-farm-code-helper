use prost::Message;
use std::collections::HashSet;

const MAX_FRAME_PAYLOAD: usize = 8 * 1024 * 1024;
const MAX_BUFFERED_BYTES: usize = MAX_FRAME_PAYLOAD + 16 * 1024 + 32;
const MAX_FRIEND_OPEN_IDS: usize = 200;
const MAX_FRIEND_OPEN_ID_LENGTH: usize = 128;
const FRIEND_SERVICE: &str = "gamepb.friendpb.FriendService";

pub struct FriendSyncInspector {
    buffer: Vec<u8>,
    fragmented: Option<FragmentedMessage>,
    enabled: bool,
}

enum FragmentedMessage {
    Binary(Vec<u8>),
    Text,
}

impl FriendSyncInspector {
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            fragmented: None,
            enabled: true,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Option<Vec<String>> {
        if !self.enabled || bytes.is_empty() {
            return None;
        }
        if self.buffer.len().saturating_add(bytes.len()) > MAX_BUFFERED_BYTES {
            self.disable();
            return None;
        }
        self.buffer.extend_from_slice(bytes);

        let mut consumed = 0;
        loop {
            match parse_server_frame(&self.buffer[consumed..]) {
                FrameParse::Incomplete => break,
                FrameParse::Invalid => {
                    self.disable();
                    return None;
                }
                FrameParse::Complete(frame, frame_bytes) => {
                    consumed += frame_bytes;
                    if let Some(open_ids) = self.process_frame(frame) {
                        return Some(open_ids);
                    }
                    if !self.enabled {
                        return None;
                    }
                }
            }
        }
        if consumed > 0 {
            self.buffer.drain(..consumed);
        }
        None
    }

    fn process_frame(&mut self, frame: Frame) -> Option<Vec<String>> {
        match frame.opcode {
            0x0 => self.process_continuation(frame),
            0x1 => self.process_data_start(frame, false),
            0x2 => self.process_data_start(frame, true),
            0x8..=0xA => None,
            _ => {
                self.disable();
                None
            }
        }
    }

    fn process_data_start(&mut self, frame: Frame, binary: bool) -> Option<Vec<String>> {
        if self.fragmented.is_some() || frame.compressed {
            self.disable();
            return None;
        }
        if frame.fin {
            return binary
                .then(|| extract_sync_all_open_ids(&frame.payload))
                .flatten();
        }
        self.fragmented = Some(if binary {
            FragmentedMessage::Binary(frame.payload)
        } else {
            FragmentedMessage::Text
        });
        None
    }

    fn process_continuation(&mut self, frame: Frame) -> Option<Vec<String>> {
        if frame.compressed {
            self.disable();
            return None;
        }
        let Some(fragmented) = self.fragmented.as_mut() else {
            self.disable();
            return None;
        };
        if let FragmentedMessage::Binary(message) = fragmented {
            if message.len().saturating_add(frame.payload.len()) > MAX_FRAME_PAYLOAD {
                self.disable();
                return None;
            }
            message.extend_from_slice(&frame.payload);
        }
        if !frame.fin {
            return None;
        }
        match self.fragmented.take() {
            Some(FragmentedMessage::Binary(message)) => extract_sync_all_open_ids(&message),
            _ => None,
        }
    }

    fn disable(&mut self) {
        self.enabled = false;
        self.buffer.clear();
        self.fragmented = None;
    }
}

struct Frame {
    fin: bool,
    compressed: bool,
    opcode: u8,
    payload: Vec<u8>,
}

enum FrameParse {
    Incomplete,
    Invalid,
    Complete(Frame, usize),
}

fn parse_server_frame(bytes: &[u8]) -> FrameParse {
    if bytes.len() < 2 {
        return FrameParse::Incomplete;
    }
    let first = bytes[0];
    let second = bytes[1];
    let fin = first & 0x80 != 0;
    let compressed = first & 0x40 != 0;
    if first & 0x30 != 0 || second & 0x80 != 0 {
        return FrameParse::Invalid;
    }
    let opcode = first & 0x0f;
    if !matches!(opcode, 0x0 | 0x1 | 0x2 | 0x8 | 0x9 | 0xA) {
        return FrameParse::Invalid;
    }

    let mut index = 2;
    let payload_len = match second & 0x7f {
        length @ 0..=125 => length as usize,
        126 => {
            if bytes.len() < index + 2 {
                return FrameParse::Incomplete;
            }
            let length = u16::from_be_bytes([bytes[index], bytes[index + 1]]) as usize;
            index += 2;
            length
        }
        127 => {
            if bytes.len() < index + 8 || bytes[index] & 0x80 != 0 {
                return FrameParse::Invalid;
            }
            let length = u64::from_be_bytes(bytes[index..index + 8].try_into().unwrap());
            index += 8;
            let Ok(length) = usize::try_from(length) else {
                return FrameParse::Invalid;
            };
            length
        }
        _ => return FrameParse::Invalid,
    };
    if payload_len > MAX_FRAME_PAYLOAD
        || (opcode >= 0x8 && (compressed || !fin || payload_len > 125))
    {
        return FrameParse::Invalid;
    }
    let Some(total) = index.checked_add(payload_len) else {
        return FrameParse::Invalid;
    };
    if bytes.len() < total {
        return FrameParse::Incomplete;
    }
    FrameParse::Complete(
        Frame {
            fin,
            compressed,
            opcode,
            payload: bytes[index..total].to_vec(),
        },
        total,
    )
}

#[derive(Clone, PartialEq, Message)]
struct GateEnvelope {
    #[prost(message, optional, tag = "1")]
    meta: Option<GateMeta>,
    #[prost(bytes = "vec", tag = "2")]
    body: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
struct GateMeta {
    #[prost(string, tag = "1")]
    service_name: String,
    #[prost(string, tag = "2")]
    method_name: String,
    #[prost(int32, tag = "3")]
    message_type: i32,
    #[prost(int64, tag = "6")]
    error_code: i64,
}

#[derive(Clone, PartialEq, Message)]
struct SyncAllReply {
    #[prost(message, repeated, tag = "1")]
    game_friends: Vec<GameFriend>,
}

#[derive(Clone, PartialEq, Message)]
struct GameFriend {
    #[prost(string, tag = "2")]
    open_id: String,
}

fn extract_sync_all_open_ids(bytes: &[u8]) -> Option<Vec<String>> {
    let envelope = GateEnvelope::decode(bytes).ok()?;
    let meta = envelope.meta?;
    if meta.service_name != FRIEND_SERVICE
        || meta.method_name != "SyncAll"
        || meta.message_type != 2
        || meta.error_code != 0
    {
        return None;
    }

    let reply = SyncAllReply::decode(envelope.body.as_slice()).ok()?;
    let mut seen = HashSet::new();
    let mut open_ids = Vec::new();
    for friend in reply.game_friends {
        let open_id = friend.open_id.trim();
        if open_id.is_empty()
            || open_id.len() > MAX_FRIEND_OPEN_ID_LENGTH
            || open_id.chars().any(char::is_control)
        {
            continue;
        }
        if seen.insert(open_id.to_owned()) {
            open_ids.push(open_id.to_owned());
            if open_ids.len() >= MAX_FRIEND_OPEN_IDS {
                break;
            }
        }
    }
    (!open_ids.is_empty()).then_some(open_ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, PartialEq, Message)]
    struct FullSyncAllReply {
        #[prost(message, repeated, tag = "1")]
        game_friends: Vec<FullGameFriend>,
    }

    #[derive(Clone, PartialEq, Message)]
    struct FullGameFriend {
        #[prost(int64, tag = "1")]
        gid: i64,
        #[prost(string, tag = "2")]
        open_id: String,
        #[prost(string, tag = "3")]
        name: String,
    }

    fn sync_all_message(open_ids: &[&str], message_type: i32, error_code: i64) -> Vec<u8> {
        let reply = FullSyncAllReply {
            game_friends: open_ids
                .iter()
                .enumerate()
                .map(|(index, open_id)| FullGameFriend {
                    gid: 10_000 + index as i64,
                    open_id: (*open_id).to_owned(),
                    name: format!("private-name-{index}"),
                })
                .collect(),
        };
        GateEnvelope {
            meta: Some(GateMeta {
                service_name: FRIEND_SERVICE.to_owned(),
                method_name: "SyncAll".to_owned(),
                message_type,
                error_code,
            }),
            body: reply.encode_to_vec(),
        }
        .encode_to_vec()
    }

    fn binary_frame(payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![0x82];
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
        frame
    }

    #[test]
    fn extracts_only_deduplicated_open_ids_from_a_successful_sync_all_reply() {
        let payload = sync_all_message(&["open-a", "open-b", "open-a", ""], 2, 0);
        let frame = binary_frame(&payload);
        let split = frame.len() / 2;
        let mut inspector = FriendSyncInspector::new();

        assert!(inspector.feed(&frame[..split]).is_none());
        assert_eq!(
            inspector.feed(&frame[split..]).unwrap(),
            vec!["open-a".to_owned(), "open-b".to_owned()]
        );
    }

    #[test]
    fn ignores_requests_and_failed_sync_all_replies() {
        assert!(extract_sync_all_open_ids(&sync_all_message(&["open-a"], 1, 0)).is_none());
        assert!(extract_sync_all_open_ids(&sync_all_message(&["open-a"], 2, 1001)).is_none());
    }
}
