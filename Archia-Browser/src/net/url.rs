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
        let mut scheme_chars = scheme.chars();
        let valid_first = scheme_chars
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic());
        if !valid_first
            || !scheme_chars.all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
            })
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
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }
    pub fn fragment(&self) -> Option<&str> {
        self.fragment.as_deref()
    }

    pub fn host(&self) -> &str {
        let authority = self
            .authority
            .rsplit_once('@')
            .map_or(self.authority.as_str(), |(_, host)| host);
        if authority.starts_with('[') {
            authority
                .find(']')
                .map_or(authority, |end| &authority[1..end])
        } else {
            authority
                .rsplit_once(':')
                .map_or(authority, |(host, _)| host)
        }
    }

    pub fn port(&self) -> Option<u16> {
        let authority = self
            .authority
            .rsplit_once('@')
            .map_or(self.authority.as_str(), |(_, host)| host);
        if authority.starts_with('[') {
            let end = authority.find(']')?;
            return authority.get(end + 1..)?.strip_prefix(':')?.parse().ok();
        }
        authority
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse().ok())
    }

    pub fn effective_port(&self) -> u16 {
        self.port()
            .unwrap_or(if self.is_secure() { 443 } else { 80 })
    }

    pub fn is_secure(&self) -> bool {
        self.scheme == "https"
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
    }

    #[test]
    fn rejects_non_alpha_scheme_start() {
        assert_eq!(Url::parse("1http://example.org").unwrap_err(), UrlError::InvalidScheme);
        assert_eq!(Url::parse("http+custom://example.org").unwrap().scheme(), "http+custom");
    }

    #[test]
    fn defaults_empty_path() {
        assert_eq!(
            Url::parse("https://example.org").unwrap().to_string(),
            "https://example.org/"
        );
    }
}
