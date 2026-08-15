use crate::{certificates::TARGET_HOST, local_friend_capture};
use rustls::ServerConfig;
use std::{
    fs::OpenOptions,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
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

#[derive(Clone)]
pub enum ProxyMode {
    CaptureCode {
        captured: mpsc::Sender<CapturedLogin>,
        capture_friend_open_ids: bool,
        diagnostics_path: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedLogin {
    pub code: String,
    pub friend_open_ids: Vec<String>,
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
    mode: ProxyMode,
) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => break,
            accepted = listener.accept() => {
                let Ok((stream, peer)) = accepted else {
                    continue;
                };
                let tls_config = tls_config.clone();
                let mode = mode.clone();
                let connection_cancellation = cancellation.clone();
                connections.spawn(async move {
                    tokio::select! {
                        _ = connection_cancellation.cancelled() => Ok(()),
                        result = handle_client(stream, peer, tls_config, mode) => result,
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
    mode: ProxyMode,
) -> Result<(), String> {
    let headers = read_headers(&mut client).await?;
    let request = ParsedRequest::parse(&headers)?;
    if request.method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = parse_authority(&request.target)?;
        if host.eq_ignore_ascii_case(TARGET_HOST) && port == 443 {
            return intercept_target(client, tls_config, mode).await;
        }
        return tunnel_connect(client, &host, port).await;
    }
    forward_http(client, headers, request).await
}

async fn intercept_target(
    mut client: TcpStream,
    tls_config: Arc<ServerConfig>,
    mode: ProxyMode,
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
        ProxyMode::CaptureCode {
            captured,
            capture_friend_open_ids,
            diagnostics_path,
        } => {
            let request = ParsedRequest::parse(&headers)?;
            log_target_request_diagnostics(&headers, &request, diagnostics_path.as_deref());
            let code = extract_farm_code(&request.target, &request.host)?;
            if !capture_friend_open_ids {
                let _ = captured.try_send(CapturedLogin {
                    code,
                    friend_open_ids: Vec::new(),
                    friend_capture_warning: None,
                });
                return write_blocked_response(&mut tls).await;
            }

            let (friend_open_ids, friend_capture_warning) =
                match local_friend_capture::capture_sync_all_request(&mut tls, &headers).await {
                    Ok(open_ids) => (open_ids, None),
                    Err(error) => (Vec::new(), Some(error)),
                };
            let _ = captured.try_send(CapturedLogin {
                code,
                friend_open_ids,
                friend_capture_warning,
            });
            let _ = tls.shutdown().await;
            Ok(())
        }
    }
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
    S: AsyncWrite + Unpin,
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
        .shutdown()
        .await
        .map_err(|error| format!("关闭连接失败: {error}"))
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
        let (sender, mut receiver) = mpsc::channel(1);
        let task = tokio::spawn(run(
            listener,
            material.server_config,
            cancellation.clone(),
            ProxyMode::CaptureCode {
                captured: sender,
                capture_friend_open_ids: false,
                diagnostics_path: None,
            },
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
        assert_eq!(receiver.recv().await.unwrap().code, TEST_CODE);
        cancellation.cancel();
        task.await.unwrap();
    }
}
