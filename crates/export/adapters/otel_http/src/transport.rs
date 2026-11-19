//! HTTP connections, response framing, and OTLP response classification.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use crate::config::{Endpoint, OtelHttpTlsConfig};
use crate::live_http::OTEL_HTTP_EXPORTER_NAME;

/// Classification of a failed POST, per the OTLP/HTTP spec.
#[derive(Debug)]
pub(super) enum PostError {
    /// Transport error or a retryable status (408/429/500/502/503/504).
    /// `retry_after` carries a server-supplied delay when present.
    Retryable {
        detail: String,
        retry_after: Option<Duration>,
    },
    /// The collector rejected the body as too large (413). Replaying it is
    /// pointless, but the same spans fit in smaller batches.
    TooLarge { detail: String },
    /// A status that replay cannot fix (400, 401, 403, 404, 415, ...). Drop now.
    Permanent { detail: String },
}

#[derive(Debug)]
pub(super) struct PostSuccess {
    pub(super) partial_rejected: u64,
}

/// The collector's body-size limit is below this batch's size.
const HTTP_PAYLOAD_TOO_LARGE: u16 = 413;

/// Retryable status codes. Everything else non-2xx is permanent.
///
/// 429/502/503/504 are the OTLP specification's retryable set. 408 and 500 are
/// added on top: both describe the collector failing to process a request it
/// otherwise accepted, so a replay is the only thing that can recover them, and
/// `retry_max_attempts` bounds the cost when a collector returns them forever.
fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
}

/// Parse a `Retry-After` header value in delay-seconds form (the HTTP-date form
/// is not emitted by OTLP collectors and is intentionally not handled). The
/// input is the raw header block (everything before the body).
fn parse_retry_after(header_block: &str) -> Option<Duration> {
    header_block.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if !name.trim().eq_ignore_ascii_case("retry-after") {
            return None;
        }
        value.trim().parse::<u64>().ok().map(Duration::from_secs)
    })
}

pub(super) enum HttpConnection {
    Plain(TcpStream),
    Tls(openssl::ssl::SslStream<TcpStream>),
}

impl Read for HttpConnection {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for HttpConnection {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buffer),
            Self::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

/// Render the configured headers into the `name: value\r\n` block the request
/// head splices in. Names and values were validated at config parse time, so
/// this cannot introduce a header the transport does not expect.
pub(super) fn render_extra_headers(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect()
}

pub(super) struct PostRequest<'a> {
    pub(super) endpoint: &'a Endpoint,
    pub(super) body: &'a [u8],
    pub(super) content_type: &'a str,
    pub(super) content_encoding: Option<&'a str>,
    pub(super) extra_headers: &'a str,
    pub(super) connect_timeout: Duration,
    pub(super) request_timeout: Duration,
    pub(super) tls: &'a OtelHttpTlsConfig,
}

pub(super) fn post_otlp_reusing(
    connection: &mut Option<HttpConnection>,
    request: PostRequest<'_>,
) -> Result<PostSuccess, PostError> {
    // Transport-level failures (DNS, connect, TLS, socket I/O) are transient:
    // classify them all as retryable so a blip degrades to a retry, not a drop.
    let retryable = |detail: String| PostError::Retryable {
        detail,
        retry_after: None,
    };
    if connection.is_none() {
        *connection = Some(
            open_connection(
                request.endpoint,
                request.connect_timeout,
                request.request_timeout,
                request.tls,
            )
            .map_err(retryable)?,
        );
    }
    let response = match http_exchange(
        connection.as_mut().expect("connection initialized"),
        request.endpoint,
        request.body,
        request.content_type,
        request.content_encoding,
        request.extra_headers,
    ) {
        Ok(response) => response,
        Err(error) => {
            connection.take();
            return Err(retryable(error));
        }
    };
    if response.connection_close {
        connection.take();
    }
    classify_response(request.endpoint, response)
}

