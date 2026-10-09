use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use super::{Request, Response, Transport, TransportError};

/// Minimal HTTP/1.1 transport built directly on the platform TCP socket API.
///
/// HTTPS is intentionally rejected until the browser has a native TLS layer.
/// This keeps the transport honest: it never silently sends plaintext bytes to
/// a secure origin.
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

    fn connect(&self, request: &Request) -> Result<TcpStream, TransportError> {
        match request.url.scheme() {
            "http" => {}
            "https" => return Err(TransportError::TlsFailed),
            _ => return Err(TransportError::UnsupportedScheme),
        }

        let address = format!("{}:{}", request.url.host(), request.url.effective_port());
        let mut addresses = address
            .to_socket_addrs()
            .map_err(|_| TransportError::ConnectionFailed)?;
        let stream = addresses
            .find_map(|address| TcpStream::connect_timeout(&address, self.connect_timeout).ok())
            .ok_or(TransportError::ConnectionFailed)?;

        stream
            .set_read_timeout(Some(self.read_timeout))
            .map_err(|_| TransportError::ConnectionFailed)?;
        stream
            .set_write_timeout(Some(self.read_timeout))
            .map_err(|_| TransportError::ConnectionFailed)?;
        Ok(stream)
    }

    fn write_request(
        &self,
        stream: &mut TcpStream,
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

        if request.header("host").is_none() {
            head.push_str("Host: ");
            head.push_str(&host_header(request));
            head.push_str("\r\n");
        }
        if request.header("connection").is_none() {
            head.push_str("Connection: close\r\n");
        }
        if request.has_body() && request.header("content-length").is_none() {
            head.push_str("Content-Length: ");
            head.push_str(&request.body.len().to_string());
            head.push_str("\r\n");
        }

        for (name, value) in &request.headers {
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
        stream: &mut TcpStream,
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
            std::fs::read(&path).map_err(|_| TransportError::ConnectionFailed)?
        };
        let content_type = content_type_for_path(&path);
        Ok(Response::new(200)
            .with_header("content-type", content_type)
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
        Some("css") => "text/css",
        Some("txt") => "text/plain; charset=utf-8",
        Some("svg") => "image/svg+xml",
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
    }

    if request.headers.iter().any(|(name, value)| {
        name.is_empty()
            || name
                .bytes()
                .any(|byte| byte.is_ascii_control() || matches!(byte, b' ' | b'\t' | b':'))
            || value.bytes().any(|byte| {
                matches!(byte, b'\r' | b'\n') || byte.is_ascii_control() && byte != b'\t'
            })
    }) {
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
            Some(suffix.strip_prefix(':').ok_or(TransportError::InvalidRequest)?)
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

    if host.is_empty()
        || (!authority.starts_with('[')
            && (host.starts_with('.')
                || host == "."
                || host.split('.').any(|label| {
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

fn host_header(request: &Request) -> String {
    let host = request.url.host();
    let default_port = request.url.effective_port() == 80 && request.url.scheme() == "http";
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

    loop {
        let (response, body_start) = parse_http_response_head(&bytes[cursor..], max_header_size)?;

        if is_interim_response(response.status) {
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
        if transfer_encoding.is_some() && content_length.is_some() {
            return Err(TransportError::ConnectionFailed);
        }

        let body_forbidden =
            matches!(method, crate::net::HttpMethod::Head) || matches!(response.status, 204 | 304);

        let body = if body_forbidden {
            Vec::new()
        } else if transfer_encoding.is_some_and(|value| {
            value
                .split(',')
                .any(|item| item.trim().eq_ignore_ascii_case("chunked"))
        }) {
            decode_chunked(body_bytes, max_response_size)?
        } else if let Some(length) = content_length {
            let length = length
                .trim()
                .parse::<usize>()
                .map_err(|_| TransportError::ConnectionFailed)?;
            if length > max_response_size {
                return Err(TransportError::ResponseTooLarge);
            }
            body_bytes
                .get(..length)
                .ok_or(TransportError::ConnectionFailed)?
                .to_vec()
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
    let status = status_parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or(TransportError::ConnectionFailed)?;

    if !version.starts_with("HTTP/") {
        return Err(TransportError::ConnectionFailed);
    }

    let mut response = Response::new(status);
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(TransportError::ConnectionFailed);
        };
        let name = name.trim();
        let value = value.trim();
        if matches!(
            name.to_ascii_lowercase().as_str(),
            "content-length" | "transfer-encoding"
        ) && response.header(name).is_some()
        {
            return Err(TransportError::ConnectionFailed);
        }
        response = response.with_header(name, value);
    }

    Ok((response, separator + 4))
}

fn is_interim_response(status: u16) -> bool {
    (100..200).contains(&status) && status != 101
}

fn decode_chunked(bytes: &[u8], max_response_size: usize) -> Result<Vec<u8>, TransportError> {
    let mut output = Vec::new();
    let mut cursor = 0;

    loop {
        let relative_end = bytes[cursor..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or(TransportError::ConnectionFailed)?;
        let line_end = cursor + relative_end;
        let line = std::str::from_utf8(&bytes[cursor..line_end])
            .map_err(|_| TransportError::ConnectionFailed)?;
        let size_text = line.split(';').next().unwrap_or_default().trim();
        let size =
            usize::from_str_radix(size_text, 16).map_err(|_| TransportError::ConnectionFailed)?;
        cursor = line_end + 2;

        if size == 0 {
            let trailer_bytes = &bytes[cursor..];
            if trailer_bytes == b"\r\n" {
                return Ok(output);
            }
            if trailer_bytes.starts_with(b"\r\n") {
                return Err(TransportError::ConnectionFailed);
            }
            let trailer_end = trailer_bytes
                .windows(4)
                .position(|window| window == b"\r\n\r\n");
            if trailer_end.is_some() {
                return Ok(output);
            }
            return Err(TransportError::ConnectionFailed);
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
        let response = parse_http_response(
            b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n",
            1024,
            1024,
        )
        .unwrap();
        assert!(response.body.is_empty());
    }

    #[test]
    fn rejects_duplicate_framing_headers() {
        let result = parse_http_response(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\nHello",
            1024,
            1024,
        );
        assert_eq!(result, Err(TransportError::ConnectionFailed));

        let result = parse_http_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
            1024,
            1024,
        );
        assert_eq!(result, Err(TransportError::ConnectionFailed));
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
    fn keeps_switching_protocols_as_a_final_response() {
        let response = parse_http_response(
            b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n",
            1024,
            1024,
        )
        .unwrap();

        assert_eq!(response.status, 101);
        assert_eq!(response.header("upgrade"), Some("websocket"));
        assert!(response.body.is_empty());
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
        assert_eq!(response.body, b"<body>Local</body>");
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
            "http://user@example.org/",
            "http://example.org:80:90/",
        ] {
            let request = Request::new(Url::parse(input).unwrap());
            assert_eq!(
                HttpTransport::new().send(&request),
                Err(TransportError::InvalidRequest),
                "expected {input} to be rejected"
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
        let request = Request::new(Url::parse("http://example.org evil/").unwrap());
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
}
