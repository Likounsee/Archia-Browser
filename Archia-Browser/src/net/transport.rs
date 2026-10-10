use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use super::{Request, Response, Transport, TransportError};

const MAX_REQUEST_HEADER_BYTES: usize = 64 * 1024;
const MAX_INTERIM_RESPONSES: usize = 16;

/// Minimal HTTP/1.1 transport with certificate-validated TLS for HTTPS origins.
#[derive(Debug, Clone)]
pub struct HttpTransport {
    connect_timeout: Duration,
    read_timeout: Duration,
    max_response_size: usize,
    max_header_size: usize,
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpTransport {
    pub const fn new() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(20),
            max_response_size: 8 * 1024 * 1024,
            max_header_size: 64 * 1024,
        }
    }

    pub const fn with_timeouts(connect_timeout: Duration, read_timeout: Duration) -> Self {
        Self {
            connect_timeout,
            read_timeout,
            max_response_size: 8 * 1024 * 1024,
            max_header_size: 64 * 1024,
        }
    }

    pub const fn with_limits(mut self, max_response_size: usize, max_header_size: usize) -> Self {
        self.max_response_size = max_response_size;
        self.max_header_size = max_header_size;
        self
    }

    fn connect(&self, request: &Request) -> Result<Box<dyn ReadWrite>, TransportError> {
        match request.url.scheme() {
            "http" | "https" => {}
            _ => return Err(TransportError::UnsupportedScheme),
        }

        let address = socket_address(&request.url);
        let addresses = address
            .to_socket_addrs()
            .map_err(|_| TransportError::ConnectionFailed)?;
        let stream = addresses
            .into_iter()
            .find_map(|address| TcpStream::connect_timeout(&address, self.connect_timeout).ok())
            .ok_or(TransportError::ConnectionFailed)?;

        stream
            .set_read_timeout(Some(self.read_timeout))
            .map_err(|_| TransportError::ConnectionFailed)?;
        stream
            .set_write_timeout(Some(self.read_timeout))
            .map_err(|_| TransportError::ConnectionFailed)?;

        if request.url.scheme() == "http" {
            return Ok(Box::new(stream));
        }

        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let server_name = tls_server_name(request.url.host())?;
        let connection = ClientConnection::new(Arc::new(config), server_name)
            .map_err(|_| TransportError::TlsFailed)?;
        Ok(Box::new(StreamOwned::new(connection, stream)))
    }

    fn write_request(
        &self,
        stream: &mut dyn ReadWrite,
        request: &Request,
    ) -> Result<(), TransportError> {
        validate_request(request)?;
        let mut head = String::new();
        head.push_str(request.method.as_str());
        head.push(' ');
        head.push_str(request.url.path());
        if let Some(query) = request.url.query() {
            head.push('?');
            head.push_str(query);
        }
        head.push_str(" HTTP/1.1\r\n");

        if !request
            .headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("host"))
        {
            head.push_str("Host: ");
            head.push_str(&host_header(request));
            head.push_str("\r\n");
        }
        // The response reader consumes until EOF. Force connection-close
        // semantics rather than allowing a caller-supplied keep-alive header
        // to make an otherwise complete response wait until the read timeout.
        head.push_str("Connection: close\r\n");
        if request.has_body()
            && !request
                .headers
                .keys()
                .any(|name| name.eq_ignore_ascii_case("content-length"))
        {
            head.push_str("Content-Length: ");
            head.push_str(&request.body.len().to_string());
            head.push_str("\r\n");
        }

        for (name, value) in &request.headers {
            if name.eq_ignore_ascii_case("connection") {
                continue;
            }
            head.push_str(name);
            head.push_str(": ");
            head.push_str(value);
            head.push_str("\r\n");
        }
        head.push_str("\r\n");

        stream
            .write_all(head.as_bytes())
            .and_then(|_| stream.write_all(&request.body))
            .map_err(|_| TransportError::ConnectionFailed)
    }

    fn read_response(
        &self,
        stream: &mut dyn ReadWrite,
        method: super::HttpMethod,
    ) -> Result<Response, TransportError> {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 16 * 1024];
        let mut header_end = None;

        loop {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    bytes.extend_from_slice(&buffer[..count]);
                    if header_end.is_none() {
                        header_end = bytes
                            .windows(4)
                            .position(|window| window == b"\r\n\r\n")
                            .map(|position| position + 4);
                        if header_end.is_none() && bytes.len() > self.max_header_size {
                            return Err(TransportError::ResponseTooLarge);
                        }
                    }
                    if bytes.len() > self.max_response_size {
                        return Err(TransportError::ResponseTooLarge);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                    return Err(TransportError::Timeout)
                }
                Err(_) => return Err(TransportError::ConnectionFailed),
            }
        }

        if bytes.len() > self.max_response_size {
            return Err(TransportError::ResponseTooLarge);
        }
        parse_http_response_for_method(&bytes, method, self.max_response_size, self.max_header_size)
    }
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

fn tls_server_name(host: &str) -> Result<ServerName<'static>, TransportError> {
    if let Ok(address) = host.parse::<std::net::IpAddr>() {
        return Ok(ServerName::IpAddress(address.into()));
    }
    ServerName::try_from(host.to_owned()).map_err(|_| TransportError::TlsFailed)
}

impl Transport for HttpTransport {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        // Reject malformed request targets before DNS lookup or socket creation.
        validate_request(request)?;
        let mut stream = self.connect(request)?;
        self.write_request(&mut stream, request)?;
        self.read_response(&mut stream, request.method)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LocalFileTransport {
    max_file_size: usize,
}

impl Default for LocalFileTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalFileTransport {
    pub const fn new() -> Self {
        Self {
            max_file_size: 8 * 1024 * 1024,
        }
    }

    pub const fn with_max_file_size(mut self, max_file_size: usize) -> Self {
        self.max_file_size = max_file_size;
        self
    }
}

impl Transport for LocalFileTransport {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        if request.url.scheme() != "file"
            || !request.url.authority().is_empty()
                && !request.url.authority().eq_ignore_ascii_case("localhost")
        {
            return Err(TransportError::UnsupportedScheme);
        }
        if !matches!(
            request.method,
            super::HttpMethod::Get | super::HttpMethod::Head
        ) || request.has_body()
        {
            return Err(TransportError::InvalidRequest);
        }

