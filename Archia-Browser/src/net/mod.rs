pub mod filter;
pub mod url;

pub use url::{Url, UrlError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub url: Url,
}

impl Request {
    pub fn new(url: Url) -> Self {
        Self { url }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub content_type: Option<String>,
}

impl Response {
    pub const fn new(status: u16) -> Self {
        Self {
            status,
            content_type: None,
        }
    }
}
