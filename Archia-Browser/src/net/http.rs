use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use super::{Request, Response, Transport, TransportError};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_MAX_RESPONSE_SIZE: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct HttpTransport {
    timeout: Duration,
    max_response_size: usize,
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpTransport {
    pub const fn new() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            max_response_size: DEFAULT_MAX_RESPONSE_SIZE,
        }
    }

    pub const fn with_limits(timeout: Duration, max_response_size: usize) -> Self {
        Self {
            timeout,
            max_response_size,
        }
    }

    pub const fn timeout(&self) -> Duration {
        self.timeout
    }

    pub const fn max_response_size(&self) -> usize {
        self.max_response_size
    }
}

impl Transport for HttpTransport {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        if request.url.scheme() != "http" {
            return Err(TransportError::UnsupportedScheme);
        }

        let host = request.url.host();
        let port = request.url.effective_port();
        let address = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        let mut addresses = address
            .to_socket_addrs()
            .map_err(|_| TransportError::ConnectionFailed)?;
        let socket = addresses
            .find_map(|candidate| TcpStream::connect_timeout(&candidate, self.timeout).ok())
            .ok_or(TransportError::ConnectionFailed)?;

        socket
            .set_read_timeout(Some(self.timeout))
            .map_err(|_| TransportError::ConnectionFailed)?;
        socket
            .set_write_timeout(Some(self.timeout))
            .map_err(|_| TransportError::ConnectionFailed)?;

        let target = request_target(request);
        let host_header = host_header(request);
        let mut wire = format!(
            "{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
            request.method.as_str(),
            target,
            host_header
        );

        for (name, value) in &request.headers {
            if matches!(name.as_str(), "host" | "connection") {
                continue;
            }
            wire.push_str(name);
            wire.push_str(": ");
            wire.push_str(value);
            wire.push_str("\r\n");
        }

        if request.has_body() && request.header("content-length").is_none() {
            wire.push_str("Content-Length: ");
            wire.push_str(&request.body.len().to_string());
            wire.push_str("\r\n");
        }
        wire.push_str("\r\n");

        let mut stream = socket;
        stream
            .write_all(wire.as_bytes())
            .and_then(|_| stream.write_all(&request.body))
            .map_err(map_io_error)?;

        read_response(&mut stream, self.max_response_size)
    }
}

fn request_target(request: &Request) -> String {
    match request.url.query() {
        Some(query) if !query.is_empty() => format!("{}?{query}", request.url.path()),
        _ => request.url.path().to_owned(),
    }
}

fn host_header(request: &Request) -> String {
    let authority = request.url.authority();
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if request.url.port().is_some() {
        host.to_owned()
    } else {
        host.trim_end_matches('/').to_owned()
    }
}

fn read_response(stream: &mut TcpStream, max_size: usize) -> Result<Response, TransportError> {
    let mut raw = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 4096];
        let read = stream.read(&mut chunk).map_err(map_io_error)?;
        if read == 0 {
            return Err(TransportError::ConnectionFailed);
        }
        raw.extend_from_slice(&chunk[..read]);
        if raw.len() > max_size.saturating_add(64 * 1024) {
            return Err(TransportError::ResponseTooLarge);
        }
        if let Some(position) = find_header_end(&raw) {
            break position;
        }
    };

    let (header_bytes, body_start) = raw.split_at(header_end);
    let body_start = &body_start[4..];
    let header_text =
        std::str::from_utf8(header_bytes).map_err(|_| TransportError::InvalidResponse)?;
    let mut lines = header_text.split("\r\n");
    let status_line = lines.next().ok_or(TransportError::InvalidResponse)?;
    let mut status_parts = status_line.splitn(3, ' ');
    let version = status_parts.next().unwrap_or_default();
    let status = status_parts
        .next()
        .ok_or(TransportError::InvalidResponse)?
        .parse::<u16>()
        .map_err(|_| TransportError::InvalidResponse)?;
    if !version.starts_with("HTTP/") {
        return Err(TransportError::InvalidResponse);
    }

    let mut response = Response::new(status);
    let mut content_length = None;
    let mut chunked = false;

    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        if name == "content-length" {
            content_length = value.parse::<usize>().ok();
        }
        if name == "transfer-encoding"
            && value
                .split(',')
                .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
        {
            chunked = true;
        }
        response = response.with_header(name, value);
    }

    let mut body = body_start.to_vec();
    if chunked {
        while !chunked_message_complete(&body) {
            read_more(stream, &mut body, max_size)?;
        }
        body = decode_chunked(&body, max_size)?;
    } else if let Some(length) = content_length {
        if length > max_size {
            return Err(TransportError::ResponseTooLarge);
        }
        while body.len() < length {
            read_more(stream, &mut body, length)?;
        }
        body.truncate(length);
    } else {
        while body.len() < max_size {
            let before = body.len();
            read_more_allow_eof(stream, &mut body, max_size)?;
            if body.len() == before {
                break;
            }
        }
    }

    if body.len() > max_size {
        return Err(TransportError::ResponseTooLarge);
    }
    response.body = body;
    Ok(response)
}

fn read_more(
    stream: &mut TcpStream,
    body: &mut Vec<u8>,
    limit: usize,
) -> Result<(), TransportError> {
    if body.len() >= limit {
        return Err(TransportError::ResponseTooLarge);
    }
    let mut chunk = [0_u8; 8192];
    let read = stream.read(&mut chunk).map_err(map_io_error)?;
    if read == 0 {
        return Err(TransportError::InvalidResponse);
    }
    if body.len().saturating_add(read) > limit {
        return Err(TransportError::ResponseTooLarge);
    }
    body.extend_from_slice(&chunk[..read]);
    Ok(())
}

