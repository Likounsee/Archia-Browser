use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    scheme: String,
    authority: String,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

impl Url {
    pub fn parse(input: &str) -> Result<Self, UrlError> {
        let (scheme, remainder) = input.split_once("://").ok_or(UrlError::MissingScheme)?;
        if scheme.is_empty()
            || !scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            return Err(UrlError::InvalidScheme);
        }
        let authority_end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
        let authority = &remainder[..authority_end];
        if authority.is_empty() {
            return Err(UrlError::MissingAuthority);
        }
        let rest = &remainder[authority_end..];
        let (without_fragment, fragment) = rest
            .split_once('#')
            .map_or((rest, None), |(before, after)| {
                (before, Some(after.to_owned()))
            });
        let (path, query) = without_fragment
            .split_once('?')
            .map_or((without_fragment, None), |(p, q)| (p, Some(q.to_owned())));
        Ok(Self {
            scheme: scheme.to_ascii_lowercase(),
            authority: authority.to_owned(),
            path: if path.is_empty() {
                "/".into()
            } else {
                path.into()
            },
            query,
            fragment,
        })
    }

    pub fn scheme(&self) -> &str {
        &self.scheme
    }
    pub fn authority(&self) -> &str {
        &self.authority
    }

    pub fn host(&self) -> &str {
        self.authority
            .strip_prefix('[')
            .and_then(|value| value.split_once(']').map(|(host, _)| host))
            .or_else(|| {
                self.authority.rsplit_once(':').map(|(host, port)| {
                    if port.chars().all(|c| c.is_ascii_digit()) {
                        host
                    } else {
                        self.authority.as_str()
                    }
                })
            })
            .unwrap_or(&self.authority)
    }

    pub fn port(&self) -> Option<u16> {
        if self.authority.starts_with('[') {
            return self
                .authority
                .split_once("]:")
                .and_then(|(_, port)| port.parse().ok());
        }
        self.authority
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse().ok())
    }

    pub fn effective_port(&self) -> Option<u16> {
        self.port().or(match self.scheme.as_str() {
            "http" => Some(80),
            "https" => Some(443),
            _ => None,
        })
    }

    pub fn is_secure(&self) -> bool {
        matches!(self.scheme.as_str(), "https" | "wss")
    }
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }
    pub fn fragment(&self) -> Option<&str> {
        self.fragment.as_deref()
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://{}{}", self.scheme, self.authority, self.path)?;
        if let Some(query) = &self.query {
            write!(f, "?{query}")?;
        }
        if let Some(fragment) = &self.fragment {
            write!(f, "#{fragment}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlError {
    MissingScheme,
    InvalidScheme,
    MissingAuthority,
}

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MissingScheme => "URL has no scheme",
            Self::InvalidScheme => "URL scheme is invalid",
            Self::MissingAuthority => "URL has no authority",
        })
    }
}

impl std::error::Error for UrlError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_components() {
        let url = Url::parse("https://example.org/a?q=1#top").unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.authority(), "example.org");
        assert_eq!(url.path(), "/a");
        assert_eq!(url.query(), Some("q=1"));
        assert_eq!(url.fragment(), Some("top"));
        assert_eq!(url.host(), "example.org");
        assert_eq!(url.port(), None);
        assert_eq!(url.effective_port(), Some(443));
        assert!(url.is_secure());
    }

    #[test]
    fn parses_explicit_port() {
        let url = Url::parse("http://example.org:8080/").unwrap();
        assert_eq!(url.host(), "example.org");
        assert_eq!(url.port(), Some(8080));
        assert_eq!(url.effective_port(), Some(8080));
        assert!(!url.is_secure());
    }

    #[test]
    fn defaults_empty_path() {
        assert_eq!(
            Url::parse("https://example.org").unwrap().to_string(),
            "https://example.org/"
        );
    }
}
