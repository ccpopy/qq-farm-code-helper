use prost::Message;
use std::collections::{HashMap, HashSet};

const MAX_FRIEND_OPEN_IDS: usize = 200;
const MAX_FRIEND_OPEN_ID_LENGTH: usize = 128;
const FRIEND_SERVICE: &str = "gamepb.friendpb.FriendService";

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
struct SyncAllRequest {
    #[prost(string, repeated, tag = "2")]
    open_ids: Vec<String>,
}

pub(crate) struct ClientGateRequest {
    pub response: Vec<u8>,
    pub sync_open_ids: Option<Result<Vec<String>, String>>,
}

pub(crate) fn inspect_client_gate_request(bytes: &[u8]) -> Option<ClientGateRequest> {
    let mut envelope = GateEnvelope::decode(bytes).ok()?;
    let mut meta = envelope.meta.take()?;
    if meta.message_type != 1 || meta.service_name.is_empty() || meta.method_name.is_empty() {
        return None;
    }

    let sync_open_ids = if meta.service_name == FRIEND_SERVICE && meta.method_name == "SyncAll" {
        Some(
            SyncAllRequest::decode(envelope.body.as_slice())
                .map_err(|_| "QQ 客户端 SyncAll 请求体无法解析，协议可能已更新".to_owned())
                .and_then(|request| {
                    normalize_open_ids(request.open_ids)
                        .ok_or_else(|| "QQ 客户端 SyncAll 请求未包含有效好友标识".to_owned())
                }),
        )
    } else {
        None
    };

    meta.message_type = 2;
    meta.server_seq = meta.server_seq.saturating_add(1).max(1);
    meta.error_code = 0;
    meta.error_message.clear();
    meta.metadata.clear();
    envelope.meta = Some(meta);
    envelope.body.clear();
    envelope.token.clear();

    Some(ClientGateRequest {
        response: envelope.encode_to_vec(),
        sync_open_ids,
    })
}

fn normalize_open_ids(values: impl IntoIterator<Item = String>) -> Option<Vec<String>> {
    let mut seen = HashSet::new();
    let mut open_ids = Vec::new();
    for value in values {
        let open_id = value.trim();
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

    fn gate_request(service_name: &str, method_name: &str, body: Vec<u8>) -> Vec<u8> {
        GateEnvelope {
            meta: Some(GateMeta {
                service_name: service_name.to_owned(),
                method_name: method_name.to_owned(),
                message_type: 1,
                client_seq: 7,
                server_seq: 12,
                error_code: 0,
                error_message: String::new(),
                metadata: HashMap::from([("trace".to_owned(), vec![1, 2, 3])]),
            }),
            body,
            token: "request-token".to_owned(),
        }
        .encode_to_vec()
    }

    #[test]
    fn extracts_deduplicated_open_ids_and_builds_a_correlated_success_response() {
        let body = SyncAllRequest {
            open_ids: vec![
                " open-a ".to_owned(),
                "open-b".to_owned(),
                "open-a".to_owned(),
                String::new(),
            ],
        }
        .encode_to_vec();

        let inspected =
            inspect_client_gate_request(&gate_request(FRIEND_SERVICE, "SyncAll", body)).unwrap();

        assert_eq!(
            inspected.sync_open_ids.unwrap().unwrap(),
            vec!["open-a".to_owned(), "open-b".to_owned()]
        );
        let response = GateEnvelope::decode(inspected.response.as_slice()).unwrap();
        let meta = response.meta.unwrap();
        assert_eq!(meta.service_name, FRIEND_SERVICE);
        assert_eq!(meta.method_name, "SyncAll");
        assert_eq!(meta.message_type, 2);
        assert_eq!(meta.client_seq, 7);
        assert_eq!(meta.server_seq, 13);
        assert_eq!(meta.error_code, 0);
        assert!(meta.error_message.is_empty());
        assert!(meta.metadata.is_empty());
        assert!(response.body.is_empty());
        assert!(response.token.is_empty());
    }

    #[test]
    fn generic_requests_receive_an_empty_success_response_without_friend_data() {
        let inspected = inspect_client_gate_request(&gate_request(
            "gamepb.qqvippb.QQVipService",
            "GetQQVipRewardsStatus",
            Vec::new(),
        ))
        .unwrap();

        assert!(inspected.sync_open_ids.is_none());
        let response = GateEnvelope::decode(inspected.response.as_slice()).unwrap();
        assert_eq!(response.meta.unwrap().message_type, 2);
        assert!(response.body.is_empty());
    }

    #[test]
    fn ignores_non_request_gate_messages() {
        let mut request =
            GateEnvelope::decode(gate_request(FRIEND_SERVICE, "SyncAll", Vec::new()).as_slice())
                .unwrap();
        request.meta.as_mut().unwrap().message_type = 2;

        assert!(inspect_client_gate_request(&request.encode_to_vec()).is_none());
    }

    #[test]
    fn reports_a_sync_all_body_that_cannot_be_decoded() {
        let inspected =
            inspect_client_gate_request(&gate_request(FRIEND_SERVICE, "SyncAll", vec![0xFF, 0xFF]))
                .unwrap();

        assert!(
            inspected
                .sync_open_ids
                .unwrap()
                .unwrap_err()
                .contains("无法解析")
        );
    }
}