fn open_connection(
    endpoint: &Endpoint,
    connect_timeout: Duration,
    request_timeout: Duration,
    tls: &OtelHttpTlsConfig,
) -> Result<HttpConnection, String> {
    let addresses = endpoint
        .authority()
        .to_socket_addrs()
        .map_err(|error| format!("resolve {}: {error}", endpoint.authority()))?;
    let stream = connect_resolved_addresses(addresses, connect_timeout)
        .map_err(|error| format!("connect {}: {error}", endpoint.authority()))?;
    stream
        .set_write_timeout(Some(request_timeout))
        .map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(request_timeout))
        .map_err(|error| error.to_string())?;
    if endpoint.secure {
        tls_connect(&endpoint.host, stream, tls).map(HttpConnection::Tls)
    } else {
        Ok(HttpConnection::Plain(stream))
    }
}

fn connect_resolved_addresses(
    addresses: impl IntoIterator<Item = SocketAddr>,
    timeout: Duration,
) -> Result<TcpStream, String> {
    let started = Instant::now();
    let mut attempted = 0usize;
    let mut failures = Vec::new();
    for address in addresses {
        attempted = attempted.saturating_add(1);
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            failures.push("connect timeout exhausted".to_string());
            break;
        }
        match TcpStream::connect_timeout(&address, remaining) {
            Ok(stream) => return Ok(stream),
            Err(error) => failures.push(format!("{address}: {error}")),
        }
    }
    if attempted == 0 {
        Err("no resolved address".to_string())
    } else {
        Err(failures.join("; "))
    }
}

/// Map an HTTP response to the OTLP retry policy.
fn classify_response(
    endpoint: &Endpoint,
    response: HttpResponse,
) -> Result<PostSuccess, PostError> {
    let status = response.status;
    if (200..300).contains(&status) {
        // OTLP partial success: some spans rejected but the request itself is
        // accepted — must NOT be retried. Surface it loudly and move on.
        let partial_rejected = response.partial_rejected.unwrap_or_default();
        if partial_rejected > 0 {
            eprintln!(
                "{OTEL_HTTP_EXPORTER_NAME}: collector {} accepted batch with partial success, \
                 {partial_rejected} spans rejected",
                endpoint.authority()
            );
        }
        return Ok(PostSuccess { partial_rejected });
    }
    let detail = format!("collector {} returned HTTP {status}", endpoint.authority());
    if status == HTTP_PAYLOAD_TOO_LARGE {
        Err(PostError::TooLarge { detail })
    } else if is_retryable_status(status) {
        Err(PostError::Retryable {
            detail,
            retry_after: response.retry_after,
        })
    } else {
        Err(PostError::Permanent { detail })
    }
}

/// Wrap an established TCP connection in a verified TLS session. The server
/// certificate is always verified (against `ca_cert_path`, else the system
/// trust store); a configured client cert/key is presented for mutual TLS.
fn tls_connect(
    host: &str,
    stream: TcpStream,
    tls: &OtelHttpTlsConfig,
) -> Result<openssl::ssl::SslStream<TcpStream>, String> {
    use openssl::ssl::{SslConnector, SslFiletype, SslMethod};

    let mut builder = SslConnector::builder(SslMethod::tls_client())
        .map_err(|error| format!("tls: build connector: {error}"))?;
    if let Some(ca) = tls.ca_cert_path.as_deref() {
        builder
            .set_ca_file(ca)
            .map_err(|error| format!("tls: load ca {ca}: {error}"))?;
    }
    if let (Some(cert), Some(key)) = (
        tls.client_cert_path.as_deref(),
        tls.client_key_path.as_deref(),
    ) {
        builder
            .set_certificate_chain_file(cert)
            .map_err(|error| format!("tls: load client cert {cert}: {error}"))?;
        builder
            .set_private_key_file(key, SslFiletype::PEM)
            .map_err(|error| format!("tls: load client key {key}: {error}"))?;
        builder
            .check_private_key()
            .map_err(|error| format!("tls: client key does not match cert: {error}"))?;
    }
    builder
        .build()
        .connect(host, stream)
        .map_err(|error| format!("tls: handshake with {host}: {error}"))
}

/// The parts of a collector response the retry policy needs.
struct HttpResponse {
    status: u16,
    retry_after: Option<Duration>,
    /// `rejectedSpans` from an OTLP partial-success body, when present.
    partial_rejected: Option<u64>,
    connection_close: bool,
}

