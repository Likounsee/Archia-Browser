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

    /// Resolve a navigation reference against this URL.
    ///
    /// This intentionally covers the common HTTP navigation forms used by
    /// redirects and document links while keeping the URL type independent
    /// from the network layer.
    pub fn resolve(&self, reference: &str) -> Result<Self, UrlError> {
        if reference.contains("://") {
            return Self::parse(reference);
        }

        if has_reference_scheme(reference) {
            return Err(UrlError::UnsupportedReferenceScheme);
        }

        if let Some(authority) = reference.strip_prefix("//") {
            return Self::parse(&format!("{}://{}", self.scheme, authority));
        }

        let (reference, fragment) = reference
            .split_once('#')
            .map_or((reference, None), |(before, after)| (before, Some(after)));
        let (path, query) = reference
            .split_once('?')
            .map_or((reference, None), |(before, after)| (before, Some(after)));

        if path.is_empty() {
            let mut resolved = self.clone();
            resolved.query = query.map(str::to_owned).or_else(|| self.query.clone());
            resolved.fragment = fragment.map(str::to_owned);
            return Ok(resolved);
        }

        let resolved_path = if path.starts_with('/') {
            normalize_path(path)
        } else {
            let base = self
                .path
                .rsplit_once('/')
                .map_or("/", |(directory, _)| directory);
            normalize_path(&format!("{base}/{path}"))
        };

        Ok(Self {
            scheme: self.scheme.clone(),
            authority: self.authority.clone(),
            path: resolved_path,
            query: query.map(str::to_owned),
            fragment: fragment.map(str::to_owned),
        })
    }
}

fn has_reference_scheme(reference: &str) -> bool {
    let first_segment = reference
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let Some((scheme, _)) = first_segment.split_once(':') else {
        return false;
    };

    let mut chars = scheme.chars();
    chars.next().is_some_and(|character| character.is_ascii_alphabetic())
        && chars.all(|character| character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.'))
}

fn normalize_path(path: &str) -> String {
    let mut segments = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            value => segments.push(value),
        }
    }

    let mut result = String::from("/");
    result.push_str(&segments.join("/"));
    if path.ends_with('/') && !result.ends_with('/') {
        result.push('/');
    }
    result
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
    UnsupportedReferenceScheme,
}

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MissingScheme => "URL has no scheme",
            Self::InvalidScheme => "URL scheme is invalid",
            Self::MissingAuthority => "URL has no authority",
            Self::UnsupportedReferenceScheme => "URL reference uses an unsupported non-hierarchical scheme",
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
        assert_eq!(
            Url::parse("1http://example.org").unwrap_err(),
            UrlError::InvalidScheme
        );
        assert_eq!(
            Url::parse("http+custom://example.org").unwrap().scheme(),
            "http+custom"
        );
    }

    #[test]
    fn resolves_relative_navigation_references() {
        let base = Url::parse("https://example.org/docs/index.html?old=1#top").unwrap();

        assert_eq!(
            base.resolve("../guide.html").unwrap().to_string(),
            "https://example.org/guide.html"
        );
        assert_eq!(
            base.resolve("/assets/app.css?v=2").unwrap().to_string(),
            "https://example.org/assets/app.css?v=2"
        );
        assert_eq!(
            base.resolve("?next=1").unwrap().to_string(),
            "https://example.org/docs/index.html?next=1"
        );
        assert_eq!(
            base.resolve("#section").unwrap().to_string(),
            "https://example.org/docs/index.html?old=1#section"
        );
        assert_eq!(
            base.resolve("//cdn.example.org/app.js")
                .unwrap()
                .to_string(),
            "https://cdn.example.org/app.js"
        );
    }

    #[test]
    fn resolves_dot_segments_and_trailing_slashes() {
        let base = Url::parse("https://example.org/a/b/index.html").unwrap();

        assert_eq!(
            base.resolve("././next/../guide/").unwrap().to_string(),
            "https://example.org/a/b/guide/"
        );
        assert_eq!(
            base.resolve("/a/../b/./").unwrap().to_string(),
            "https://example.org/b/"
        );
        assert_eq!(
            base.resolve("../").unwrap().to_string(),
            "https://example.org/a/"
        );
    }

    #[test]
    #[test]
    fn rejects_non_hierarchical_scheme_references() {
        let base = Url::parse("https://example.org/docs/index.html").unwrap();

        assert_eq!(
            base.resolve("javascript:alert(1)").unwrap_err(),
            UrlError::UnsupportedReferenceScheme
        );
        assert_eq!(
            base.resolve("mailto:test@example.org").unwrap_err(),
            UrlError::UnsupportedReferenceScheme
        );
        assert_eq!(
            base.resolve("this:that").unwrap_err(),
            UrlError::UnsupportedReferenceScheme
        );
        assert_eq!(
            base.resolve("./this:that").unwrap().to_string(),
            "https://example.org/docs/this:that"
        );
    }

    #[test]
    fn preserves_empty_and_replaced_queries() {
        let base = Url::parse("https://example.org/page?old=1#top").unwrap();

        assert_eq!(
            base.resolve("?").unwrap().to_string(),
            "https://example.org/page?"
        );
        assert_eq!(
            base.resolve("?new=2").unwrap().to_string(),
            "https://example.org/page?new=2"
        );
        assert_eq!(
            base.resolve("#next").unwrap().to_string(),
            "https://example.org/page?old=1#next"
        );
    }

    #[test]
    fn defaults_empty_path() {
        assert_eq!(
            Url::parse("https://example.org").unwrap().to_string(),
            "https://example.org/"
        );
    }
}
