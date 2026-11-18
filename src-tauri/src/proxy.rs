use crate::{
    certificates::{TARGET_HOST, upstream_client_config},
    friend_capture::CapturedLoginIdentity,
    friend_proxy,
    protocol_capture::{self, ProtocolCaptureSession},
    qq_identity::LocalQqIdentity,
};
use rustls::{ClientConfig, ServerConfig};
use std::{
    collections::HashSet,
    fs::OpenOptions,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as StdMutex, RwLock,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, copy_bidirectional},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    task::JoinSet,
    time::timeout,
};
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use url::Url;

const MAX_HEADER_SIZE: usize = 32 * 1024;
const TARGET_PATH: &str = "/prod/ws";

const ARM_ACTIVE: u8 = 0;
const ARM_CLAIMED: u8 = 1;
const ARM_CANCELLED: u8 = 2;

#[derive(Debug)]
pub struct CaptureArm {
    pub identity: LocalQqIdentity,
    state: AtomicU8,
}

impl CaptureArm {
    pub fn new(identity: LocalQqIdentity) -> Arc<Self> {
        Arc::new(Self {
            identity,
            state: AtomicU8::new(ARM_ACTIVE),
        })
    }

    pub fn try_claim(&self) -> bool {
        self.state
            .compare_exchange(ARM_ACTIVE, ARM_CLAIMED, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    pub fn try_cancel(&self) -> bool {
        self.state
            .compare_exchange(
                ARM_ACTIVE,
                ARM_CANCELLED,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
    }

    pub fn is_active(&self) -> bool {
        self.state.load(Ordering::SeqCst) == ARM_ACTIVE
    }

    #[cfg(test)]
    pub fn is_claimed(&self) -> bool {
        self.state.load(Ordering::SeqCst) == ARM_CLAIMED
    }
}

#[derive(Clone)]
pub enum ActiveProxyMode {
    CaptureCode {
        captured: mpsc::Sender<CaptureEvent>,
        /// 只有选中账号后才拦截农场登录；未布防时目标连接按普通隧道透传。
        arm: Option<Arc<CaptureArm>>,
        capture_friend_gids: bool,
        diagnostics_path: Option<PathBuf>,
    },
    ProtocolCapture {
        session: Arc<ProtocolCaptureSession>,
    },
}

pub struct ProxyController {
    mode: RwLock<ActiveProxyMode>,
    claimed_codes: StdMutex<HashSet<String>>,
    #[cfg(test)]
    upstream_override: Option<(SocketAddr, Arc<ClientConfig>)>,
}

impl ProxyController {
    pub fn new(mode: ActiveProxyMode) -> Arc<Self> {
        Arc::new(Self {
            mode: RwLock::new(mode),
            claimed_codes: StdMutex::new(HashSet::new()),
            #[cfg(test)]
            upstream_override: None,
        })
    }

    #[cfg(test)]
    fn new_with_test_upstream(
        mode: ActiveProxyMode,
        addr: SocketAddr,
        upstream_tls: Arc<ClientConfig>,
    ) -> Arc<Self> {
        Arc::new(Self {
            mode: RwLock::new(mode),
            claimed_codes: StdMutex::new(HashSet::new()),
            upstream_override: Some((addr, upstream_tls)),
        })
    }

    pub fn set_mode(&self, mode: ActiveProxyMode) {
        *self
            .mode
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = mode;
    }

    pub fn set_arm(&self, new_arm: Option<Arc<CaptureArm>>) -> Result<(), String> {
        let mut guard = self
            .mode
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match &mut *guard {
            ActiveProxyMode::CaptureCode { arm, .. } => {
                *arm = new_arm;
                Ok(())
            }
            ActiveProxyMode::ProtocolCapture { .. } => Err("协议模式运行中，请先关闭".to_owned()),
        }
    }

    pub fn set_capture_friend_gids(&self, enabled: bool) {
        let mut guard = self
            .mode
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let ActiveProxyMode::CaptureCode {
            capture_friend_gids,
            ..
        } = &mut *guard
        {
            *capture_friend_gids = enabled;
        }
    }

    pub fn interception_mode(&self) -> Option<ActiveProxyMode> {
        let mode = self
            .mode
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match &*mode {
            ActiveProxyMode::CaptureCode { arm: Some(arm), .. } if arm.is_active() => {
                Some(mode.clone())
            }
            ActiveProxyMode::ProtocolCapture { .. } => Some(mode.clone()),
            _ => None,
        }
    }

    pub fn claim_code(&self, arm: &CaptureArm, code: &str) -> bool {
        let mut claimed = self
            .claimed_codes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if claimed.contains(code) {
            return false;
        }
        if !arm.try_claim() {
            return false;
        }
        if claimed.len() >= 64 {
            claimed.clear();
        }
        claimed.insert(code.to_owned());
        true
    }
}

#[derive(Debug)]
pub enum CaptureEvent {
    /// 已从登录请求中取得 Code，本次布防随即失效，好友读取可能仍在进行。
    Intercepted { arm: Arc<CaptureArm> },
    Captured {
        arm: Arc<CaptureArm>,
        login: CapturedLogin,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedLogin {
    pub code: String,
    pub own_identity: Option<CapturedLoginIdentity>,
    pub friend_gids: Vec<String>,
    pub friend_capture_warning: Option<String>,
}

pub async fn bind(port: u16) -> Result<TcpListener, String> {
    TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|error| format!("无法监听本地端口 {port}: {error}"))
}

pub async fn run(
    listener: TcpListener,
    tls_config: Arc<ServerConfig>,
    cancellation: CancellationToken,
    controller: Arc<ProxyController>,
) {
    let upstream_tls = upstream_client_config();
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            accepted = listener.accept() => {
                let Ok((stream, peer)) = accepted else {
                    continue;
                };
                let tls_config = tls_config.clone();
                let controller = controller.clone();
                let upstream_tls = upstream_tls.clone();
                let connection_cancellation = cancellation.clone();
                connections.spawn(async move {
                    tokio::select! {
                        _ = connection_cancellation.cancelled() => Ok(()),
                        result = handle_client(stream, peer, tls_config, controller, upstream_tls) => result,
                    }
                });
            }
            result = connections.join_next(), if !connections.is_empty() => {
                report_connection_result(result);
            }
        }
    }
    cancellation.cancel();
    while let Some(result) = connections.join_next().await {
        report_connection_result(Some(result));
    }
}

fn report_connection_result(result: Option<Result<Result<(), String>, tokio::task::JoinError>>) {
    match result {
        Some(Ok(Err(error))) => eprintln!("proxy connection closed: {error}"),
        Some(Err(error)) if !error.is_cancelled() => eprintln!("proxy task failed: {error}"),
        _ => {}
    }
}

async fn handle_client(
    mut client: TcpStream,
    _peer: SocketAddr,
    tls_config: Arc<ServerConfig>,
    controller: Arc<ProxyController>,
    upstream_tls: Arc<ClientConfig>,
) -> Result<(), String> {
    let headers = read_headers(&mut client).await?;
    let request = ParsedRequest::parse(&headers)?;
    if request.method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = parse_authority(&request.target)?;
        if host.eq_ignore_ascii_case(TARGET_HOST)
            && port == 443
            && let Some(mode) = controller.interception_mode()
        {
            return intercept_target(client, tls_config, controller, upstream_tls, mode).await;
        }
        return tunnel_connect(client, &host, port).await;
    }
    forward_http(client, headers, request).await
}

async fn intercept_target(
    mut client: TcpStream,
    tls_config: Arc<ServerConfig>,
    controller: Arc<ProxyController>,
    upstream_tls: Arc<ClientConfig>,
    mode: ActiveProxyMode,
) -> Result<(), String> {
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .map_err(|error| format!("写入 CONNECT 响应失败: {error}"))?;
    let mut tls = timeout(
        Duration::from_secs(15),
        TlsAcceptor::from(tls_config).accept(client),
    )
    .await
    .map_err(|_| "等待 QQ TLS 握手超时".to_owned())?
    .map_err(|error| format!("QQ TLS 握手失败: {error}"))?;
    let headers = read_headers(&mut tls).await?;
    match mode {
        ActiveProxyMode::CaptureCode {
            captured,
            arm,
            capture_friend_gids,
            diagnostics_path,
        } => {
            let request = ParsedRequest::parse(&headers)?;
            log_target_request_diagnostics(&headers, &request, diagnostics_path.as_deref());
            let code = extract_farm_code(&request.target, &request.host)?;
            // 同一次布防只认领第一条未用过的新 Code；若已取消/重复，则透明转发避免打断正常连接。
            let Some(arm) = arm.filter(|candidate| controller.claim_code(candidate, &code)) else {
                #[cfg(test)]
                if let Some((addr, test_tls)) = &controller.upstream_override {
                    let (mut tls_client, mut upstream) =
                        open_test_upstream(tls, headers, *addr, test_tls.clone()).await?;
                    let _ = copy_bidirectional(&mut tls_client, &mut upstream).await;
                    return Ok(());
                }
                let (mut tls_client, mut upstream) =
                    friend_proxy::open_target_websocket(tls, headers, upstream_tls).await?;
                copy_bidirectional(&mut tls_client, &mut upstream)
                    .await
                    .map_err(|error| format!("透明转发 QQ WebSocket 流量失败: {error}"))?;
                return Ok(());
            };
            let _ = captured.try_send(CaptureEvent::Intercepted { arm: arm.clone() });
            if !capture_friend_gids {
                let _ = captured.try_send(CaptureEvent::Captured {
                    arm,
                    login: CapturedLogin {
                        code,
                        own_identity: None,
                        friend_gids: Vec::new(),
                        friend_capture_warning: None,
                    },
                });
                return write_blocked_response(&mut tls).await;
            }

            let (own_identity, friend_gids, friend_capture_warning) =
                match friend_proxy::relay_target_websocket(tls, headers, upstream_tls).await {
                    Ok(outcome) => (outcome.own_identity, outcome.gids, outcome.warning),
                    Err(error) => (None, Vec::new(), Some(error)),
                };
            let _ = captured.try_send(CaptureEvent::Captured {
                arm,
                login: CapturedLogin {
                    code,
                    own_identity,
                    friend_gids,
                    friend_capture_warning,
                },
            });
            Ok(())
        }
        ActiveProxyMode::ProtocolCapture { session } => {
            #[cfg(test)]
            if let Some((addr, test_tls)) = &controller.upstream_override {
                let handshake_url = friend_proxy::target_websocket_url(&headers)?;
                let connection_id = session.store_handshake_url(&handshake_url)?;
                let (tls_client, upstream) =
                    open_test_upstream(tls, headers, *addr, test_tls.clone()).await?;
                return protocol_capture::relay_opened_websocket(
                    tls_client,
                    upstream,
                    connection_id,
                    session,
                )
                .await;
            }
            protocol_capture::relay_target_websocket(tls, headers, upstream_tls, session).await
        }
    }
}

#[cfg(test)]
async fn open_test_upstream<S>(
    mut client: S,
    request_headers: Vec<u8>,
    addr: SocketAddr,
    upstream_tls: Arc<ClientConfig>,
) -> Result<(S, tokio_rustls::client::TlsStream<TcpStream>), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let stream = TcpStream::connect(addr)
        .await
        .map_err(|error| format!("连接测试上游失败: {error}"))?;
    let server_name = rustls_pki_types::ServerName::try_from(TARGET_HOST.to_owned()).unwrap();
    let mut upstream = tokio_rustls::TlsConnector::from(upstream_tls)
        .connect(server_name, stream)
        .await
        .map_err(|error| format!("测试上游 TLS 握手失败: {error}"))?;
    upstream
        .write_all(&request_headers)
        .await
        .map_err(|error| format!("写入测试上游请求失败: {error}"))?;
    let response_headers = read_headers(&mut upstream).await?;
    client
        .write_all(&response_headers)
        .await
        .map_err(|error| format!("写入测试上游响应失败: {error}"))?;
    Ok((client, upstream))
}

