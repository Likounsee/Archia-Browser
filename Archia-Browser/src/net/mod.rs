use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

pub mod cache;
pub mod cookies;
pub mod filter;
pub mod loader;
pub mod pipeline;
pub mod pool;
pub mod transport;
pub mod url;

pub use cache::HttpCache;
pub use cookies::{Cookie, CookieJar};
pub use loader::{DocumentLoadError, DocumentLoader};
pub use transport::{HttpTransport, LocalFileTransport};
pub use url::{Url, UrlError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Head,
    Post,
    Put,
    Delete,
    Patch,
    Options,
    Connect,
}

impl HttpMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Patch => "PATCH",
            Self::Options => "OPTIONS",
            Self::Connect => "CONNECT",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: HttpMethod,
    pub url: Url,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub policy: pipeline::RequestPolicy,
}
impl Request {
    pub fn new(url: Url) -> Self {
        Self {
            method: HttpMethod::Get,
            url,
            headers: BTreeMap::new(),
            body: Vec::new(),
            policy: pipeline::RequestPolicy::default(),
        }
    }
    pub fn with_method(mut self, method: HttpMethod) -> Self {
        self.method = method;
        self
    }
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers
            .insert(name.into().to_ascii_lowercase(), value.into());
        self
    }
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    pub fn has_body(&self) -> bool {
        !self.body.is_empty()
    }

    pub fn with_cookies(mut self, jar: &CookieJar) -> Self {
        if let Some(value) = jar.header_for(&self.url) {
            self = self.with_header("cookie", value);
        }
        self
    }
    pub fn with_policy(mut self, policy: pipeline::RequestPolicy) -> Self {
        self.policy = policy;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub content_type: Option<String>,
    set_cookies: Vec<String>,
}
impl Response {
    pub fn new(status: u16) -> Self {
        Self {
            status,
            headers: BTreeMap::new(),
            body: Vec::new(),
            content_type: None,
            set_cookies: Vec::new(),
        }
    }
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into().to_ascii_lowercase();
        let value = value.into();
        if name == "content-type" {
            self.content_type = Some(value.clone());
        }
        if name == "set-cookie" {
            self.set_cookies.push(value.clone());
        }
        self.headers.insert(name, value);
        self
    }
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    pub fn set_cookie_headers(&self) -> &[String] {
        &self.set_cookies
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    UnsupportedScheme,
    InvalidUrl(UrlError),
    InvalidRequest,
    /// The request was rejected by the browser's request policy before transport.
    BlockedByPolicy,
    ConnectionFailed,
    Timeout,
    TlsFailed,
    ResponseTooLarge,
    Cancelled,
}
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub fn check(&self) -> Result<(), TransportError> {
        if self.is_cancelled() {
            Err(TransportError::Cancelled)
        } else {
            Ok(())
        }
    }
}
impl PartialEq for CancellationToken {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for CancellationToken {}

pub trait Transport {
    fn send(&self, request: &Request) -> Result<Response, TransportError>;
    fn send_cancellable(
        &self,
        request: &Request,
        cancellation: &CancellationToken,
    ) -> Result<Response, TransportError> {
        cancellation.check()?;
        let response = self.send(request)?;
        cancellation.check()?;
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_token_clones_share_state() {
        let token = CancellationToken::new();
        let clone = token.clone();
        assert!(!clone.is_cancelled());
        token.cancel();
        assert!(clone.is_cancelled());
        assert!(matches!(clone.check(), Err(TransportError::Cancelled)));
    }

    #[test]
    fn builds_http_request() {
        let request = Request::new(Url::parse("https://example.org/").unwrap())
            .with_method(HttpMethod::Post)
            .with_header("Content-Type", "text/plain")
            .with_body(b"hello".to_vec());
        assert_eq!(request.method, HttpMethod::Post);
        assert_eq!(
            request.headers.get("content-type"),
            Some(&"text/plain".to_string())
        );
        assert_eq!(request.body, b"hello");
    }
    #[test]
    fn headers_are_case_insensitive() {
        let request = Request::new(Url::parse("https://example.org/").unwrap())
            .with_header("Content-Type", "text/plain");
        assert_eq!(request.header("CONTENT-TYPE"), Some("text/plain"));

        let response = Response::new(200).with_header("Content-Type", "text/html");
        assert_eq!(response.header("content-type"), Some("text/html"));
    }
}

#[cfg(test)]
mod response_cookie_tests {
    use super::*;

    #[test]
    fn preserves_multiple_set_cookie_headers() {
        let response = Response::new(200)
            .with_header("set-cookie", "a=1; Path=/")
            .with_header("set-cookie", "b=2; Path=/");

        assert_eq!(
            response.set_cookie_headers(),
            &["a=1; Path=/", "b=2; Path=/"]
        );
    }
}
