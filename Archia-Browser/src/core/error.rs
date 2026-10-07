use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserError {
    InvalidConfiguration(&'static str),
    ResourceLimit(&'static str),
    Unsupported(&'static str),
}

impl fmt::Display for BrowserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(message) => write!(f, "invalid configuration: {message}"),
            Self::ResourceLimit(message) => write!(f, "resource limit: {message}"),
            Self::Unsupported(message) => write!(f, "unsupported: {message}"),
        }
    }
}

impl std::error::Error for BrowserError {}
