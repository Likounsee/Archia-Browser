use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use archia_browser::net::{
    CancellationToken, HttpTransport, Request, Transport, TransportError, Url,
};

fn serve_once(response: &'static [u8]) -> (String, thread::JoinHandle<()>) {
    serve_once_owned(response.to_vec())
}

fn serve_once_owned(response: Vec<u8>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
    let address = listener.local_addr().expect("listener address");
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept browser connection");
        let mut request = Vec::new();
        let mut byte = [0_u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            if stream.read_exact(&mut byte).is_err() {
                return;
            }
            request.push(byte[0]);
            if request.len() > 64 * 1024 {
                return;
            }
        }
        stream.write_all(&response).expect("write test response");
        stream.flush().expect("flush test response");
    });
    (format!("http://{address}/"), worker)
}

#[test]
fn transport_enforces_total_response_limit_at_the_read_boundary() {
    const RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK";
    let (url, worker) = serve_once(RESPONSE);
    let request = Request::new(Url::parse(&url).unwrap());

    assert_eq!(
        HttpTransport::new()
            .with_limits(RESPONSE.len() - 1, 1024)
            .send(&request),
        Err(TransportError::ResponseTooLarge)
    );
    worker.join().expect("test server thread");
}

#[test]
fn transport_enforces_header_limit_when_delimiter_arrives_in_same_read() {
    const RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK";
    let header_bytes = RESPONSE
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("fixture has a complete header delimiter");
    let (url, worker) = serve_once(RESPONSE);
    let request = Request::new(Url::parse(&url).unwrap());

    assert_eq!(
        HttpTransport::new()
            .with_limits(1024, header_bytes - 1)
            .send(&request),
        Err(TransportError::ResponseTooLarge)
    );
    worker.join().expect("test server thread");
}

#[test]
fn mismatched_host_header_is_rejected_before_opening_a_connection() {
    let request = Request::new(Url::parse("http://127.0.0.1:1/").unwrap())
        .with_header("Host", "attacker.example");

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::InvalidRequest)
    );
}

#[test]
fn connect_method_is_rejected_before_opening_a_connection() {
    let request = Request::new(Url::parse("http://127.0.0.1:1/").unwrap())
        .with_method(archia_browser::net::HttpMethod::Connect);

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::InvalidRequest)
    );
}

#[test]
fn cancelled_request_fails_before_opening_a_connection() {
    let request = Request::new(Url::parse("http://127.0.0.1:1/").unwrap());
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    assert_eq!(
        HttpTransport::new().send_cancellable(&request, &cancellation),
        Err(TransportError::Cancelled)
    );
}

#[test]
fn transport_rejects_conflicting_response_framing_end_to_end() {
    let (url, worker) = serve_once(
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 4\r\nConnection: close\r\n\r\n4\r\ntest\r\n0\r\n\r\n",
    );
    let request = Request::new(Url::parse(&url).unwrap());

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::ConnectionFailed)
    );
    worker.join().expect("test server thread");
}

#[test]
fn transport_rejects_incomplete_content_length_end_to_end() {
    let (url, worker) =
        serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nshort");
    let request = Request::new(Url::parse(&url).unwrap());

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::ConnectionFailed)
    );
    worker.join().expect("test server thread");
}

#[test]
fn transport_rejects_invalid_chunk_terminator_end_to_end() {
    let (url, worker) = serve_once(
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\ntestXX0\r\n\r\n",
    );
    let request = Request::new(Url::parse(&url).unwrap());

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::ConnectionFailed)
    );
    worker.join().expect("test server thread");
}

#[test]
fn transport_rejects_connection_nominated_representation_header_end_to_end() {
    let (url, worker) = serve_once(
        b"HTTP/1.1 200 OK\r\nConnection: content-type\r\nContent-Type: text/html\r\nContent-Length: 0\r\n\r\n",
    );
    let request = Request::new(Url::parse(&url).unwrap());

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::ConnectionFailed)
    );
    worker.join().expect("test server thread");
}