fn log_target_request_diagnostics(headers: &[u8], request: &ParsedRequest, path: Option<&Path>) {
    let query = Url::parse(&format!("https://{TARGET_HOST}{}", request.target))
        .ok()
        .map(|url| {
            url.query_pairs()
                .map(|(name, value)| {
                    let description = if name.eq_ignore_ascii_case("code") {
                        format!("<redacted:{} chars>", value.len())
                    } else if value.bytes().all(|byte| byte.is_ascii_digit()) {
                        format!("<numeric:{} digits>", value.len())
                    } else {
                        format!("<present:{} chars>", value.len())
                    };
                    format!("{name}={description}")
                })
                .collect::<Vec<_>>()
                .join("&")
        })
        .unwrap_or_default();

    let header_descriptions = std::str::from_utf8(headers)
        .ok()
        .map(|text| {
            text.split("\r\n")
                .skip(1)
                .filter_map(|line| line.split_once(':'))
                .filter_map(|(name, value)| {
                    let name = name.trim();
                    if name.is_empty() {
                        return None;
                    }
                    let lower = name.to_ascii_lowercase();
                    let looks_like_identity = [
                        "uin", "qq", "account", "openid", "open-id", "nickname", "avatar",
                    ]
                    .iter()
                    .any(|needle| lower.contains(needle));
                    if looks_like_identity {
                        Some(format!("{name}=<present:{} chars>", value.trim().len()))
                    } else if lower == "cookie" {
                        let names = value
                            .split(';')
                            .filter_map(|item| item.trim().split_once('=').map(|(key, _)| key))
                            .collect::<Vec<_>>()
                            .join("|");
                        Some(format!("Cookie=<keys:{names}>"))
                    } else {
                        Some(name.to_owned())
                    }
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let line = format!(
        "time={timestamp} method={} path={} query=[{}] headers=[{}]\n",
        request.method,
        request.target.split('?').next().unwrap_or_default(),
        query,
        header_descriptions
    );
    eprint!("{line}");
    if let Some(path) = path
        && let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

async fn tunnel_connect(mut client: TcpStream, host: &str, port: u16) -> Result<(), String> {
    let mut upstream = TcpStream::connect((host, port))
        .await
        .map_err(|error| format!("连接 {host}:{port} 失败: {error}"))?;
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .map_err(|error| format!("建立直连隧道失败: {error}"))?;
    copy_bidirectional(&mut client, &mut upstream)
        .await
        .map_err(|error| format!("转发直连流量失败: {error}"))?;
    Ok(())
}

async fn forward_http(
    mut client: TcpStream,
    headers: Vec<u8>,
    request: ParsedRequest,
) -> Result<(), String> {
    let url = Url::parse(&request.target).map_err(|_| "HTTP 代理请求缺少完整 URL".to_owned())?;
    let host = url
        .host_str()
        .ok_or_else(|| "HTTP 请求缺少主机名".to_owned())?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "HTTP 请求端口无效".to_owned())?;
    let mut upstream = TcpStream::connect((host, port))
        .await
        .map_err(|error| format!("连接 {host}:{port} 失败: {error}"))?;
    let rewritten = rewrite_http_request(&headers, &request, &url)?;
    upstream
        .write_all(&rewritten)
        .await
        .map_err(|error| format!("转发 HTTP 请求失败: {error}"))?;
    copy_bidirectional(&mut client, &mut upstream)
        .await
        .map_err(|error| format!("转发 HTTP 流量失败: {error}"))?;
    Ok(())
}

pub(crate) async fn read_headers<S>(stream: &mut S) -> Result<Vec<u8>, String>
where
    S: AsyncRead + Unpin,
{
    timeout(Duration::from_secs(15), async {
        let mut bytes = Vec::with_capacity(1024);
        let mut byte = [0_u8; 1];
        while bytes.len() < MAX_HEADER_SIZE {
            stream
                .read_exact(&mut byte)
                .await
                .map_err(|error| format!("读取请求头失败: {error}"))?;
            bytes.push(byte[0]);
            if bytes.ends_with(b"\r\n\r\n") {
                return Ok(bytes);
            }
        }
        Err("请求头超过安全限制".to_owned())
    })
    .await
    .map_err(|_| "等待请求头超时".to_owned())?
}

async fn write_blocked_response<S>(stream: &mut S) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let body = b"QQ Farm Code captured by local helper.";
    let response = format!(
        "HTTP/1.1 451 Unavailable For Legal Reasons\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|error| format!("写入阻断响应失败: {error}"))?;
    stream
        .write_all(body)
        .await
        .map_err(|error| format!("写入阻断响应失败: {error}"))?;
    stream
        .flush()
        .await
        .map_err(|error| format!("刷新阻断响应失败: {error}"))?;
    // 等待对端按 Content-Length 读完响应并主动关闭，再执行服务端 shutdown，避免 Windows 下触发 10054。
    let _ = timeout(Duration::from_secs(2), async {
        let mut drain = [0_u8; 64];
        while matches!(stream.read(&mut drain).await, Ok(read) if read > 0) {}
    })
    .await;
    let _ = stream.shutdown().await;
    Ok(())
}

#[derive(Debug, Clone)]
struct ParsedRequest {
    method: String,
    target: String,
    version: String,
    host: String,
}

impl ParsedRequest {
    fn parse(headers: &[u8]) -> Result<Self, String> {
        let text = std::str::from_utf8(headers).map_err(|_| "请求头不是有效 UTF-8".to_owned())?;
        let mut lines = text.split("\r\n");
        let request_line = lines.next().ok_or_else(|| "请求行为空".to_owned())?;
        let mut parts = request_line.split_whitespace();
        let method = parts
            .next()
            .ok_or_else(|| "请求方法缺失".to_owned())?
            .to_owned();
        let target = parts
            .next()
            .ok_or_else(|| "请求目标缺失".to_owned())?
            .to_owned();
        let version = parts
            .next()
            .ok_or_else(|| "HTTP 版本缺失".to_owned())?
            .to_owned();
        let host = lines
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("host")
                    .then(|| value.trim().to_owned())
            })
            .unwrap_or_default();
        Ok(Self {
            method,
            target,
            version,
            host,
        })
    }
}

fn parse_authority(authority: &str) -> Result<(String, u16), String> {
    let (host, port) = authority
        .rsplit_once(':')
        .ok_or_else(|| "CONNECT 地址缺少端口".to_owned())?;
    let port = port
        .parse::<u16>()
        .map_err(|_| "CONNECT 端口无效".to_owned())?;
    Ok((host.trim_matches(['[', ']']).to_owned(), port))
}

fn extract_farm_code(target: &str, host_header: &str) -> Result<String, String> {
    let host = host_header.split(':').next().unwrap_or_default();
    if !host.eq_ignore_ascii_case(TARGET_HOST) {
        return Err("请求主机不是 QQ 农场网关".to_owned());
    }
    let url = Url::parse(&format!("https://{TARGET_HOST}{target}"))
        .map_err(|_| "QQ 农场登录 URL 无效".to_owned())?;
    if url.path() != TARGET_PATH {
        return Err("请求不是 QQ 农场登录路径".to_owned());
    }
    let code = url
        .query_pairs()
        .find(|(name, _)| name.eq_ignore_ascii_case("code"))
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| "登录请求中没有 Code".to_owned())?;
    validate_code(&code)?;
    Ok(code)
}

fn validate_code(code: &str) -> Result<(), String> {
    if !(16..=512).contains(&code.len()) {
        return Err("Code 长度无效".to_owned());
    }
    if !code
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("Code 包含异常字符".to_owned());
    }
    Ok(())
}