        let path = file_url_path(request.url.path())?;
        let metadata = std::fs::metadata(&path).map_err(|_| TransportError::ConnectionFailed)?;
        if !metadata.is_file() || metadata.len() > self.max_file_size as u64 {
            return Err(if metadata.len() > self.max_file_size as u64 {
                TransportError::ResponseTooLarge
            } else {
                TransportError::ConnectionFailed
            });
        }

        let body = if request.method == super::HttpMethod::Head {
            Vec::new()
        } else {
            let file = std::fs::File::open(&path).map_err(|_| TransportError::ConnectionFailed)?;
            let read_limit = u64::try_from(self.max_file_size)
                .unwrap_or(u64::MAX)
                .saturating_add(1);
            let mut body = Vec::new();
            file.take(read_limit)
                .read_to_end(&mut body)
                .map_err(|_| TransportError::ConnectionFailed)?;
            if body.len() > self.max_file_size {
                return Err(TransportError::ResponseTooLarge);
            }
            body
        };
        let content_length = if request.method == super::HttpMethod::Head {
            metadata.len()
        } else {
            body.len() as u64
        };
        let content_type = content_type_for_path(&path);
        Ok(Response::new(200)
            .with_header("content-type", content_type)
            .with_header("content-length", content_length.to_string())
            .with_body(body))
    }
}

fn file_url_path(url_path: &str) -> Result<std::path::PathBuf, TransportError> {
    let decoded = percent_decode(url_path)?;
    if decoded.as_bytes().contains(&0) {
        return Err(TransportError::InvalidUrl(super::UrlError::InvalidScheme));
    }
    #[cfg(windows)]
    let path = decoded
        .strip_prefix('/')
        .filter(|value| value.as_bytes().get(1) == Some(&b':'))
        .unwrap_or(&decoded);
    #[cfg(not(windows))]
    let path = decoded.as_str();
    Ok(std::path::PathBuf::from(path))
}

fn percent_decode(input: &str) -> Result<String, TransportError> {
    let mut output = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(TransportError::InvalidRequest);
            }
            let high = hex_value(bytes[index + 1]).ok_or(TransportError::InvalidRequest)?;
            let low = hex_value(bytes[index + 2]).ok_or(TransportError::InvalidRequest)?;
            output.push(high << 4 | low);
            index += 3;
        } else {
            let character = input[index..]
                .chars()
                .next()
                .ok_or(TransportError::InvalidRequest)?;
            let mut encoded = [0_u8; 4];
            let encoded = character.encode_utf8(&mut encoded);
            output.extend_from_slice(encoded.as_bytes());
            index += encoded.len();
        }
    }

    String::from_utf8(output).map_err(|_| TransportError::InvalidRequest)
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn content_type_for_path(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("xhtml") => "application/xhtml+xml",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("xml") => "application/xml",
        Some("txt") => "text/plain; charset=utf-8",
        Some("csv") => "text/csv; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/vnd.microsoft.icon",
        Some("avif") => "image/avif",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("wasm") => "application/wasm",
        Some("pdf") => "application/pdf",
        Some("mp3") => "audio/mpeg",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        _ => "application/octet-stream",
    }
}

fn validate_request(request: &Request) -> Result<(), TransportError> {
    let authority = request.url.authority();
    if authority
        .bytes()
        .any(|byte| byte.is_ascii_control() || matches!(byte, b' ' | b'\t'))
        || request
            .url
            .path()
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
        || request.url.query().is_some_and(|query| {
            query
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte == b' ')
        })
    {
        return Err(TransportError::InvalidRequest);
    }

    if matches!(request.url.scheme(), "http" | "https") {
        validate_http_authority(authority)?;
        if request
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
            .any(|(_, host)| !host.eq_ignore_ascii_case(&host_header(request)))
        {
            return Err(TransportError::InvalidRequest);
        }
    }

    let mut seen_headers = std::collections::HashSet::new();
    for (name, value) in &request.headers {
        if name.is_empty()
            || !name.bytes().all(is_http_token_byte)
            || value.bytes().any(|byte| {
                matches!(byte, b'\r' | b'\n') || byte.is_ascii_control() && byte != b'\t'
            })
            // HTTP field names are case-insensitive. Reject duplicate spellings
            // so framing/security checks cannot inspect a different value than
            // the one a downstream server chooses.
            || (!seen_headers.insert(name.to_ascii_lowercase())
                && !name.eq_ignore_ascii_case("set-cookie"))
        {
            return Err(TransportError::InvalidRequest);
        }
    }

    // Request.headers is public, so callers can bypass with_header()'s
    // lowercase normalization. Framing checks must therefore be
    // case-insensitive just like HTTP field names, or mixed-case fields can
    // bypass validation while still being serialized onto the wire.
    if request
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("transfer-encoding"))
    {
        // This transport writes bodies verbatim and does not implement
        // chunked request encoding.
        return Err(TransportError::InvalidRequest);
    }

    let content_lengths = request
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>();
    if content_lengths.len() > 1 {
        return Err(TransportError::InvalidRequest);
    }
    if let Some(length) = content_lengths.first() {
        let length = trim_http_ows(length)
            .parse::<usize>()
            .map_err(|_| TransportError::InvalidRequest)?;
        if length != request.body.len() {
            return Err(TransportError::InvalidRequest);
        }
    }

    // Bound the serialized request line and headers before connecting. Requests
    // are caller-constructible, so a valid but enormous URL/header map must not
    // make the transport allocate or write an unbounded HTTP header block.
    let target_len = request
        .url
        .path()
        .len()
        .checked_add(request.url.query().map_or(0, |query| query.len() + 1))
        .ok_or(TransportError::InvalidRequest)?;
    let mut head_len = request
        .method
        .as_str()
        .len()
        .checked_add(target_len)
        .and_then(|length| length.checked_add(12)) // spaces, HTTP version, CRLF
        .ok_or(TransportError::InvalidRequest)?;
    let has_host = request
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("host"));
    if !has_host {
        head_len = head_len
            .checked_add(host_header(request).len() + 8) // "Host: " + CRLF
            .ok_or(TransportError::InvalidRequest)?;
    }
    head_len = head_len
        .checked_add(19) // "Connection: close\\r\\n"
        .ok_or(TransportError::InvalidRequest)?;
    if request.has_body()
        && !request
            .headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("content-length"))
    {
        head_len = head_len
            .checked_add(16 + request.body.len().to_string().len() + 2)
            .ok_or(TransportError::InvalidRequest)?;
    }
    for (name, value) in &request.headers {
        if name.eq_ignore_ascii_case("connection") {
            continue;
        }
        head_len = head_len
            .checked_add(name.len())
            .and_then(|length| length.checked_add(value.len()))
            .and_then(|length| length.checked_add(4)) // ": " + CRLF
            .ok_or(TransportError::InvalidRequest)?;
    }
    head_len = head_len
        .checked_add(2) // final CRLF
        .ok_or(TransportError::InvalidRequest)?;
    if head_len > MAX_REQUEST_HEADER_BYTES {
        return Err(TransportError::InvalidRequest);
    }

    Ok(())
}