#[test]
fn transport_rejects_connection_nominated_framing_header_end_to_end() {
    let (url, worker) =
        serve_once(b"HTTP/1.1 200 OK\r\nConnection: Content-Length\r\nContent-Length: 0\r\n\r\n");
    let request = Request::new(Url::parse(&url).unwrap());

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::ConnectionFailed)
    );
    worker.join().expect("test server thread");
}

#[test]
fn transport_rejects_connection_nominated_cookie_metadata_end_to_end() {
    let (url, worker) = serve_once(
        b"HTTP/1.1 200 OK\r\nConnection: Set-Cookie\r\nSet-Cookie: sid=secret\r\nContent-Length: 0\r\n\r\n",
    );
    let request = Request::new(Url::parse(&url).unwrap());

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::ConnectionFailed)
    );
    worker.join().expect("test server thread");
}

#[test]
fn transport_rejects_malformed_connection_tokens_end_to_end() {
    let (url, worker) = serve_once(
        b"HTTP/1.1 200 OK\r\nConnection: close,,keep-alive\r\nContent-Length: 0\r\n\r\n",
    );
    let request = Request::new(Url::parse(&url).unwrap());

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::ConnectionFailed)
    );
    worker.join().expect("test server thread");
}

#[test]
fn transport_accepts_connection_close_with_valid_framing_end_to_end() {
    let (url, worker) =
        serve_once(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nOK");
    let request = Request::new(Url::parse(&url).unwrap());

    let response = HttpTransport::new()
        .send(&request)
        .expect("valid close-delimited connection metadata is accepted");
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"OK");
    worker.join().expect("test server thread");
}

#[test]
fn transport_preserves_multiple_set_cookie_headers_end_to_end() {
    let (url, worker) = serve_once(
        b"HTTP/1.1 200 OK\r\nSet-Cookie: first=one; Path=/\r\nSet-Cookie: second=two; Path=/\r\nContent-Length: 2\r\n\r\nOK",
    );
    let request = Request::new(Url::parse(&url).unwrap());

    let response = HttpTransport::new()
        .send(&request)
        .expect("valid response with multiple cookies");
    assert_eq!(
        response
            .set_cookie_headers()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["first=one; Path=/", "second=two; Path=/"]
    );
    worker.join().expect("test server thread");
}

#[test]
fn transport_accepts_connection_naming_an_absent_header_end_to_end() {
    let (url, worker) =
        serve_once(b"HTTP/1.1 200 OK\r\nConnection: x-hop-by-hop\r\nContent-Length: 2\r\n\r\nOK");
    let request = Request::new(Url::parse(&url).unwrap());

    let response = HttpTransport::new()
        .send(&request)
        .expect("a nominated but absent header does not make framing ambiguous");
    assert_eq!(response.body, b"OK");
    worker.join().expect("test server thread");
}

#[test]
fn transport_accepts_early_hints_before_final_response_end_to_end() {
    let (url, worker) = serve_once(
        b"HTTP/1.1 103 Early Hints\r\nLink: </style.css>; rel=preload\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK",
    );
    let request = Request::new(Url::parse(&url).unwrap());

    let response = HttpTransport::new()
        .send(&request)
        .expect("a valid informational response must not hide the final response");
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"OK");
    assert!(response.header("link").is_none());
    worker.join().expect("test server thread");
}

#[test]
fn transport_rejects_case_insensitive_duplicate_content_length_end_to_end() {
    let (url, worker) = serve_once(
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\ncontent-length: 2\r\nConnection: close\r\n\r\nOK",
    );
    let request = Request::new(Url::parse(&url).unwrap());

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::ConnectionFailed)
    );
    worker.join().expect("test server thread");
}

#[test]
fn transport_rejects_non_ascii_whitespace_in_content_length_end_to_end() {
    let (url, worker) =
        serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: \xc2\xa02\r\nConnection: close\r\n\r\nOK");
    let request = Request::new(Url::parse(&url).unwrap());

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::ConnectionFailed)
    );
    worker.join().expect("test server thread");
}