/// Cap on the response bytes we buffer — status line + headers + a small OTLP
/// `ExportTraceServiceResponse` are tiny; anything larger is rejected.
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// Send the POST and read the response over an established stream (plain or
/// TLS). Returns the parsed status/headers/partial-success; a `String` error is
/// a transport failure (always retryable at the call site).
fn http_exchange<S: Read + Write>(
    mut stream: S,
    endpoint: &Endpoint,
    body: &[u8],
    content_type: &str,
    content_encoding: Option<&str>,
    extra_headers: &str,
) -> Result<HttpResponse, String> {
    let content_encoding_header = content_encoding
        .map(|encoding| format!("Content-Encoding: {encoding}\r\n"))
        .unwrap_or_default();
    let head = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: {}\r\n\
         {}{}Content-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        endpoint.path,
        endpoint.authority(),
        content_type,
        content_encoding_header,
        extra_headers,
        body.len(),
    );
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body))
        .map_err(|error| format!("send to {}: {error}", endpoint.authority()))?;

    // Read until the response is complete (Content-Length aware) rather than to
    // EOF: a TLS peer that closes without `close_notify` surfaces an abrupt
    // "connection reset" instead of a clean EOF, so relying on EOF is fragile.
    let mut response = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break, // clean EOF
            Ok(read) => {
                response.extend_from_slice(&chunk[..read]);
                if response.len() >= MAX_RESPONSE_BYTES || response_is_complete(&response) {
                    break;
                }
            }
            // Some TLS peers omit close_notify. A close-delimited response can
            // still end this way, but a declared Content-Length must be complete.
            Err(error) => {
                if response_is_complete(&response) || response_is_close_delimited(&response) {
                    break;
                }
                return Err(format!(
                    "read response from {}: {error}",
                    endpoint.authority()
                ));
            }
        }
    }
    parse_http_response(&response)
}

/// Byte offset just past the `\r\n\r\n` header/body separator, if present.
fn header_block_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

/// A response is complete once its headers are in and the body has reached the
/// declared `Content-Length`. Without a Content-Length the length is unknown, so
/// we fall back to reading until EOF / abrupt close.
fn response_is_complete(buf: &[u8]) -> bool {
    let Some(body_start) = header_block_end(buf) else {
        return false;
    };
    let head = String::from_utf8_lossy(&buf[..body_start]);
    if response_is_chunked(&head) {
        return decode_chunked_body(&buf[body_start..]).is_ok();
    }
    match content_length(&head) {
        Some(len) => buf.len() - body_start >= len,
        None => false,
    }
}

fn response_is_close_delimited(buf: &[u8]) -> bool {
    let Some(body_start) = header_block_end(buf) else {
        return false;
    };
    let head = String::from_utf8_lossy(&buf[..body_start]);
    content_length(&head).is_none() && !response_is_chunked(&head)
}

fn content_length(head: &str) -> Option<usize> {
    head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    })
}

fn parse_http_response(response: &[u8]) -> Result<HttpResponse, String> {
    let body_start = header_block_end(response)
        .ok_or_else(|| "malformed HTTP response: missing header terminator".to_string())?;
    let head = std::str::from_utf8(&response[..body_start - 4])
        .map_err(|error| format!("malformed HTTP response headers: {error}"))?;
    let status = head
        .lines()
        .next()
        .and_then(|line| {
            let mut parts = line.split_whitespace();
            match (parts.next(), parts.next()) {
                (Some(version), Some(code)) if version.starts_with("HTTP/") => {
                    code.parse::<u16>().ok()
                }
                _ => None,
            }
        })
        .ok_or_else(|| "malformed HTTP response".to_string())?;
    let available_body = &response[body_start..];
    let decoded_chunked;
    let body = if response_is_chunked(head) {
        decoded_chunked = decode_chunked_body(available_body)?;
        decoded_chunked.as_slice()
    } else {
        match content_length(head) {
            Some(expected) if available_body.len() < expected => {
                return Err(format!(
                    "truncated HTTP response body: expected {expected} bytes, received {}",
                    available_body.len()
                ));
            }
            Some(expected) => &available_body[..expected],
            None => available_body,
        }
    };
    let partial_rejected = if (200..300).contains(&status) {
        parse_partial_rejected_response(head, body)?
    } else {
        None
    };
    Ok(HttpResponse {
        status,
        retry_after: parse_retry_after(head),
        partial_rejected,
        connection_close: response_connection_must_close(head),
    })
}