fn rewrite_http_request(
    headers: &[u8],
    request: &ParsedRequest,
    url: &Url,
) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(headers).map_err(|_| "HTTP 请求头编码无效".to_owned())?;
    let (_, remainder) = text
        .split_once("\r\n")
        .ok_or_else(|| "HTTP 请求头不完整".to_owned())?;
    let mut path = url.path().to_owned();
    if path.is_empty() {
        path.push('/');
    }
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    Ok(format!(
        "{} {} {}\r\n{}",
        request.method, path, request.version, remainder
    )
    .into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::certificates::generate_tls_material;

    const TEST_CODE: &str = "testcode0123456789abcdef0123456789";

    fn test_identity(qq_number: &str, nickname: &str) -> LocalQqIdentity {
        LocalQqIdentity {
            qq_number: qq_number.to_owned(),
            nickname: nickname.to_owned(),
            avatar_url: String::new(),
            source: "test",
            verification_detail: "test",
        }
    }

    fn test_client_config(ca_der: &[u8]) -> Arc<ClientConfig> {
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(rustls_pki_types::CertificateDer::from(ca_der.to_vec()))
            .unwrap();
        Arc::new(
            ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    }

    async fn connect_via_proxy(
        proxy_port: u16,
        client_tls: Arc<ClientConfig>,
    ) -> tokio_rustls::client::TlsStream<TcpStream> {
        let mut stream = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
        let connect_req =
            format!("CONNECT {TARGET_HOST}:443 HTTP/1.1\r\nHost: {TARGET_HOST}:443\r\n\r\n");
        stream.write_all(connect_req.as_bytes()).await.unwrap();
        let response = read_headers(&mut stream).await.unwrap();
        assert!(std::str::from_utf8(&response).unwrap().contains("200"));
        let server_name = rustls_pki_types::ServerName::try_from(TARGET_HOST.to_owned()).unwrap();
        tokio_rustls::TlsConnector::from(client_tls)
            .connect(server_name, stream)
            .await
            .unwrap()
    }

    fn masked_binary_frame(payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() <= 125);
        let mask = [0x11, 0x22, 0x33, 0x44];
        let mut frame = vec![0x82, 0x80 | payload.len() as u8];
        frame.extend_from_slice(&mask);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(index, byte)| byte ^ mask[index % mask.len()]),
        );
        frame
    }

    fn unmasked_binary_frame(payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() <= 125);
        let mut frame = vec![0x82, payload.len() as u8];
        frame.extend_from_slice(payload);
        frame
    }

    #[test]
    fn extracts_expected_code() {
        let code = extract_farm_code(
            &format!("/prod/ws?platform=qq&code={TEST_CODE}"),
            TARGET_HOST,
        )
        .unwrap();
        assert_eq!(code, TEST_CODE);
    }

    #[test]
    fn rejects_other_hosts() {
        let result = extract_farm_code("/prod/ws?code=852294d2c176d091", "example.com");
        assert!(result.is_err());
    }

    #[test]
    fn rewrites_absolute_http_target() {
        let headers = b"GET http://example.com/a?q=1 HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let request = ParsedRequest::parse(headers).unwrap();
        let url = Url::parse(&request.target).unwrap();
        let rewritten = rewrite_http_request(headers, &request, &url).unwrap();
        assert!(
            String::from_utf8(rewritten)
                .unwrap()
                .starts_with("GET /a?q=1 HTTP/1.1")
        );
    }

    #[tokio::test]
    async fn intercepts_target_before_upstream_connection() {
        let material = generate_tls_material().unwrap();
        let listener = bind(0).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let cancellation = CancellationToken::new();
        let (sender, mut receiver) = mpsc::channel(4);
        let arm = CaptureArm::new(test_identity("12345678", "测试账号"));
        let controller = ProxyController::new(ActiveProxyMode::CaptureCode {
            captured: sender,
            arm: Some(arm.clone()),
            capture_friend_gids: false,
            diagnostics_path: None,
        });
        let task = tokio::spawn(run(
            listener,
            material.server_config,
            cancellation.clone(),
            controller,
        ));

        let client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}")).unwrap())
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap();
        let response = client
            .get(format!(
                "https://{TARGET_HOST}/prod/ws?platform=qq&code={TEST_CODE}"
            ))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status().as_u16(), 451);
        assert!(matches!(
            receiver.recv().await.unwrap(),
            CaptureEvent::Intercepted { .. }
        ));
        let CaptureEvent::Captured {
            arm: got_arm,
            login,
        } = receiver.recv().await.unwrap()
        else {
            panic!("expected captured login");
        };
        assert_eq!(got_arm.identity.qq_number, "12345678");
        assert_eq!(login.code, TEST_CODE);
        assert!(arm.is_claimed());
        assert!(!arm.is_active());
        cancellation.cancel();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn switches_modes_live_and_ignores_duplicate_codes_on_same_listener() {
        let proxy_material = generate_tls_material().unwrap();
        let upstream_material = generate_tls_material().unwrap();
        let proxy_client_tls = test_client_config(&proxy_material.ca_der);
        let upstream_client_tls = test_client_config(&upstream_material.ca_der);

        let upstream_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let upstream_acceptor = TlsAcceptor::from(upstream_material.server_config);
        let upstream_cancel = CancellationToken::new();
        let upstream_task_cancel = upstream_cancel.clone();
        let upstream_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = upstream_task_cancel.cancelled() => break,
                    accepted = upstream_listener.accept() => {
                        let Ok((socket, _)) = accepted else {
                            break;
                        };
                        let acceptor = upstream_acceptor.clone();
                        tokio::spawn(async move {
                            let mut tls = acceptor.accept(socket).await.unwrap();
                            let headers = read_headers(&mut tls).await.unwrap();
                            let is_proto_conn = String::from_utf8_lossy(&headers)
                                .contains("code=syntheticproto_");
                            tls.write_all(
                                b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: test\r\n\r\n",
                            )
                            .await
                            .unwrap();
                            if is_proto_conn {
                                // Read client binary frame (`0x82`, len+mask, 4-byte mask, payload) before sending reply
                                let mut buf = [0_u8; 64];
                                let _ = tls.read(&mut buf).await.unwrap();
                                tls.write_all(&unmasked_binary_frame(b"hello-client"))
                                    .await
                                    .unwrap();
                            }
                            let _ = tls.flush().await;
                            let mut drain = [0_u8; 32];
                            let _ = timeout(Duration::from_millis(200), tls.read(&mut drain)).await;
                            let _ = tls.shutdown().await;
                        });
                    }
                }
            }
        });

        let listener = bind(0).await.unwrap();
        let proxy_port = listener.local_addr().unwrap().port();
        let cancellation = CancellationToken::new();
        let (sender, mut receiver) = mpsc::channel(8);
        let arm_a = CaptureArm::new(test_identity("100001", "账号A"));
        let controller = ProxyController::new_with_test_upstream(
            ActiveProxyMode::CaptureCode {
                captured: sender.clone(),
                arm: Some(arm_a.clone()),
                capture_friend_gids: false,
                diagnostics_path: None,
            },
            upstream_addr,
            upstream_client_tls,
        );
        let proxy_task = tokio::spawn(run(
            listener,
            proxy_material.server_config,
            cancellation.clone(),
            controller.clone(),
        ));

        // 阶段 A: CaptureCode 捕获 CODE_A
        let code_a = "syntheticcodeA_0123456789abcdef0123";
        let mut late_a = connect_via_proxy(proxy_port, proxy_client_tls.clone()).await;
        let mut client_a = connect_via_proxy(proxy_port, proxy_client_tls.clone()).await;
        let req_a = format!(
            "GET /prod/ws?platform=qq&code={code_a} HTTP/1.1\r\nHost: {TARGET_HOST}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
        );
        client_a.write_all(req_a.as_bytes()).await.unwrap();
        let resp_a = read_headers(&mut client_a).await.unwrap();
        assert!(
            std::str::from_utf8(&resp_a)
                .unwrap()
                .starts_with("HTTP/1.1 451")
        );
        let _ = client_a.shutdown().await;
        assert!(matches!(
            receiver.recv().await.unwrap(),
            CaptureEvent::Intercepted { .. }
        ));
        let CaptureEvent::Captured {
            arm: got_a,
            login: login_a,
        } = receiver.recv().await.unwrap()
        else {
            panic!("expected captured login for A");
        };
        assert_eq!(got_a.identity.qq_number, "100001");
        assert_eq!(login_a.code, code_a);

        // 阶段 B: 切换到账号 B 后重放同一 CODE_A，应透明转发而不消耗 arm_b
        let arm_b = CaptureArm::new(test_identity("100002", "账号B"));
        controller.set_arm(Some(arm_b.clone())).unwrap();
        let mut dup_tls = connect_via_proxy(proxy_port, proxy_client_tls.clone()).await;
        let dup_req = format!(
            "GET /prod/ws?platform=qq&code={code_a} HTTP/1.1\r\nHost: {TARGET_HOST}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
        );
        dup_tls.write_all(dup_req.as_bytes()).await.unwrap();
        let dup_resp = read_headers(&mut dup_tls).await.unwrap();
        assert!(std::str::from_utf8(&dup_resp).unwrap().contains("101"));
        let _ = dup_tls.shutdown().await;
        assert!(arm_b.is_active());
        assert!(receiver.try_recv().is_err());

        // 切换前已建立的 TLS 连接仍归原来的账号。
        let late_req = req_a.replace(code_a, "syntheticlateA_0123456789abcdef0123");
        late_a.write_all(late_req.as_bytes()).await.unwrap();
        let late_resp = read_headers(&mut late_a).await.unwrap();
        assert!(std::str::from_utf8(&late_resp).unwrap().contains("101"));
        let _ = late_a.shutdown().await;
        assert!(arm_b.is_active());
        assert!(receiver.try_recv().is_err());

        // 阶段 C: 同一监听端口热切换至 ProtocolCapture
        let capture_root = std::env::temp_dir().join(format!(
            "qq-farm-proxy-hotswitch-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let session = ProtocolCaptureSession::create(&capture_root).unwrap();
        controller.set_mode(ActiveProxyMode::ProtocolCapture {
            session: session.clone(),
        });
        let proto_code = "syntheticproto_0123456789abcdef0123";
        let mut proto_tls = connect_via_proxy(proxy_port, proxy_client_tls.clone()).await;
        let proto_req = format!(
            "GET /prod/ws?platform=qq&code={proto_code} HTTP/1.1\r\nHost: {TARGET_HOST}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
        );
        proto_tls.write_all(proto_req.as_bytes()).await.unwrap();
        let proto_resp = read_headers(&mut proto_tls).await.unwrap();
        assert!(std::str::from_utf8(&proto_resp).unwrap().contains("101"));
        proto_tls
            .write_all(&masked_binary_frame(b"hello-gate"))
            .await
            .unwrap();
        proto_tls.flush().await.unwrap();
        let mut server_reply = [0_u8; 14];
        proto_tls.read_exact(&mut server_reply).await.unwrap();
        assert_eq!(
            server_reply.as_slice(),
            unmasked_binary_frame(b"hello-client").as_slice()
        );
        let _ = proto_tls.shutdown().await;
        for _ in 0..40 {
            if session.summary().message_count >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        let summary = session.summary();
        assert_eq!(summary.connection_count, 1);
        assert_eq!(summary.message_count, 2);
        let handshake = std::fs::read_to_string(
            summary
                .directory
                .join("handshakes/connection-000001-url.txt"),
        )
        .unwrap();
        assert!(handshake.contains(proto_code));
        assert_eq!(
            std::fs::read(summary.directory.join("000001-send.bin")).unwrap(),
            b"hello-gate"
        );
        assert_eq!(
            std::fs::read(summary.directory.join("000002-recv.bin")).unwrap(),
            b"hello-client"
        );

        // 阶段 D: 再热切回 CaptureCode，正常捕获账号 B 的新 CODE_B
        let (sender_b, mut receiver_b) = mpsc::channel(4);
        controller.set_mode(ActiveProxyMode::CaptureCode {
            captured: sender_b,
            arm: Some(arm_b.clone()),
            capture_friend_gids: false,
            diagnostics_path: None,
        });
        let code_b = "syntheticcodeB_0123456789abcdef0123";
        let mut client_b = connect_via_proxy(proxy_port, proxy_client_tls).await;
        let req_b = format!(
            "GET /prod/ws?platform=qq&code={code_b} HTTP/1.1\r\nHost: {TARGET_HOST}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
        );
        client_b.write_all(req_b.as_bytes()).await.unwrap();
        let resp_b = read_headers(&mut client_b).await.unwrap();
        assert!(
            std::str::from_utf8(&resp_b)
                .unwrap()
                .starts_with("HTTP/1.1 451")
        );
        let _ = client_b.shutdown().await;
        assert!(matches!(
            receiver_b.recv().await.unwrap(),
            CaptureEvent::Intercepted { .. }
        ));
        let CaptureEvent::Captured {
            arm: got_b,
            login: login_b,
        } = receiver_b.recv().await.unwrap()
        else {
            panic!("expected captured login for B");
        };
        assert_eq!(got_b.identity.qq_number, "100002");
        assert_eq!(login_b.code, code_b);
        assert!(arm_b.is_claimed());

        cancellation.cancel();
        proxy_task.await.unwrap();
        upstream_cancel.cancel();
        upstream_task.await.unwrap();
        drop(controller);
        drop(session);
        let _ = std::fs::remove_dir_all(capture_root);
    }
}