fn validate_http_authority(authority: &str) -> Result<(), TransportError> {
    // User-info is not used by this browser and makes host parsing ambiguous.
    if authority.is_empty() || authority.contains('@') {
        return Err(TransportError::InvalidRequest);
    }

    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let end = bracketed.find(']').ok_or(TransportError::InvalidRequest)?;
        let host = &bracketed[..end];
        host.parse::<std::net::Ipv6Addr>()
            .map_err(|_| TransportError::InvalidRequest)?;
        let suffix = &bracketed[end + 1..];
        let port = if suffix.is_empty() {
            None
        } else {
            Some(
                suffix
                    .strip_prefix(':')
                    .ok_or(TransportError::InvalidRequest)?,
            )
        };
        (host, port)
    } else {
        if authority.matches(':').count() > 1 {
            return Err(TransportError::InvalidRequest);
        }
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };

    // A single trailing dot is valid DNS absolute-name notation. Validate
    // the labels without it, while preserving the original authority for the
    // request target and Host header.
    let hostname = host.strip_suffix('.').unwrap_or(host);
    if host.is_empty()
        || (!authority.starts_with('[')
            && (hostname.is_empty()
                || hostname.starts_with('.')
                || hostname.split('.').any(|label| {
                    label.is_empty()
                        || label.starts_with('-')
                        || label.ends_with('-')
                        || !label
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                })))
    {
        return Err(TransportError::InvalidRequest);
    }

    if let Some(port) = port {
        let parsed = port
            .parse::<u16>()
            .map_err(|_| TransportError::InvalidRequest)?;
        if parsed == 0 {
            return Err(TransportError::InvalidRequest);
        }
    }

    Ok(())
}

fn socket_address(url: &super::Url) -> String {
    let host = url.host();
    if host.contains(':') {
        format!("[{host}]:{}", url.effective_port())
    } else {
        format!("{host}:{}", url.effective_port())
    }
}

fn host_header(request: &Request) -> String {
    let host = request.url.host();
    let default_port = matches!(
        (request.url.scheme(), request.url.effective_port()),
        ("http", 80) | ("https", 443)
    );
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    if default_port {
        host
    } else {
        format!("{host}:{}", request.url.effective_port())
    }
}

fn parse_http_response(
    bytes: &[u8],
    max_response_size: usize,
    max_header_size: usize,
) -> Result<Response, TransportError> {
    parse_http_response_for_method(
        bytes,
        super::HttpMethod::Get,
        max_response_size,
        max_header_size,
    )
}

fn parse_http_response_for_method(
    bytes: &[u8],
    method: super::HttpMethod,
    max_response_size: usize,
    max_header_size: usize,
) -> Result<Response, TransportError> {
    let mut cursor = 0;
    let mut interim_responses = 0;

    loop {
        let (response, body_start) = parse_http_response_head(&bytes[cursor..], max_header_size)?;

        if is_interim_response(response.status) {
            // Informational responses cannot carry message framing fields.
            // Reject them rather than letting a peer disagree about where the
            // next response starts.
            if response.header("content-length").is_some()
                || response.header("transfer-encoding").is_some()
            {
                return Err(TransportError::ConnectionFailed);
            }
            interim_responses += 1;
            if interim_responses > MAX_INTERIM_RESPONSES {
                return Err(TransportError::ConnectionFailed);
            }
            cursor = cursor
                .checked_add(body_start)
                .ok_or(TransportError::ConnectionFailed)?;
            if cursor >= bytes.len() {
                return Err(TransportError::ConnectionFailed);
            }
            continue;
        }

        let body_bytes = &bytes[cursor + body_start..];
        let transfer_encoding = response.header("transfer-encoding");
        let content_length = response.header("content-length");
        if response.status == 204 && (transfer_encoding.is_some() || content_length.is_some()) {
            return Err(TransportError::ConnectionFailed);
        }
        if transfer_encoding.is_some() && content_length.is_some() {
            return Err(TransportError::ConnectionFailed);
        }
        // Validate Content-Length even on HEAD and body-forbidden status codes.
        // Those responses carry no body, but malformed framing metadata must
        // not become accepted merely because the body branch is skipped.
        let parsed_content_length = content_length
            .map(|length| trim_http_ows(length).parse::<usize>())
            .transpose()
            .map_err(|_| TransportError::ConnectionFailed)?;
        // 205 Reset Content must carry no content. Reject transfer coding
        // (which this no-body parser cannot consume) and non-zero lengths.
        if response.status == 205
            && (transfer_encoding.is_some()
                || parsed_content_length.is_some_and(|length| length != 0))
        {
            return Err(TransportError::ConnectionFailed);
        }
        let is_chunked = match transfer_encoding {
            Some(value) if trim_http_ows(value).eq_ignore_ascii_case("chunked") => true,
            Some(_) => return Err(TransportError::ConnectionFailed),
            None => false,
        };

        let body_forbidden = matches!(method, crate::net::HttpMethod::Head)
            || matches!(response.status, 204 | 205 | 304);

        let body = if body_forbidden {
            if !body_bytes.is_empty() {
                return Err(TransportError::ConnectionFailed);
            }
            Vec::new()
        } else if is_chunked {
            decode_chunked(body_bytes, max_response_size, max_header_size)?
        } else if let Some(length) = parsed_content_length {
            if length > max_response_size {
                return Err(TransportError::ResponseTooLarge);
            }
            if body_bytes.len() != length {
                return Err(TransportError::ConnectionFailed);
            }
            body_bytes.to_vec()
        } else {
            body_bytes.to_vec()
        };

        if body.len() > max_response_size {
            return Err(TransportError::ResponseTooLarge);
        }

        return Ok(response.with_body(body));
    }
}