fn read_more_allow_eof(
    stream: &mut TcpStream,
    body: &mut Vec<u8>,
    limit: usize,
) -> Result<(), TransportError> {
    if body.len() >= limit {
        return Ok(());
    }
    let mut chunk = [0_u8; 8192];
    let read = stream.read(&mut chunk).map_err(map_io_error)?;
    if read == 0 {
        return Ok(());
    }
    if body.len().saturating_add(read) > limit {
        return Err(TransportError::ResponseTooLarge);
    }
    body.extend_from_slice(&chunk[..read]);
    Ok(())
}

fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4).position(|window| window == b"\r\n\r\n")
}

fn chunked_message_complete(body: &[u8]) -> bool {
    let mut cursor = 0;
    loop {
        let Some(line_end) = find_crlf(body, cursor) else {
            return false;
        };
        let line = &body[cursor..line_end];
        let Ok(size_text) = std::str::from_utf8(line) else {
            return false;
        };
        let size_text = size_text.split(';').next().unwrap_or_default().trim();
        let Ok(size) = usize::from_str_radix(size_text, 16) else {
            return false;
        };
        cursor = line_end + 2;
        let Some(end) = cursor
            .checked_add(size)
            .and_then(|value| value.checked_add(2))
        else {
            return false;
        };
        if end > body.len() {
            return false;
        }
        if size == 0 {
            return body[cursor + 2..]
                .windows(2)
                .any(|window| window == b"\r\n");
        }
        cursor = end;
    }
}

fn decode_chunked(body: &[u8], max_size: usize) -> Result<Vec<u8>, TransportError> {
    let mut cursor = 0;
    let mut decoded = Vec::new();
    loop {
        let line_end = find_crlf(body, cursor).ok_or(TransportError::InvalidResponse)?;
        let line = std::str::from_utf8(&body[cursor..line_end])
            .map_err(|_| TransportError::InvalidResponse)?;
        let size_text = line.split(';').next().unwrap_or_default().trim();
        let size =
            usize::from_str_radix(size_text, 16).map_err(|_| TransportError::InvalidResponse)?;
        cursor = line_end + 2;
        if size == 0 {
            if cursor + 2 > body.len() {
                return Err(TransportError::InvalidResponse);
            }
            return Ok(decoded);
        }
        let end = cursor
            .checked_add(size)
            .and_then(|value| value.checked_add(2))
            .ok_or(TransportError::ResponseTooLarge)?;
        if end > body.len() || &body[cursor + size..end] != b"\r\n" {
            return Err(TransportError::InvalidResponse);
        }
        if decoded.len().saturating_add(size) > max_size {
            return Err(TransportError::ResponseTooLarge);
        }
        decoded.extend_from_slice(&body[cursor..cursor + size]);
        cursor = end;
    }
}

fn find_crlf(body: &[u8], start: usize) -> Option<usize> {
    body.get(start..)?
        .windows(2)
        .position(|window| window == b"\r\n")
        .map(|offset| start + offset)
}

fn map_io_error(error: io::Error) -> TransportError {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => TransportError::Timeout,
        _ => TransportError::ConnectionFailed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_request_target_with_query() {
        let request = Request::new(super::super::Url::parse("http://example.org/a?q=1").unwrap());
        assert_eq!(request_target(&request), "/a?q=1");
    }

    #[test]
    fn parses_content_length_response() {
        let mut stream = std::io::Cursor::new(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 5\r\n\r\nHello"
                .to_vec(),
        );
        let response = read_response_from_reader(&mut stream, 1024).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"Hello");
        assert_eq!(response.content_type.as_deref(), Some("text/html"));
    }

    #[test]
    fn decodes_chunked_body() {
        let body = b"5\r\nHello\r\n0\r\n\r\n";
        assert_eq!(decode_chunked(body, 1024).unwrap(), b"Hello");
    }

    fn read_response_from_reader(
        reader: &mut impl Read,
        max_size: usize,
    ) -> Result<Response, TransportError> {
        let mut raw = Vec::new();
        loop {
            let mut chunk = [0_u8; 1024];
            let read = reader.read(&mut chunk).map_err(map_io_error)?;
            if read == 0 {
                return Err(TransportError::ConnectionFailed);
            }
            raw.extend_from_slice(&chunk[..read]);
            if let Some(header_end) = find_header_end(&raw) {
                let (header_bytes, body_start) = raw.split_at(header_end);
                let header_text = std::str::from_utf8(header_bytes)
                    .map_err(|_| TransportError::InvalidResponse)?;
                let mut lines = header_text.split("\r\n");
                let status_line = lines.next().ok_or(TransportError::InvalidResponse)?;
                let status = status_line
                    .split_whitespace()
                    .nth(1)
                    .ok_or(TransportError::InvalidResponse)?
                    .parse::<u16>()
                    .map_err(|_| TransportError::InvalidResponse)?;
                let mut response = Response::new(status);
                let mut length = 0;
                for line in lines {
                    if let Some((name, value)) = line.split_once(':') {
                        response = response.with_header(name.trim(), value.trim());
                        if name.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse().unwrap_or_default();
                        }
                    }
                }
                let body = &body_start[4..];
                if body.len() < length {
                    return Err(TransportError::InvalidResponse);
                }
                response.body = body[..length].to_vec();
                return Ok(response);
            }
        }
    }
}