#[test]
fn transport_rejects_unsupported_protocol_upgrade_end_to_end() {
    let (url, worker) = serve_once(
        concat!(
            "HTTP/1.1 101 Switching Protocols\r\n",
            "Connection: Upgrade\r\n",
            "Upgrade: websocket\r\n",
            "\r\n"
        )
        .as_bytes(),
    );
    let request = Request::new(Url::parse(&url).unwrap());

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::ConnectionFailed)
    );
    worker.join().expect("test server thread");
}

#[test]
fn transport_rejects_invalid_http_header_field_name_end_to_end() {
    let (url, worker) = serve_once(
        concat!(
            "HTTP/1.1 200 OK\r\n",
            "Bad Header: value\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        )
        .as_bytes(),
    );
    let request = Request::new(Url::parse(&url).unwrap());

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::ConnectionFailed)
    );
    worker.join().expect("test server thread");
}

#[test]
fn transport_rejects_content_length_with_trailing_junk_end_to_end() {
    let (url, worker) = serve_once(
        concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Length: 2x\r\n",
            "Connection: close\r\n",
            "\r\nOK"
        )
        .as_bytes(),
    );
    let request = Request::new(Url::parse(&url).unwrap());

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::ConnectionFailed)
    );
    worker.join().expect("test server thread");
}

#[test]
fn transport_rejects_transfer_encoding_and_content_length_together_end_to_end() {
    let (url, worker) = serve_once(
        concat!(
            "HTTP/1.1 200 OK\r\n",
            "Transfer-Encoding: chunked\r\n",
            "Content-Length: 2\r\n",
            "Connection: close\r\n",
            "\r\n2\r\nOK\r\n0\r\n\r\n"
        )
        .as_bytes(),
    );
    let request = Request::new(Url::parse(&url).unwrap());

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::ConnectionFailed)
    );
    worker.join().expect("test server thread");
}

#[test]
fn transport_rejects_response_with_too_many_headers_end_to_end() {
    let mut response = b"HTTP/1.1 200 OK\r\n".to_vec();
    // The transport's response-header count cap is 256. Include two standard
    // headers as well, so the response crosses the cap without approaching the
    // independent byte-size limit.
    for index in 0..255 {
        response.extend_from_slice(format!("X-Test-{index}: value\r\n").as_bytes());
    }
    response.extend_from_slice(b"Content-Length: 0\r\nConnection: close\r\n\r\n");

    let (url, worker) = serve_once_owned(response);
    let request = Request::new(Url::parse(&url).expect("valid test URL"));

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::ResponseTooLarge)
    );
    worker.join().expect("test server thread should finish");
}

#[test]
fn transport_rejects_synthesized_headers_that_exceed_request_header_count() {
    let mut request = Request::new(Url::parse("http://127.0.0.1:1/").expect("valid test URL"));
    for index in 0..255 {
        request = request.with_header(format!("X-Test-{index}"), "value");
    }

    // 255 caller headers plus synthesized Host and Connection fields exceed
    // the 256-field wire limit. Validation must fail before opening a socket.
    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::InvalidRequest)
    );
}

#[test]
fn request_content_length_must_match_body_before_opening_a_connection() {
    let request = Request::new(Url::parse("http://127.0.0.1:1/upload").unwrap())
        .with_method(archia_browser::net::HttpMethod::Post)
        .with_header("Content-Length", "4")
        .with_body(b"abc".to_vec());

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::InvalidRequest)
    );
}

#[test]
fn mixed_case_transfer_encoding_is_rejected_before_opening_a_connection() {
    let request = Request::new(Url::parse("http://127.0.0.1:1/upload").unwrap())
        .with_method(archia_browser::net::HttpMethod::Post)
        .with_header("TrAnSfEr-EnCoDiNg", "chunked")
        .with_body(b"abc".to_vec());

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::InvalidRequest)
    );
}

#[test]
fn oversized_request_header_block_is_rejected_before_opening_a_connection() {
    let request = Request::new(Url::parse("http://127.0.0.1:1/").unwrap())
        .with_header("X-Oversized", "x".repeat(64 * 1024));

    assert_eq!(
        HttpTransport::new().send(&request),
        Err(TransportError::InvalidRequest)
    );
}