fn parse_http_response_head(
    bytes: &[u8],
    max_header_size: usize,
) -> Result<(Response, usize), TransportError> {
    let separator = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(TransportError::ConnectionFailed)?;
    let header_bytes = &bytes[..separator];
    if header_bytes.len() > max_header_size {
        return Err(TransportError::ResponseTooLarge);
    }

    let header_text =
        std::str::from_utf8(header_bytes).map_err(|_| TransportError::ConnectionFailed)?;
    let mut lines = header_text.split("\r\n");
    let status_line = lines.next().ok_or(TransportError::ConnectionFailed)?;
    let mut status_parts = status_line.splitn(3, ' ');
    let version = status_parts.next().unwrap_or_default();
    let status_text = status_parts.next().unwrap_or_default();
    let Some(reason) = status_parts.next() else {
        return Err(TransportError::ConnectionFailed);
    };
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1")
        || status_text.len() != 3
        || !status_text.bytes().all(|byte| byte.is_ascii_digit())
        || reason
            .bytes()
            .any(|byte| (byte < 0x20 && byte != b'\t') || byte == 0x7f)
    {
        return Err(TransportError::ConnectionFailed);
    }
    let status = status_text
        .parse::<u16>()
        .map_err(|_| TransportError::ConnectionFailed)?;
    if !(100..=599).contains(&status) || status == 101 {
        // Protocol upgrades are not supported by this request/response transport.
        // Do not expose upgraded protocol bytes to the document parser.
        return Err(TransportError::ConnectionFailed);
    }

    let mut response = Response::new(status);
    let mut seen_headers = std::collections::HashSet::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(TransportError::ConnectionFailed);
        };
        // Field names must be HTTP tokens; trimming malformed names can turn
        // an invalid response into a different, accepted header. Reject
        // duplicates case-insensitively so consumers cannot observe a
        // merged/overwritten value different from the validated sequence.
        if name.is_empty()
            || !name.bytes().all(is_http_token_byte)
            || value
                .bytes()
                .any(|byte| (byte < 0x20 && byte != b'\t') || byte == 0x7f)
            || !seen_headers.insert(name.to_ascii_lowercase())
        {
            return Err(TransportError::ConnectionFailed);
        }
        let value = trim_http_ows(value);
        response = response.with_header(name, value);
    }

    Ok((response, separator + 4))
}

fn trim_http_ows(value: &str) -> &str {
    // HTTP optional whitespace is ASCII SP / HTAB only. Unicode str::trim()
    // accepts characters such as NBSP that remain distinct bytes on the wire,
    // creating parser/peer disagreement around framing fields.
    value.trim_matches(|character| character == ' ' || character == '\t')
}

fn is_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            33 | 35..=39 | 42..=43 | 45..=46 | 94..=96 | 124 | 126
        )
}

fn is_interim_response(status: u16) -> bool {
    (100..200).contains(&status) && status != 101
}

