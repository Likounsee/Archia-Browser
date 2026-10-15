use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use archia_browser::net::{
    CancellationToken, HttpTransport, Request, Transport, TransportError, Url,
};

fn serve_once(response: &'static [u8]) -> (String, thread::JoinHandle<()>) {
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
        stream.write_all(response).expect("write test response");
        stream.flush().expect("flush test response");
    });
    (format!("http://{address}/"), worker)
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
    let (url, worker) = serve_once(
        b"HTTP/1.1 200 OK\r\nConnection: Content-Length\r\nContent-Length: 0\r\n\r\n",
    );
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

