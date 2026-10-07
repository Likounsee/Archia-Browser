use std::collections::BTreeMap;

pub mod cookies;
pub mod filter;
pub mod pipeline;
pub mod pool;
pub mod url;

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
}
impl Response {
    pub fn new(status: u16) -> Self {
        Self {
            status,
            headers: BTreeMap::new(),
            body: Vec::new(),
            content_type: None,
        }
    }
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into().to_ascii_lowercase();
        let value = value.into();
        if name == "content-type" {
            self.content_type = Some(value.clone());
        }
        self.headers.insert(name, value);
        self
    }
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    UnsupportedScheme,
    InvalidUrl(UrlError),
    ConnectionFailed,
    Timeout,
    TlsFailed,
}
pub trait Transport {
    fn send(&self, request: &Request) -> Result<Response, TransportError>;
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