fn decode_chunked(
    bytes: &[u8],
    max_response_size: usize,
    max_header_size: usize,
) -> Result<Vec<u8>, TransportError> {
    let mut output = Vec::new();
    let mut cursor = 0;

    loop {
        let relative_end = bytes[cursor..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or(TransportError::ConnectionFailed)?;
        if relative_end > max_header_size {
            return Err(TransportError::ResponseTooLarge);
        }
        let line_end = cursor + relative_end;
        let line = std::str::from_utf8(&bytes[cursor..line_end])
            .map_err(|_| TransportError::ConnectionFailed)?;
        if line
            .bytes()
            .any(|byte| !byte.is_ascii() || (byte < 0x20 && byte != b'\t') || byte == 0x7f)
        {
            return Err(TransportError::ConnectionFailed);
        }
        let size_text = line.split(';').next().unwrap_or_default();
        if size_text.is_empty() || !size_text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(TransportError::ConnectionFailed);
        }
        let size =
            usize::from_str_radix(size_text, 16).map_err(|_| TransportError::ConnectionFailed)?;
        cursor = line_end + 2;

        if size == 0 {
            let trailer_bytes = &bytes[cursor..];
            if trailer_bytes == b"\r\n" {
                return Ok(output);
            }
            let trailer_end = trailer_bytes
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .ok_or(TransportError::ConnectionFailed)?;
            if trailer_end > max_header_size {
                return Err(TransportError::ResponseTooLarge);
            }
            if trailer_end + 4 != trailer_bytes.len() {
                return Err(TransportError::ConnectionFailed);
            }

            let trailer_text = std::str::from_utf8(&trailer_bytes[..trailer_end])
                .map_err(|_| TransportError::ConnectionFailed)?;
            let mut seen_trailers = std::collections::HashSet::new();
            for line in trailer_text.split("\r\n") {
                let Some((name, value)) = line.split_once(':') else {
                    return Err(TransportError::ConnectionFailed);
                };
                if name.is_empty()
                    || !name.bytes().all(is_http_token_byte)
                    || value
                        .bytes()
                        .any(|byte| (byte < 0x20 && byte != b'\t') || byte == 0x7f)
                    || !seen_trailers.insert(name.to_ascii_lowercase())
                    || matches!(
                        name.to_ascii_lowercase().as_str(),
                        "content-length" | "transfer-encoding" | "host"
                    )
                {
                    return Err(TransportError::ConnectionFailed);
                }
            }
            return Ok(output);
        }

        let end = cursor
            .checked_add(size)
            .ok_or(TransportError::ConnectionFailed)?;
        if output.len().saturating_add(size) > max_response_size {
            return Err(TransportError::ResponseTooLarge);
        }
        output.extend_from_slice(
            bytes
                .get(cursor..end)
                .ok_or(TransportError::ConnectionFailed)?,
        );
        if bytes.get(end..end + 2) != Some(b"\r\n") {
            return Err(TransportError::ConnectionFailed);
        }
        cursor = end + 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::Url;

    #[test]
    fn detects_common_local_web_asset_content_types_case_insensitively() {
        for (path, expected) in [
            ("index.HTML", "text/html; charset=utf-8"),
            ("app.js", "text/javascript; charset=utf-8"),
            ("app.mjs", "text/javascript; charset=utf-8"),
            ("data.json", "application/json"),
            ("site.CSS", "text/css; charset=utf-8"),
            ("image.PNG", "image/png"),
            ("font.woff2", "font/woff2"),
            ("module.wasm", "application/wasm"),
            ("unknown.custom", "application/octet-stream"),
        ] {
            assert_eq!(
                content_type_for_path(std::path::Path::new(path)),
                expected,
                "unexpected MIME type for {path}"
            );
        }
    }

    #[test]
    fn creates_tls_server_names_for_dns_and_ip_hosts() {
        assert!(tls_server_name("example.org").is_ok());
        assert!(tls_server_name("127.0.0.1").is_ok());
        assert!(tls_server_name("::1").is_ok());
        assert!(tls_server_name("").is_err());
    }

    #[test]
    fn rejects_unsupported_protocol_switching_responses() {
        assert_eq!(
            parse_http_response(
                b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n",
                1024,
                1024,
            ),
            Err(TransportError::ConnectionFailed)
        );
    }

    #[test]
    fn parses_content_length_response() {
        let response = parse_http_response(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: 5\r\n\r\nHello",
            1024,
            1024,
        )
        .unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(
            response.content_type.as_deref(),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(response.body, b"Hello");
    }

    #[test]
    fn decodes_chunked_response_with_trailer() {
        let response = parse_http_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nHello\r\n0\r\nX-Test: yes\r\n\r\n",
            1024,
            1024,
        )
        .unwrap();

        assert_eq!(response.body, b"Hello");
    }

    #[test]
    fn rejects_bytes_after_chunked_trailers() {
        let response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\nunexpected";
        assert_eq!(
            parse_http_response(response, 1024, 1024),
            Err(TransportError::ConnectionFailed)
        );
    }

    #[test]
    fn rejects_invalid_chunk_size_lines() {
        for chunk_size in [" 5", "5 ", "+5", "5;bad\u{1}extension"] {
            let response = format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{chunk_size}\r\nHello\r\n0\r\n\r\n"
            );
            assert_eq!(
                parse_http_response(response.as_bytes(), 1024, 1024),
                Err(TransportError::ConnectionFailed),
                "chunk size line must be rejected: {chunk_size:?}"
            );
        }
    }

    #[test]
    fn rejects_malformed_or_framing_chunked_trailers() {
        for trailer in [
            "Bad Header: value",
            "Content-Length: 0",
            "Transfer-Encoding: chunked",
            "X-Test: bad\u{1}value",
            "X-Test: one\r\nx-test: two",
        ] {
            let response = format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n{trailer}\r\n\r\n"
            );
            assert_eq!(
                parse_http_response(response.as_bytes(), 1024, 1024),
                Err(TransportError::ConnectionFailed),
                "trailer must be rejected: {trailer}"
            );
        }
    }

    #[test]
    fn decodes_chunked_response() {
        let response = parse_http_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nHello\r\n0\r\n\r\n",
            1024,
            1024,
        )
        .unwrap();

        assert_eq!(response.body, b"Hello");
    }

    #[test]
    fn body_forbidden_responses_still_validate_content_length_syntax() {
        for (response, method) in [
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: invalid\r\n\r\n".as_slice(),
                crate::net::HttpMethod::Head,
            ),
            (
                b"HTTP/1.1 204 No Content\r\nContent-Length: invalid\r\n\r\n".as_slice(),
                crate::net::HttpMethod::Get,
            ),
            (
                b"HTTP/1.1 304 Not Modified\r\nContent-Length: \xC2\xA05\xC2\xA0\r\n\r\n"
                    .as_slice(),
                crate::net::HttpMethod::Get,
            ),
        ] {
            assert_eq!(
                parse_http_response_for_method(response, method, 1024, 1024),
                Err(TransportError::ConnectionFailed)
            );
        }
    }

    #[test]
    fn head_response_ignores_declared_body_length() {
        let response = parse_http_response_for_method(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n",
            crate::net::HttpMethod::Head,
            1024,
            1024,
        )
        .unwrap();
        assert!(response.body.is_empty());
    }

    #[test]
    fn no_content_response_has_no_body() {
        let response = parse_http_response(b"HTTP/1.1 204 No Content\r\n\r\n", 1024, 1024).unwrap();
        assert!(response.body.is_empty());
    }

    #[test]
    fn rejects_unsupported_and_ambiguous_transfer_encodings() {
        for encoding in ["gzip", "gzip, chunked", "chunked, gzip"] {
            let response =
                format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: {encoding}\r\n\r\n0\r\n\r\n");
            assert_eq!(
                parse_http_response(response.as_bytes(), 1024, 1024),
                Err(TransportError::ConnectionFailed),
                "encoding {encoding} must not be partially decoded"
            );
        }
    }

    #[test]
    fn rejects_extra_bytes_after_content_length_body() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nHelloextra";
        assert_eq!(
            parse_http_response(response, 1024, 1024),
            Err(TransportError::ConnectionFailed)
        );
    }

    #[test]
    fn reset_content_response_must_not_contain_body_bytes() {
        let empty = parse_http_response(b"HTTP/1.1 205 Reset Content\r\n\r\n", 1024, 1024);
        assert_eq!(empty.unwrap().body, b"");

        let with_body =
            parse_http_response(b"HTTP/1.1 205 Reset Content\r\n\r\nunexpected", 1024, 1024);
        assert_eq!(with_body, Err(TransportError::ConnectionFailed));
    }

    #[test]
    fn rejects_body_bytes_for_head_and_no_content_responses() {
        let head = parse_http_response_for_method(
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\nunexpected",
            crate::net::HttpMethod::Head,
            1024,
            1024,
        );
        assert_eq!(head, Err(TransportError::ConnectionFailed));

        let no_content =
            parse_http_response(b"HTTP/1.1 204 No Content\r\n\r\nunexpected", 1024, 1024);
        assert_eq!(no_content, Err(TransportError::ConnectionFailed));
    }

    #[test]
    fn rejects_duplicate_response_headers_case_insensitively() {
        for response in [
            b"HTTP/1.1 200 OK\\r\\nContent-Length: 5\\r\\nContent-Length: 5\\r\\n\\r\\nHello".as_slice(),
            b"HTTP/1.1 200 OK\\r\\nTransfer-Encoding: chunked\\r\\nTransfer-Encoding: chunked\\r\\n\\r\\n0\\r\\n\\r\\n".as_slice(),
            b"HTTP/1.1 200 OK\\r\\nContent-Type: text/plain\\r\\ncontent-type: text/html\\r\\n\\r\\n".as_slice(),
        ] {
            assert_eq!(
                parse_http_response(response, 1024, 1024),
                Err(TransportError::ConnectionFailed)
            );
        }
    }

    #[test]
    fn preserves_multiple_set_cookie_headers() {
        let response = parse_http_response(
            b"HTTP/1.1 200 OK\r\nSet-Cookie: a=1\r\nset-cookie: b=2\r\n\r\n",
            1024,
            1024,
        )
        .unwrap();

        assert_eq!(
            response.set_cookie_headers(),
            &["a=1".to_string(), "b=2".to_string()]
        );
        assert_eq!(response.header("set-cookie"), Some("b=2"));
    }

    #[test]
    fn skips_multiple_informational_responses_before_final_response() {
        let response = parse_http_response(
            b"HTTP/1.1 103 Early Hints\r\nLink: </style.css>; rel=preload\r\n\r\nHTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nHello",
            1024,
            1024,
        )
        .unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"Hello");
    }

    #[test]
    fn reset_content_requires_zero_length_framing() {
        for response in [
            b"HTTP/1.1 205 Reset Content\r\nContent-Length: 1\r\n\r\n".as_slice(),
            b"HTTP/1.1 205 Reset Content\r\nTransfer-Encoding: chunked\r\n\r\n".as_slice(),
        ] {
            assert_eq!(
                parse_http_response(response, 1024, 1024),
                Err(TransportError::ConnectionFailed)
            );
        }

        let zero_length = parse_http_response(
            b"HTTP/1.1 205 Reset Content\r\nContent-Length: 0\r\n\r\n",
            1024,
            1024,
        )
        .unwrap();
        assert!(zero_length.body.is_empty());
    }

    #[test]
    fn rejects_framing_headers_on_informational_and_204_responses() {
        for response in [
            b"HTTP/1.1 100 Continue\r\nContent-Length: 0\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 103 Early Hints\r\nTransfer-Encoding: chunked\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 204 No Content\r\nTransfer-Encoding: chunked\r\n\r\n".as_slice(),
        ] {
            assert_eq!(
                parse_http_response(response, 1024, 1024),
                Err(TransportError::ConnectionFailed)
            );
        }
    }

    #[test]
    fn rejects_excessive_informational_responses() {
        let mut bytes = Vec::new();
        for _ in 0..=MAX_INTERIM_RESPONSES {
            bytes.extend_from_slice(b"HTTP/1.1 103 Early Hints\r\n\r\n");
        }
        bytes.extend_from_slice(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");

        assert_eq!(
            parse_http_response(&bytes, 4096, 1024),
            Err(TransportError::ConnectionFailed)
        );
    }

    #[test]
    fn rejects_invalid_response_header_names_and_control_bytes() {
        for response in [
            b"HTTP/1.1 200 OK\r\nBad Header: value\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nX-Test: safe\x01value\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\n: missing-name\r\n\r\n".as_slice(),
        ] {
            assert_eq!(
                parse_http_response(response, 1024, 1024),
                Err(TransportError::ConnectionFailed)
            );
        }
    }

    #[test]
    fn rejects_invalid_http_versions_and_status_codes() {
        for status_line in [
            "HTTP/9.9 200 OK",
            "HTTP/1.1 20 OK",
            "HTTP/1.1 200",
            "HTTP/1.1 600 Unknown",
            "HTTP/1.1 200 Bad\u{1}Reason",
        ] {
            let response = format!("{status_line}\r\n\r\n");
            assert_eq!(
                parse_http_response(response.as_bytes(), 1024, 1024),
                Err(TransportError::ConnectionFailed),
                "status line must be rejected: {status_line:?}"
            );
        }
    }

    #[test]
    fn rejects_malformed_status() {
        assert!(parse_http_response(b"not-http\r\n\r\nbody", 1024, 1024).is_err());
    }

    #[test]
    fn rejects_non_http_schemes_before_connecting() {
        let transport = HttpTransport::new();
        let request = Request::new(Url::parse("ftp://example.org/file").unwrap());
        assert_eq!(
            transport.send(&request),
            Err(TransportError::UnsupportedScheme)
        );
    }

    #[test]
    fn percent_decode_preserves_utf8_paths() {
        assert_eq!(percent_decode("/tmp/%C3%A9.html").unwrap(), "/tmp/é.html");
        assert_eq!(
            percent_decode("/tmp/caf%C3%A9.html").unwrap(),
            "/tmp/café.html"
        );
    }

    #[test]
    fn local_file_transport_loads_html() {
        let path =
            std::env::temp_dir().join(format!("archia-browser-local-{}.html", std::process::id()));
        std::fs::write(&path, b"<body>Local</body>").unwrap();
        let url = if cfg!(windows) {
            format!("file:///{}", path.display())
        } else {
            format!("file://{}", path.display())
        };
        let request = Request::new(super::super::Url::parse(&url).unwrap());
        let response = LocalFileTransport::new().send(&request).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(
            response.content_type.as_deref(),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(response.header("content-length"), Some("18"));
        assert_eq!(response.body, b"<body>Local</body>");

        let head_request = Request::new(super::super::Url::parse(&url).unwrap())
            .with_method(crate::net::HttpMethod::Head);
        let head_response = LocalFileTransport::new().send(&head_request).unwrap();
        assert_eq!(head_response.header("content-length"), Some("18"));
        assert!(head_response.body.is_empty());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn local_file_transport_rejects_non_file_urls() {
        let request = Request::new(super::super::Url::parse("https://example.org/").unwrap());
        assert_eq!(
            LocalFileTransport::new().send(&request).unwrap_err(),
            TransportError::UnsupportedScheme
        );
    }

    #[test]
    fn rejects_malformed_ports_and_user_info_before_connecting() {
        for input in [
            "http://example.org:bad/",
            "http://example.org:99999/",
            "http://example.org:/",
            "http://example.org:80:90/",
        ] {
            assert!(
                Url::parse(input).is_err(),
                "expected malformed URL authority to be rejected: {input}"
            );
        }

        let request = Request::new(Url::parse("http://user@example.org/").unwrap());
        assert_eq!(
            HttpTransport::new().send(&request),
            Err(TransportError::InvalidRequest),
            "userinfo must be rejected by the HTTP transport"
        );
    }

    #[test]
    fn accepts_absolute_dns_hostnames_with_one_trailing_dot() {
        let request = Request::new(Url::parse("http://example.org./").unwrap());
        assert_eq!(validate_request(&request), Ok(()));
        assert_eq!(host_header(&request), "example.org.");
    }

    #[test]
    fn rejects_empty_dns_labels_even_with_trailing_dots() {
        for input in [
            "http://example.org../",
            "http://example..org./",
            "http://./",
        ] {
            assert!(
                Url::parse(input).is_err(),
                "expected empty DNS labels to be rejected: {input}"
            );
        }
    }

    #[test]
    fn accepts_valid_ipv6_authorities() {
        let request = Request::new(Url::parse("http://[::1]:8080/").unwrap());
        assert_eq!(validate_request(&request), Ok(()));
    }

    #[test]
    fn rejects_authority_injection() {
        assert_eq!(
            Url::parse("http://example.org evil/"),
            Err(crate::net::url::UrlError::InvalidCharacter)
        );

        let request = Request::new(Url::parse("http://example.org/").unwrap())
            .with_header("Host", "example.org evil");
        assert_eq!(
            validate_request(&request),
            Err(TransportError::InvalidRequest)
        );
    }

    #[test]
    fn rejects_conflicting_transfer_encoding_and_content_length() {
        let result = parse_http_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 5\r\n\r\n5\r\nHello\r\n0\r\n\r\n",
            1024,
            1024,
        );
        assert_eq!(result, Err(TransportError::ConnectionFailed));
    }

    #[test]
    fn forces_connection_close_even_if_request_asks_for_keep_alive() {
        #[derive(Debug, Default)]
        struct CaptureStream {
            written: Vec<u8>,
        }

        impl Read for CaptureStream {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Ok(0)
            }
        }

        impl Write for CaptureStream {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.written.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let request = Request::new(Url::parse("http://example.org/").unwrap())
            .with_header("connection", "keep-alive");
        let transport = HttpTransport::new();
        let mut stream = CaptureStream::default();
        transport.write_request(&mut stream, &request).unwrap();

        let head = String::from_utf8(stream.written).unwrap();
        assert_eq!(head.matches("Connection: close").count(), 1);
        assert!(!head.to_ascii_lowercase().contains("connection: keep-alive"));
    }

    #[test]
    fn rejects_invalid_request_header_names_and_mismatched_host() {
        let invalid_name = Request::new(Url::parse("http://example.org/").unwrap())
            .with_header("Bad Header", "value");
        assert_eq!(
            validate_request(&invalid_name),
            Err(TransportError::InvalidRequest)
        );

        let mismatched_host = Request::new(Url::parse("http://example.org/").unwrap())
            .with_header("host", "attacker.example");
        assert_eq!(
            validate_request(&mismatched_host),
            Err(TransportError::InvalidRequest)
        );
    }

    #[test]
    fn rejects_ambiguous_request_body_framing() {
        let url = Url::parse("http://example.org/").unwrap();

        let short_length = Request::new(url.clone())
            .with_method(crate::net::HttpMethod::Post)
            .with_body(b"Hello".to_vec())
            .with_header("content-length", "4");
        assert_eq!(
            validate_request(&short_length),
            Err(TransportError::InvalidRequest)
        );

        let invalid_length = Request::new(url.clone())
            .with_body(b"Hello".to_vec())
            .with_header("content-length", "five");
        assert_eq!(
            validate_request(&invalid_length),
            Err(TransportError::InvalidRequest)
        );

        let transfer_encoded = Request::new(url.clone())
            .with_body(b"Hello".to_vec())
            .with_header("transfer-encoding", "chunked");
        assert_eq!(
            validate_request(&transfer_encoded),
            Err(TransportError::InvalidRequest)
        );

        let mut duplicate_length = Request::new(url)
            .with_body(b"Hello".to_vec())
            .with_header("content-length", "5");
        // Construct a malformed map directly: the public builder normalizes
        // names and would otherwise collapse this duplicate before validation.
        duplicate_length
            .headers
            .insert("Content-Length".into(), "5".into());
        assert_eq!(
            validate_request(&duplicate_length),
            Err(TransportError::InvalidRequest)
        );
    }

    #[test]
    fn validates_mixed_case_host_headers_case_insensitively() {
        let mut request = Request::new(Url::parse("http://example.org/").unwrap());
        request
            .headers
            .insert("Host".into(), "attacker.example".into());
        assert_eq!(
            validate_request(&request),
            Err(TransportError::InvalidRequest)
        );

        request.headers.insert("Host".into(), "example.org".into());
        assert_eq!(validate_request(&request), Ok(()));
    }

    #[test]
    fn does_not_emit_duplicate_content_length_for_mixed_case_header() {
        #[derive(Debug, Default)]
        struct CaptureStream {
            written: Vec<u8>,
        }

        impl Read for CaptureStream {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Ok(0)
            }
        }

        impl Write for CaptureStream {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.written.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let mut request =
            Request::new(Url::parse("http://example.org/").unwrap()).with_body(b"Hello".to_vec());
        request.headers.insert("Content-Length".into(), "5".into());
        let mut stream = CaptureStream::default();
        HttpTransport::new()
            .write_request(&mut stream, &request)
            .unwrap();

        let head = String::from_utf8(stream.written).unwrap();
        assert_eq!(
            head.lines()
                .filter(|line| line
                    .split_once(':')
                    .is_some_and(|(name, _)| { name.eq_ignore_ascii_case("content-length") }))
                .count(),
            1,
            "a mixed-case caller header must not trigger a second Content-Length"
        );
    }

    #[test]
    fn content_length_uses_only_http_ascii_optional_whitespace() {
        let url = Url::parse("http://example.org/").unwrap();

        let mut request = Request::new(url.clone()).with_body(b"Hello".to_vec());
        request
            .headers
            .insert("Content-Length".into(), "\u{00a0}5\u{00a0}".into());
        assert_eq!(
            validate_request(&request),
            Err(TransportError::InvalidRequest),
            "non-ASCII whitespace must not be silently stripped from wire framing"
        );

        let valid_ows = Request::new(url)
            .with_body(b"Hello".to_vec())
            .with_header("content-length", "\t5 \t");
        assert_eq!(validate_request(&valid_ows), Ok(()));

        let response = b"HTTP/1.1 200 OK\r\nContent-Length: \xC2\xA05\xC2\xA0\r\n\r\nHello";
        assert_eq!(
            parse_http_response(response, 1024, 1024),
            Err(TransportError::ConnectionFailed),
            "response framing must use the same ASCII OWS rules"
        );
    }

    #[test]
    fn rejects_mixed_case_request_framing_headers() {
        let url = Url::parse("http://example.org/").unwrap();

        let mut transfer_encoded = Request::new(url.clone()).with_body(b"Hello".to_vec());
        transfer_encoded
            .headers
            .insert("Transfer-Encoding".into(), "chunked".into());
        assert_eq!(
            validate_request(&transfer_encoded),
            Err(TransportError::InvalidRequest)
        );

        let mut invalid_length = Request::new(url.clone()).with_body(b"Hello".to_vec());
        invalid_length
            .headers
            .insert("Content-Length".into(), "4".into());
        assert_eq!(
            validate_request(&invalid_length),
            Err(TransportError::InvalidRequest)
        );

        let mut duplicate_length = Request::new(url).with_body(b"Hello".to_vec());
        duplicate_length
            .headers
            .insert("content-length".into(), "5".into());
        duplicate_length
            .headers
            .insert("Content-Length".into(), "5".into());
        assert_eq!(
            validate_request(&duplicate_length),
            Err(TransportError::InvalidRequest)
        );
    }

    #[test]
    fn rejects_oversized_request_header_block_before_connecting() {
        let request = Request::new(Url::parse("http://example.org/").unwrap())
            .with_header("x-large", "a".repeat(MAX_REQUEST_HEADER_BYTES));
        assert_eq!(
            validate_request(&request),
            Err(TransportError::InvalidRequest)
        );

        let long_target = Request::new(
            Url::parse(&format!(
                "http://example.org/{}",
                "a".repeat(MAX_REQUEST_HEADER_BYTES)
            ))
            .unwrap(),
        );
        assert_eq!(
            validate_request(&long_target),
            Err(TransportError::InvalidRequest)
        );
    }

    #[test]
    fn rejects_header_injection() {
        let transport = HttpTransport::new();
        let request = Request::new(Url::parse("http://example.org/").unwrap())
            .with_header("x-test", "safe\r\nX-Injected: yes");
        assert_eq!(
            transport.send(&request),
            Err(TransportError::InvalidRequest)
        );
    }

    #[test]
    fn formats_ipv6_socket_addresses_with_brackets() {
        let url = Url::parse("http://[::1]:8080/").unwrap();
        assert_eq!(socket_address(&url), "[::1]:8080");
    }

    #[test]
    fn omits_default_ports_from_http_and_https_host_headers() {
        let http = Request::new(Url::parse("http://example.org:80/").unwrap());
        assert_eq!(host_header(&http), "example.org");

        let https = Request::new(Url::parse("https://example.org/").unwrap());
        assert_eq!(host_header(&https), "example.org");

        let explicit_https_port = Request::new(Url::parse("https://example.org:8443/").unwrap());
        assert_eq!(host_header(&explicit_https_port), "example.org:8443");
    }

    #[test]
    fn builds_default_and_explicit_host_headers() {
        let request = Request::new(Url::parse("http://example.org/").unwrap());
        assert_eq!(host_header(&request), "example.org");

        let request = Request::new(Url::parse("http://example.org:8080/").unwrap());
        assert_eq!(host_header(&request), "example.org:8080");

        let request = Request::new(Url::parse("http://[::1]:8080/").unwrap());
        assert_eq!(host_header(&request), "[::1]:8080");
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;

    #[test]
    fn rejects_incomplete_chunk_terminator() {
        let result = parse_http_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nHello\r\n0\r\n",
            1024,
            1024,
        );
        assert_eq!(result, Err(TransportError::ConnectionFailed));
    }

    #[test]
    fn rejects_oversized_content_length() {
        let result = parse_http_response(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nHello",
            4,
            1024,
        );
        assert_eq!(result, Err(TransportError::ResponseTooLarge));
    }

    #[test]
    fn rejects_oversized_chunked_body() {
        let result = parse_http_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nHello\r\n0\r\n\r\n",
            4,
            1024,
        );
        assert_eq!(result, Err(TransportError::ResponseTooLarge));
    }

    #[test]
    fn rejects_oversized_chunk_size_line() {
        let mut response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1;".to_vec();
        response.extend(std::iter::repeat_n(b'a', 256));
        response.extend_from_slice(b"\r\nx\r\n0\r\n\r\n");

        assert_eq!(
            parse_http_response(&response, 1024, 128),
            Err(TransportError::ResponseTooLarge)
        );
    }
}
