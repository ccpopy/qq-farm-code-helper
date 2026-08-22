use prost::Message;
use std::collections::{HashMap, HashSet};

const MAX_FRAME_PAYLOAD: usize = 8 * 1024 * 1024;
const MAX_BUFFERED_BYTES: usize = MAX_FRAME_PAYLOAD + 16 * 1024 + 32;
const MAX_FRIEND_GIDS: usize = 500;
const FRIEND_SERVICE: &str = "gamepb.friendpb.FriendService";
const USER_SERVICE: &str = "gamepb.userpb.UserService";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FriendReplyKind {
    SyncAll,
    GetGameFriends,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapturedFriendReply {
    pub kind: FriendReplyKind,
    pub client_seq: i64,
    pub gids: Vec<String>,
}

#[derive(Debug, Default)]
pub(crate) struct ServerInspection {
    pub latest_server_seq: i64,
    pub own_gid: Option<String>,
    pub friend_replies: Vec<CapturedFriendReply>,
}

pub(crate) struct ServerFriendInspector {
    buffer: Vec<u8>,
    fragmented: Option<FragmentedMessage>,
    enabled: bool,
}

enum FragmentedMessage {
    Binary(Vec<u8>),
    Text,
}

impl ServerFriendInspector {
    pub(crate) fn new() -> Self {
        Self {
            buffer: Vec::new(),
            fragmented: None,
            enabled: true,
        }
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) -> ServerInspection {
        let mut inspection = ServerInspection::default();
        if !self.enabled || bytes.is_empty() {
            return inspection;
        }
        if self.buffer.len().saturating_add(bytes.len()) > MAX_BUFFERED_BYTES {
            self.disable();
            return inspection;
        }
        self.buffer.extend_from_slice(bytes);

        let mut consumed = 0;
        loop {
            match parse_server_frame(&self.buffer[consumed..]) {
                FrameParse::Incomplete => break,
                FrameParse::Invalid => {
                    self.disable();
                    break;
                }
                FrameParse::Complete(frame, frame_bytes) => {
                    consumed += frame_bytes;
                    if let Some(message) = self.process_frame(frame) {
                        inspection.latest_server_seq =
                            inspection.latest_server_seq.max(message.server_seq.max(0));
                        if inspection.own_gid.is_none() {
                            inspection.own_gid = message.own_gid;
                        }
                        if let Some(friend_reply) = message.friend_reply {
                            inspection.friend_replies.push(friend_reply);
                        }
                    }
                    if !self.enabled {
                        break;
                    }
                }
            }
        }
        if self.enabled && consumed > 0 {
            self.buffer.drain(..consumed);
        }
        inspection
    }

    fn process_frame(&mut self, frame: Frame) -> Option<MessageInspection> {
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

    fn process_data_start(&mut self, frame: Frame, binary: bool) -> Option<MessageInspection> {
        if self.fragmented.is_some() || frame.compressed {
            self.disable();
            return None;
        }
        if frame.fin {
            return binary.then(|| inspect_server_message(&frame.payload));
        }
        self.fragmented = Some(if binary {
            FragmentedMessage::Binary(frame.payload)
        } else {
            FragmentedMessage::Text
        });
        None
    }

    fn process_continuation(&mut self, frame: Frame) -> Option<MessageInspection> {
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
            Some(FragmentedMessage::Binary(message)) => Some(inspect_server_message(&message)),
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
    #[prost(string, tag = "3")]
    token: String,
}

#[derive(Clone, PartialEq, Message)]
struct GateMeta {
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
struct FriendListReply {
    #[prost(message, repeated, tag = "1")]
    game_friends: Vec<GameFriend>,
}

#[derive(Clone, PartialEq, Message)]
struct GameFriend {
    #[prost(int64, tag = "1")]
    gid: i64,
}

#[derive(Clone, PartialEq, Message)]
struct SyncAllRequest {
    #[prost(string, repeated, tag = "2")]
    open_ids: Vec<String>,
}

#[derive(Clone, PartialEq, Message)]
struct LoginReply {
    #[prost(message, optional, tag = "1")]
    basic: Option<BasicInfo>,
}

#[derive(Clone, PartialEq, Message)]
struct BasicInfo {
    #[prost(int64, tag = "1")]
    gid: i64,
}

struct MessageInspection {
    server_seq: i64,
    own_gid: Option<String>,
    friend_reply: Option<CapturedFriendReply>,
}

fn inspect_server_message(bytes: &[u8]) -> MessageInspection {
    let Ok(envelope) = GateEnvelope::decode(bytes) else {
        return MessageInspection {
            server_seq: 0,
            own_gid: None,
            friend_reply: None,
        };
    };
    let Some(meta) = envelope.meta else {
        return MessageInspection {
            server_seq: 0,
            own_gid: None,
            friend_reply: None,
        };
    };
    let own_gid = extract_own_gid(&meta, &envelope.body);
    let friend_reply = extract_friend_reply(&meta, &envelope.body);
    MessageInspection {
        server_seq: meta.server_seq,
        own_gid,
        friend_reply,
    }
}

fn extract_own_gid(meta: &GateMeta, body: &[u8]) -> Option<String> {
    if meta.service_name != USER_SERVICE
        || meta.method_name != "Login"
        || meta.message_type != 2
        || meta.error_code != 0
    {
        return None;
    }
    let gid = LoginReply::decode(body).ok()?.basic?.gid;
    (gid > 0).then(|| gid.to_string())
}

fn extract_friend_reply(meta: &GateMeta, body: &[u8]) -> Option<CapturedFriendReply> {
    if meta.service_name != FRIEND_SERVICE || meta.message_type != 2 || meta.error_code != 0 {
        return None;
    }
    let kind = match meta.method_name.as_str() {
        "SyncAll" => FriendReplyKind::SyncAll,
        "GetGameFriends" => FriendReplyKind::GetGameFriends,
        _ => return None,
    };
    let reply = FriendListReply::decode(body).ok()?;
    let mut seen = HashSet::new();
    let mut gids = Vec::new();
    for friend in reply.game_friends {
        if friend.gid <= 0 || !seen.insert(friend.gid) {
            continue;
        }
        gids.push(friend.gid.to_string());
        if gids.len() >= MAX_FRIEND_GIDS {
            break;
        }
    }
    Some(CapturedFriendReply {
        kind,
        client_seq: meta.client_seq,
        gids,
    })
}

pub(crate) fn encode_empty_sync_all_request(
    client_seq: i64,
    server_seq: i64,
    token: String,
) -> Vec<u8> {
    GateEnvelope {
        meta: Some(GateMeta {
            service_name: FRIEND_SERVICE.to_owned(),
            method_name: "SyncAll".to_owned(),
            message_type: 1,
            client_seq,
            server_seq,
            error_code: 0,
            error_message: String::new(),
            metadata: HashMap::new(),
        }),
        body: SyncAllRequest {
            open_ids: Vec::new(),
        }
        .encode_to_vec(),
        token,
    }
    .encode_to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, PartialEq, Message)]
    struct FullFriendListReply {
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

    #[derive(Clone, PartialEq, Message)]
    struct FullLoginReply {
        #[prost(message, optional, tag = "1")]
        basic: Option<FullBasicInfo>,
        #[prost(int64, tag = "3")]
        time_now_millis: i64,
    }

    #[derive(Clone, PartialEq, Message)]
    struct FullBasicInfo {
        #[prost(int64, tag = "1")]
        gid: i64,
        #[prost(string, tag = "2")]
        name: String,
        #[prost(string, tag = "6")]
        open_id: String,
    }

    fn friend_message(method_name: &str, gids: &[i64], error_code: i64) -> Vec<u8> {
        let reply = FullFriendListReply {
            game_friends: gids
                .iter()
                .enumerate()
                .map(|(index, gid)| FullGameFriend {
                    gid: *gid,
                    open_id: format!("private-open-id-{index}"),
                    name: format!("private-name-{index}"),
                })
                .collect(),
        };
        GateEnvelope {
            meta: Some(GateMeta {
                service_name: FRIEND_SERVICE.to_owned(),
                method_name: method_name.to_owned(),
                message_type: 2,
                client_seq: 17,
                server_seq: 29,
                error_code,
                error_message: String::new(),
                metadata: HashMap::new(),
            }),
            body: reply.encode_to_vec(),
            token: String::new(),
        }
        .encode_to_vec()
    }

    fn login_message(gid: i64, error_code: i64) -> Vec<u8> {
        let reply = FullLoginReply {
            basic: Some(FullBasicInfo {
                gid,
                name: "测试农夫".to_owned(),
                open_id: "private-open-id".to_owned(),
            }),
            time_now_millis: 1_700_000_000_000,
        };
        GateEnvelope {
            meta: Some(GateMeta {
                service_name: USER_SERVICE.to_owned(),
                method_name: "Login".to_owned(),
                message_type: 2,
                client_seq: 11,
                server_seq: 13,
                error_code,
                error_message: String::new(),
                metadata: HashMap::new(),
            }),
            body: reply.encode_to_vec(),
            token: String::new(),
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
    fn extracts_only_deduplicated_positive_gids_from_sync_all() {
        let payload = friend_message("SyncAll", &[10_001, 10_002, 10_001, 0, -1], 0);
        let frame = binary_frame(&payload);
        let split = frame.len() / 2;
        let mut inspector = ServerFriendInspector::new();

        assert!(inspector.feed(&frame[..split]).friend_replies.is_empty());
        let inspection = inspector.feed(&frame[split..]);
        assert_eq!(inspection.latest_server_seq, 29);
        assert_eq!(
            inspection.friend_replies,
            vec![CapturedFriendReply {
                kind: FriendReplyKind::SyncAll,
                client_seq: 17,
                gids: vec!["10001".to_owned(), "10002".to_owned()],
            }]
        );
    }

    #[test]
    fn extracts_own_gid_from_the_successful_login_reply() {
        let frame = binary_frame(&login_message(1_027_000_001, 0));
        let split = frame.len() / 2;
        let mut inspector = ServerFriendInspector::new();

        assert_eq!(inspector.feed(&frame[..split]).own_gid, None);
        let inspection = inspector.feed(&frame[split..]);

        assert_eq!(inspection.own_gid.as_deref(), Some("1027000001"));
        assert_eq!(inspection.latest_server_seq, 13);
        assert!(inspection.friend_replies.is_empty());
    }

    #[test]
    fn ignores_own_gid_from_failed_or_invalid_login_replies() {
        assert_eq!(
            inspect_server_message(&login_message(1_027_000_001, 1001)).own_gid,
            None
        );
        assert_eq!(inspect_server_message(&login_message(0, 0)).own_gid, None);
    }

    #[test]
    fn recognizes_get_game_friends_as_a_fallback_source() {
        let message = inspect_server_message(&friend_message("GetGameFriends", &[20_001], 0));
        let reply = message.friend_reply.unwrap();

        assert_eq!(reply.kind, FriendReplyKind::GetGameFriends);
        assert_eq!(reply.gids, vec!["20001"]);
    }

    #[test]
    fn ignores_failed_friend_replies() {
        let message = inspect_server_message(&friend_message("SyncAll", &[10_001], 1001));
        assert!(message.friend_reply.is_none());
    }

    #[test]
    fn builds_an_empty_sync_all_request_with_supplied_sequences() {
        let bytes = encode_empty_sync_all_request(41, 73, "random-token=".to_owned());
        let envelope = GateEnvelope::decode(bytes.as_slice()).unwrap();
        let meta = envelope.meta.unwrap();

        assert_eq!(meta.service_name, FRIEND_SERVICE);
        assert_eq!(meta.method_name, "SyncAll");
        assert_eq!(meta.message_type, 1);
        assert_eq!(meta.client_seq, 41);
        assert_eq!(meta.server_seq, 73);
        assert_eq!(envelope.token, "random-token=");
        assert!(
            SyncAllRequest::decode(envelope.body.as_slice())
                .unwrap()
                .open_ids
                .is_empty()
        );
    }
}