fn response_connection_must_close(head: &str) -> bool {
    let connection_close = header_value(head, "connection").is_some_and(|value| {
        value
            .split(',')
            .any(|token| token.trim().eq_ignore_ascii_case("close"))
    });
    let connection_keep_alive = header_value(head, "connection").is_some_and(|value| {
        value
            .split(',')
            .any(|token| token.trim().eq_ignore_ascii_case("keep-alive"))
    });
    let http_11_or_newer = head
        .lines()
        .next()
        .is_some_and(|line| line.starts_with("HTTP/1.1") || line.starts_with("HTTP/2"));
    connection_close
        || (!http_11_or_newer && !connection_keep_alive)
        || (content_length(head).is_none() && !response_is_chunked(head))
}

fn response_is_chunked(head: &str) -> bool {
    header_value(head, "transfer-encoding").is_some_and(|value| {
        value
            .split(',')
            .any(|token| token.trim().eq_ignore_ascii_case("chunked"))
    })
}

fn decode_chunked_body(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut offset = 0usize;
    let mut decoded = Vec::new();
    loop {
        let size_line_end = body[offset..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| "truncated chunked HTTP response size".to_string())?;
        let size_line = std::str::from_utf8(&body[offset..offset + size_line_end])
            .map_err(|error| format!("invalid chunk size line: {error}"))?;
        let size_token = size_line.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_token, 16)
            .map_err(|error| format!("invalid chunk size {size_token:?}: {error}"))?;
        offset = offset.saturating_add(size_line_end + 2);
        if size == 0 {
            if body.get(offset..offset + 2) == Some(b"\r\n") {
                return Ok(decoded);
            }
            body[offset..]
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .ok_or_else(|| "truncated chunked HTTP response trailers".to_string())?;
            return Ok(decoded);
        }
        let data_end = offset
            .checked_add(size)
            .ok_or_else(|| "chunked HTTP response size overflow".to_string())?;
        let framed_end = data_end
            .checked_add(2)
            .ok_or_else(|| "chunked HTTP response size overflow".to_string())?;
        if body.len() < framed_end {
            return Err(format!(
                "truncated chunked HTTP response body: expected {size} byte chunk"
            ));
        }
        if body.get(data_end..framed_end) != Some(b"\r\n") {
            return Err("invalid chunked HTTP response delimiter".to_string());
        }
        decoded.extend_from_slice(&body[offset..data_end]);
        offset = framed_end;
    }
}

fn parse_partial_rejected_response(head: &str, body: &[u8]) -> Result<Option<u64>, String> {
    if body.is_empty() {
        return Ok(None);
    }
    match header_value(head, "content-type")
        .and_then(|value| value.split(';').next())
        .map(str::trim)
    {
        Some(value) if value.eq_ignore_ascii_case("application/x-protobuf") => {
            export_otel_codec::parse_otlp_protobuf_partial_rejected(body)
        }
        Some(value) if value.eq_ignore_ascii_case("application/json") => {
            let body = std::str::from_utf8(body)
                .map_err(|error| format!("decode OTLP JSON response as UTF-8: {error}"))?;
            let value = serde_json::from_str::<serde_json::Value>(body)
                .map_err(|error| format!("decode OTLP JSON response: {error}"))?;
            Ok(partial_rejected_from_value(&value))
        }
        Some(value) => Err(format!("unsupported OTLP response Content-Type {value:?}")),
        None => {
            let Ok(body) = std::str::from_utf8(body) else {
                return Err("OTLP response body has no Content-Type and is not UTF-8".to_string());
            };
            Ok(parse_partial_rejected(body))
        }
    }
}

fn header_value<'a>(head: &'a str, expected_name: &str) -> Option<&'a str> {
    head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case(expected_name)
            .then_some(value.trim())
    })
}

/// Extract `partialSuccess.rejectedSpans` from an OTLP response body, tolerating
/// both the JSON string and number encodings. Absent/garbage => `None`.
fn parse_partial_rejected(body: &str) -> Option<u64> {
    let value = serde_json::from_str::<serde_json::Value>(body.trim()).ok()?;
    partial_rejected_from_value(&value)
}

fn partial_rejected_from_value(value: &serde_json::Value) -> Option<u64> {
    let rejected = value.get("partialSuccess")?.get("rejectedSpans")?;
    rejected
        .as_u64()
        .or_else(|| rejected.as_str().and_then(|s| s.parse::<u64>().ok()))
}
