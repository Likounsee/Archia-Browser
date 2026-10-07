use std::collections::BTreeMap;

use super::url::{Url, UrlError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestPriority {
    High,
    Normal,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Document,
    Stylesheet,
    Script,
    Image,
    Font,
    Media,
    Fetch,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestPolicy {
    pub priority: RequestPriority,
    pub resource_kind: ResourceKind,
    pub referrer: Option<Url>,
}

impl Default for RequestPolicy {
    fn default() -> Self {
        Self {
            priority: RequestPriority::Normal,
            resource_kind: ResourceKind::Other,
            referrer: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: super::HttpMethod,
    pub url: Url,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub policy: RequestPolicy,
}

impl Request {
    pub fn new(url: Url) -> Self {
        Self {
            method: super::HttpMethod::Get,
            url,
            headers: BTreeMap::new(),
            body: Vec::new(),
            policy: RequestPolicy::default(),
        }
    }

    pub fn with_method(mut self, method: super::HttpMethod) -> Self {
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

    pub fn with_policy(mut self, policy: RequestPolicy) -> Self {
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
    fn request_policy_is_attached_to_request() {
        let url = Url::parse("https://example.org/app").unwrap();
        let request = Request::new(url.clone()).with_policy(RequestPolicy {
            priority: RequestPriority::High,
            resource_kind: ResourceKind::Document,
            referrer: Some(url),
        });

        assert_eq!(request.policy.priority, RequestPriority::High);
        assert_eq!(request.policy.resource_kind, ResourceKind::Document);
        assert!(request.policy.referrer.is_some());
    }

    #[test]
    fn response_tracks_content_type() {
        let response = Response::new(200).with_header("Content-Type", "text/html");
        assert_eq!(response.content_type.as_deref(), Some("text/html"));
    }
}
