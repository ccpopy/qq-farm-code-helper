use crate::certificates::TARGET_HOST;
use rustls::ServerConfig;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, copy_bidirectional},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    time::timeout,
};
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;
use url::Url;

const MAX_HEADER_SIZE: usize = 32 * 1024;
const TARGET_PATH: &str = "/prod/ws";

pub async fn bind(port: u16) -> Result<TcpListener, String> {
    TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|error| format!("无法监听本地端口 {port}: {error}"))
}

pub async fn run(
    listener: TcpListener,
    tls_config: Arc<ServerConfig>,
    cancellation: CancellationToken,
    captured: mpsc::Sender<String>,
) {
    loop {
        let accepted = tokio::select! {
            _ = cancellation.cancelled() => break,
            result = listener.accept() => result,
        };
        let Ok((stream, peer)) = accepted else {
            continue;
        };
        let tls_config = tls_config.clone();
        let captured = captured.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_client(stream, peer, tls_config, captured).await {
                eprintln!("proxy connection closed: {error}");
            }
        });
    }
}

async fn handle_client(
    mut client: TcpStream,
    _peer: SocketAddr,
    tls_config: Arc<ServerConfig>,
    captured: mpsc::Sender<String>,
) -> Result<(), String> {
    let headers = read_headers(&mut client).await?;
    let request = ParsedRequest::parse(&headers)?;
    if request.method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = parse_authority(&request.target)?;
        if host.eq_ignore_ascii_case(TARGET_HOST) && port == 443 {
            return intercept_target(client, tls_config, captured).await;
        }
        return tunnel_connect(client, &host, port).await;
    }
    forward_http(client, headers, request).await
}

async fn intercept_target(
    mut client: TcpStream,
    tls_config: Arc<ServerConfig>,
    captured: mpsc::Sender<String>,
) -> Result<(), String> {
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .map_err(|error| format!("写入 CONNECT 响应失败: {error}"))?;
    let mut tls = TlsAcceptor::from(tls_config)
        .accept(client)
        .await
        .map_err(|error| format!("QQ TLS 握手失败: {error}"))?;
    let headers = read_headers(&mut tls).await?;
    let request = ParsedRequest::parse(&headers)?;
    let code = extract_farm_code(&request.target, &request.host)?;
    let _ = captured.try_send(code);
    write_blocked_response(&mut tls).await
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

async fn read_headers<S>(stream: &mut S) -> Result<Vec<u8>, String>
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
            sender,
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
        assert_eq!(receiver.recv().await.unwrap(), TEST_CODE);
        cancellation.cancel();
        task.await.unwrap();
    }
}
