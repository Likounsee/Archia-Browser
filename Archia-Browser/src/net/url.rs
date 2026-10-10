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
        if contains_unsafe_url_whitespace(input) {
            return Err(UrlError::InvalidCharacter);
        }

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
        let normalized_scheme = scheme.to_ascii_lowercase();
        let authority_end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
        let authority = &remainder[..authority_end];
        if authority.is_empty() && normalized_scheme != "file" {
            return Err(UrlError::MissingAuthority);
        }
        if !authority.is_empty() && normalized_scheme != "file" {
            validate_authority(authority)?;
        }
        if normalized_scheme == "file"
            && !authority.is_empty()
            && !authority.eq_ignore_ascii_case("localhost")
        {
            return Err(UrlError::UnsupportedFileAuthority);
        }
        let rest = &remainder[authority_end..];
        if !has_valid_percent_encoding(rest) {
            return Err(UrlError::InvalidCharacter);
        }
        let (without_fragment, fragment) = rest
            .split_once('#')
            .map_or((rest, None), |(before, after)| {
                (before, Some(after.to_owned()))
            });
        let (path, query) = without_fragment
            .split_once('?')
            .map_or((without_fragment, None), |(p, q)| (p, Some(q.to_owned())));
        // Canonicalize dot segments before checking the local-file path:
        // a path such as "/.//server/share" can become a double-leading-slash
        // path only after normalization and may be interpreted as a UNC share
        // by Windows filesystem APIs.
        let normalized_path = normalize_path_for_scheme(
            if path.is_empty() { "/" } else { path },
            &normalized_scheme,
        );
        // The local-file transport percent-decodes paths before filesystem
        // access. Encoded separators could therefore create new path segments
        // after URL policy and dot-segment normalization have already run.
        if normalized_scheme == "file"
            && (normalized_path.starts_with("//")
                || contains_encoded_path_separator(path)
                || contains_encoded_windows_drive_colon(path))
        {
            return Err(UrlError::InvalidCharacter);
        }
        Ok(Self {
            scheme: normalized_scheme,
            authority: normalize_ipv6_authority(authority),
            path: normalized_path,
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

    /// Compare tuple origins for network URLs.
    ///
    /// Local-file URLs are deliberately treated as opaque here: this URL type
    /// has no per-document opaque-origin identity, so equating two file paths
    /// would be an unsafe assumption for security-sensitive decisions.
    pub fn same_origin(&self, other: &Self) -> bool {
        if self.scheme == "file" || other.scheme == "file" {
            return false;
        }

        self.scheme == other.scheme
            && self.host().eq_ignore_ascii_case(other.host())
            && self.origin_port() == other.origin_port()
    }

    pub fn same_document(&self, other: &Self) -> bool {
        self.scheme == other.scheme
            && self.host().eq_ignore_ascii_case(other.host())
            && self.origin_port() == other.origin_port()
            && self.path == other.path
            && self.query == other.query
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
        self.port().unwrap_or(match self.scheme.as_str() {
            "http" => 80,
            "https" => 443,
            // Non-HTTP schemes have no implicit HTTP port.
            _ => 0,
        })
    }

    // Preserve the distinction between an absent port and explicit port 0
    // for origin/document comparisons. The transport separately rejects port 0.
    fn origin_port(&self) -> Option<u16> {
        self.port().or(match self.scheme.as_str() {
            "http" => Some(80),
            "https" => Some(443),
            _ => None,
        })
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
        if contains_unsafe_url_whitespace(reference) {
            return Err(UrlError::InvalidCharacter);
        }

        if reference.contains("://") {
            return Self::parse(reference);
        }

        if has_reference_scheme(reference) {
            return Err(UrlError::UnsupportedReferenceScheme);
        }

        if let Some(authority) = reference.strip_prefix("//") {
            return Self::parse(&format!("{}://{}", self.scheme, authority));
        }
        if !has_valid_percent_encoding(reference) {
            return Err(UrlError::InvalidCharacter);
        }

        let (reference, fragment) = reference
            .split_once('#')
            .map_or((reference, None), |(before, after)| (before, Some(after)));
        let (path, query) = reference
            .split_once('?')
            .map_or((reference, None), |(before, after)| (before, Some(after)));

        // Apply the same file-path rule to relative references; otherwise
        // percent decoding in LocalFileTransport could change path boundaries.
        if self.scheme == "file"
            && (contains_encoded_path_separator(path)
                || contains_encoded_windows_drive_colon(path))
        {
            return Err(UrlError::InvalidCharacter);
        }

        if path.is_empty() {
            let mut resolved = self.clone();
            resolved.query = query.map(str::to_owned).or_else(|| self.query.clone());
            resolved.fragment = fragment.map(str::to_owned);
            return Ok(resolved);
        }

        let resolved_path = if path.starts_with('/') {
            // A root-relative reference inside a Windows file URL stays on
            // the base drive (file:///C:/dir/page + /asset -> /C:/asset).
            let absolute_path = if self.scheme == "file"
                && has_windows_drive_prefix(&self.path)
                && !has_windows_drive_prefix(path)
            {
                format!("{}{}", &self.path[..3], path)
            } else {
                path.to_owned()
            };
            normalize_path_for_scheme(&absolute_path, &self.scheme)
        } else {
            let base = self
                .path
                .rsplit_once('/')
                .map_or("/", |(directory, _)| directory);
            normalize_path_for_scheme(&format!("{base}/{path}"), &self.scheme)
        };
        if self.scheme == "file" && resolved_path.starts_with("//") {
            return Err(UrlError::InvalidCharacter);
        }

        Ok(Self {
            scheme: self.scheme.clone(),
            authority: self.authority.clone(),
            path: resolved_path,
            query: query.map(str::to_owned),
            fragment: fragment.map(str::to_owned),
        })
    }
}

fn normalize_ipv6_authority(authority: &str) -> String {
    let (userinfo, host_port) = authority
        .split_once('@')
        .map_or((None, authority), |(userinfo, host_port)| {
            (Some(userinfo), host_port)
        });
    let Some(bracketed) = host_port.strip_prefix('[') else {
        return authority.to_owned();
    };
    let Some(end) = bracketed.find(']') else {
        return authority.to_owned();
    };
    let Ok(address) = bracketed[..end].parse::<std::net::Ipv6Addr>() else {
        return authority.to_owned();
    };

    let mut normalized = String::new();
    if let Some(userinfo) = userinfo {
        normalized.push_str(userinfo);
        normalized.push('@');
    }
    normalized.push('[');
    normalized.push_str(&address.to_string());
    normalized.push(']');
    normalized.push_str(&bracketed[end + 1..]);
    normalized
}

fn validate_authority(authority: &str) -> Result<(), UrlError> {
    // Split user information exactly once. A second literal '@' is invalid:
    // accepting it can make URL consumers disagree about where the host starts.
    let host_port = if let Some((userinfo, host_port)) = authority.split_once('@') {
        if userinfo.contains('@') || !valid_userinfo(userinfo) {
            return Err(UrlError::InvalidAuthority);
        }
        host_port
    } else {
        authority
    };
    if host_port.is_empty() {
        return Err(UrlError::InvalidAuthority);
    }

    let (host, port, is_ipv6) = if let Some(bracketed) = host_port.strip_prefix('[') {
        let end = bracketed.find(']').ok_or(UrlError::InvalidAuthority)?;
        let host = &bracketed[..end];
        host.parse::<std::net::Ipv6Addr>()
            .map_err(|_| UrlError::InvalidAuthority)?;
        let suffix = &bracketed[end + 1..];
        let port = if suffix.is_empty() {
            None
        } else {
            Some(suffix.strip_prefix(':').ok_or(UrlError::InvalidAuthority)?)
        };
        (host, port, true)
    } else {
        if host_port.contains(['[', ']']) {
            return Err(UrlError::InvalidAuthority);
        }
        let (host, port) = match host_port.matches(':').count() {
            0 => (host_port, None),
            1 => {
                let (host, port) = host_port
                    .rsplit_once(':')
                    .ok_or(UrlError::InvalidAuthority)?;
                (host, Some(port))
            }
            _ => return Err(UrlError::InvalidAuthority),
        };
        (host, port, false)
    };

    if host.is_empty() {
        return Err(UrlError::InvalidAuthority);
    }
    if !is_ipv6 {
        validate_dns_or_ipv4_host(host)?;
    }
    if let Some(port) = port {
        if port.is_empty()
            || !port.bytes().all(|byte| byte.is_ascii_digit())
            || port.parse::<u16>().is_err()
        {
            return Err(UrlError::InvalidAuthority);
        }
    }
    Ok(())
}

fn valid_userinfo(userinfo: &str) -> bool {
    let bytes = userinfo.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return false;
            }
            index += 3;
            continue;
        }
        if !(byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                45 | 46 | 95 | 126 | 33 | 36 | 38 | 39 | 40 | 41 | 42 | 43 | 44 | 59 | 61 | 58
            ))
        {
            return false;
        }
        index += 1;
    }
    true
}

fn validate_dns_or_ipv4_host(host: &str) -> Result<(), UrlError> {
    // This implementation intentionally accepts ASCII DNS names only. IDNs
    // must be converted to their ASCII form (punycode) before parsing.
    if !host.is_ascii() || host.len() > 253 {
        return Err(UrlError::InvalidAuthority);
    }
    if host
        .bytes()
        .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-')))
    {
        return Err(UrlError::InvalidAuthority);
    }

    let without_trailing_dot = host.strip_suffix('.').unwrap_or(host);
    if without_trailing_dot.is_empty() {
        return Err(UrlError::InvalidAuthority);
    }
    let labels: Vec<&str> = without_trailing_dot.split('.').collect();
    if labels.iter().any(|label| {
        label.is_empty()
            || label.len() > 63
            || !label.as_bytes()[0].is_ascii_alphanumeric()
            || !label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
    }) {
        return Err(UrlError::InvalidAuthority);
    }

    // Some system resolvers accept legacy IPv4 spellings such as 127.1,
    // 0177.0.0.1, or a single decimal integer, while URL/origin and policy
    // code treats those spellings as ordinary host names. Accept only strict
    // dotted-decimal IPv4 when the hostname is entirely numeric or uses
    // hexadecimal numeric labels, so all consumers agree on the destination.
    let legacy_numeric_host = labels.iter().all(|label| {
        label.bytes().all(|byte| byte.is_ascii_digit())
            || label
                .strip_prefix("0x")
                .or_else(|| label.strip_prefix("0X"))
                .is_some_and(|hex| {
                    !hex.is_empty() && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
    });
    if legacy_numeric_host && host.parse::<std::net::Ipv4Addr>().is_err() {
        return Err(UrlError::InvalidAuthority);
    }
    Ok(())
}

fn has_valid_percent_encoding(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len()
            || !bytes[index + 1].is_ascii_hexdigit()
            || !bytes[index + 2].is_ascii_hexdigit()
        {
            return false;
        }
        index += 3;
    }
    true
}

fn contains_encoded_windows_drive_colon(path: &str) -> bool {
    let bytes = path.as_bytes();
    // Reject the encoded colon even without a following slash: on Windows,
    // "C:relative" is drive-relative rather than rooted and has different
    // semantics from the URL path the policy layer normalized.
    bytes.len() >= 5
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b'%'
        && bytes[3] == b'3'
        && bytes[4].eq_ignore_ascii_case(&b'a')
}

fn contains_encoded_path_separator(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.windows(3).any(|sequence| {
        sequence[0] == b'%'
            && matches!(
                (
                    sequence[1].to_ascii_lowercase(),
                    sequence[2].to_ascii_lowercase()
                ),
                (b'2', b'f') | (b'5', b'c')
            )
    })
}

fn contains_unsafe_url_whitespace(value: &str) -> bool {
    // Backslashes are interpreted as path separators by several URL consumers
    // (notably browsers and Windows-oriented code), but as ordinary bytes by
    // this parser. Reject them rather than let policy, cache, and transport
    // disagree about the URL being requested.
    value
        .bytes()
        .any(|byte| byte.is_ascii_control() || matches!(byte, b' ' | b'\\'))
}

fn has_reference_scheme(reference: &str) -> bool {
    let first_segment = reference.split(['/', '?', '#']).next().unwrap_or_default();
    let Some((scheme, _)) = first_segment.split_once(':') else {
        return false;
    };

    let mut chars = scheme.chars();
    chars
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic())
        && chars.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
}

fn dot_segment_kind(segment: &str) -> Option<u8> {
    // Special-scheme URLs treat percent-encoded dots as dot segments too.
    // Normalize all accepted hierarchical URLs conservatively so downstream
    // servers and local-file handling cannot reinterpret traversal segments.
    match segment.to_ascii_lowercase().as_str() {
        "." | "%2e" => Some(1),
        ".." | ".%2e" | "%2e." | "%2e%2e" => Some(2),
        _ => None,
    }
}

fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
        && (bytes.len() == 3 || bytes[3] == b'/')
}

fn normalize_path_for_scheme(path: &str, scheme: &str) -> String {
    // Windows file URLs are converted to drive paths by the file transport.
    // Keep the drive prefix rooted while collapsing dot segments so a path
    // above C:/ cannot become /Windows and point at a different filesystem path.
    let has_windows_drive = scheme == "file" && has_windows_drive_prefix(path);

    if has_windows_drive {
        let drive = &path[1..3];
        let remainder = &path[3..];
        let normalized_remainder = normalize_path(if remainder.is_empty() {
            "/"
        } else {
            remainder
        });
        format!("/{drive}{normalized_remainder}")
    } else {
        normalize_path(path)
    }
}

fn normalize_path(path: &str) -> String {
    let mut segments = Vec::new();
    let mut trailing_slash = path.ends_with('/');
    for (index, segment) in path.split('/').enumerate() {
        match dot_segment_kind(segment) {
            None if segment.is_empty() && index == 0 => {}
            None if segment.is_empty() => segments.push(""),
            Some(1) => trailing_slash = true,
            Some(2) => {
                segments.pop();
                trailing_slash = true;
            }
            None => {
                segments.push(segment);
                trailing_slash = false;
            }
            _ => unreachable!("dot segment kind is limited to one or two"),
        }
    }

    let mut result = String::from("/");
    result.push_str(&segments.join("/"));
    if trailing_slash && !result.ends_with('/') {
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
    UnsupportedFileAuthority,
    InvalidCharacter,
    InvalidAuthority,
}

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MissingScheme => "URL has no scheme",
            Self::InvalidScheme => "URL scheme is invalid",
            Self::MissingAuthority => "URL has no authority",
            Self::UnsupportedReferenceScheme => {
                "URL reference uses an unsupported non-hierarchical scheme"
            }
            Self::UnsupportedFileAuthority => "file URL uses an unsupported authority",
            Self::InvalidCharacter => {
                "URL contains disallowed whitespace, control characters, or backslashes"
            }
            Self::InvalidAuthority => "URL authority has an invalid host or port",
        })
    }
}

impl std::error::Error for UrlError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_origin_normalizes_hosts_and_default_ports_but_not_schemes() {
        let cases = [
            ("http://example.org/a", "http://EXAMPLE.org:80/b", true),
            ("https://example.org/a", "https://example.org:443/b", true),
            ("http://example.org/", "https://example.org/", false),
            ("http://example.org/", "http://example.org:8080/", false),
            ("http://[::1]/", "http://[0:0:0:0:0:0:0:1]:80/", true),
            ("file:///tmp/a", "file:///tmp/a", false),
            ("file:///tmp/a", "file:///tmp/b", false),
        ];
        for (left, right, expected) in cases {
            let left = Url::parse(left).unwrap();
            let right = Url::parse(right).unwrap();
            assert_eq!(
                left.same_origin(&right),
                expected,
                "unexpected origin comparison: {left} vs {right}"
            );
        }
    }

    #[test]
    fn parses_local_file_urls_without_authority() {
        let url = Url::parse("file:///tmp/index.html").unwrap();
        assert_eq!(url.scheme(), "file");
        assert_eq!(url.authority(), "");
        assert_eq!(url.path(), "/tmp/index.html");
        assert_eq!(url.to_string(), "file:///tmp/index.html");
    }

    #[test]
    fn file_urls_reject_percent_encoded_windows_drive_colons() {
        for input in [
            "file:///C%3A/../Windows/system.ini",
            "file:///d%3a/Users/example/index.html",
            "file:///C%3A",
            "file:///C%3Arelative/path",
        ] {
            assert_eq!(
                Url::parse(input).unwrap_err(),
                UrlError::InvalidCharacter,
                "encoded drive colon must not change path normalization: {input}"
            );
        }

        let base = Url::parse("file:///C:/Users/example/index.html").unwrap();
        assert_eq!(
            base.resolve("/D%3A/Windows/system.ini").unwrap_err(),
            UrlError::InvalidCharacter
        );
    }

    #[test]
    fn file_url_dot_segments_cannot_escape_a_windows_drive_root() {
        let url = Url::parse("file:///C:/../Windows/system.ini").unwrap();
        assert_eq!(url.path(), "/C:/Windows/system.ini");

        let base = Url::parse("file:///D:/Users/example/index.html").unwrap();
        assert_eq!(
            base.resolve("../../../../Windows/system.ini").unwrap().path(),
            "/D:/Windows/system.ini"
        );
        assert_eq!(
            base.resolve("/E:/folder/../Windows").unwrap().path(),
            "/E:/Windows"
        );
        assert_eq!(
            base.resolve("/assets/app.css").unwrap().path(),
            "/D:/assets/app.css"
        );
    }

    #[test]
    fn rejects_file_paths_that_normalize_into_unc_prefixes() {
        for input in ["file:///.//server/share", "file:///%2e//server/share"] {
            assert_eq!(
                Url::parse(input).unwrap_err(),
                UrlError::InvalidCharacter,
                "normalized UNC-like file path must be rejected: {input}"
            );
        }
    }

    #[test]
    fn rejects_remote_file_authorities() {
        assert_eq!(
            Url::parse("file://example.org/index.html").unwrap_err(),
            UrlError::UnsupportedFileAuthority
        );
        assert_eq!(
            Url::parse("file://localhost/index.html")
                .unwrap()
                .authority(),
            "localhost"
        );
    }

    #[test]
    fn rejects_malformed_percent_escapes_in_all_url_components() {
        for input in [
            "https://example.org/percent%",
            "https://example.org/percent%2",
            "https://example.org/percent%GG",
            "https://example.org/?query=%",
            "https://example.org/#fragment%XZ",
        ] {
            assert_eq!(
                Url::parse(input).unwrap_err(),
                UrlError::InvalidCharacter,
                "malformed percent escape must be rejected: {input}"
            );
        }

        let base = Url::parse("https://example.org/path").unwrap();
        for reference in ["next%", "?query=%GG", "#fragment%2"] {
            assert_eq!(
                base.resolve(reference).unwrap_err(),
                UrlError::InvalidCharacter,
                "malformed relative-reference escape must be rejected: {reference}"
            );
        }

        assert!(Url::parse("https://user%40name@example.org/%2e%2e/safe").is_ok());
    }

    #[test]
    fn rejects_spaces_and_control_characters_in_urls() {
        assert_eq!(
            Url::parse("https://example.org/a b").unwrap_err(),
            UrlError::InvalidCharacter
        );
        assert_eq!(
            Url::parse("https://example.org/\nheader-injection").unwrap_err(),
            UrlError::InvalidCharacter
        );
        let base = Url::parse("https://example.org/path").unwrap();
        assert_eq!(
            base.resolve("next\r\nInjected: yes").unwrap_err(),
            UrlError::InvalidCharacter
        );
    }

    #[test]
    fn rejects_backslashes_that_can_be_interpreted_as_path_separators() {
        for input in [
            "https://example.org\\\\@evil.test/path",
            "https://example.org/a\\\\..\\\\private",
            "https://example.org/a?next=\\\\\\\\evil.test",
        ] {
            assert_eq!(
                Url::parse(input).unwrap_err(),
                UrlError::InvalidCharacter,
                "backslashes must not have consumer-dependent URL semantics: {input}"
            );
        }

        let base = Url::parse("https://example.org/docs/index.html").unwrap();
        assert_eq!(
            base.resolve("..\\\\private").unwrap_err(),
            UrlError::InvalidCharacter
        );
    }

    #[test]
    fn rejects_malformed_authority_ports_and_ipv6_hosts() {
        for input in [
            "https://example.org:abc/path",
            "https://example.org:/path",
            "https://example.org:65536/path",
            "https://[not-an-ipv6-address]/path",
            "https://2001:db8::1/path",
            "https://[]/path",
        ] {
            assert_eq!(
                Url::parse(input).unwrap_err(),
                UrlError::InvalidAuthority,
                "expected malformed authority to be rejected: {input}"
            );
        }
    }

    #[test]
    fn accepts_valid_ipv6_authorities_and_numeric_ports() {
        let ipv6 = Url::parse("https://[2001:db8::1]:8443/path").unwrap();
        assert_eq!(ipv6.host(), "2001:db8::1");
        assert_eq!(ipv6.port(), Some(8443));

        let equivalent = Url::parse("https://[2001:0DB8:0:0:0:0:0:1]:8443/path").unwrap();
        assert_eq!(equivalent.host(), ipv6.host());
        assert_eq!(equivalent.authority(), ipv6.authority());
        assert!(equivalent.same_document(&ipv6));

        let explicit_default = Url::parse("https://example.org:443/path").unwrap();
        assert_eq!(explicit_default.port(), Some(443));
    }

    #[test]
    fn rejects_invalid_hosts_and_ambiguous_userinfo() {
        for input in [
            "https://exa[mple.org/path",
            "https://-invalid.example/path",
            "https://invalid-.example/path",
            "https://example..org/path",
            "https://999.999.999.999/path",
            "https://user@@example.org/path",
            "https://user%ZZ@example.org/path",
        ] {
            assert_eq!(
                Url::parse(input).unwrap_err(),
                UrlError::InvalidAuthority,
                "expected invalid authority to be rejected: {input}"
            );
        }

        assert_eq!(
            Url::parse("https://user name@example.org/path").unwrap_err(),
            UrlError::InvalidCharacter
        );
    }

    #[test]
    fn rejects_legacy_numeric_ipv4_spellings() {
        for input in [
            "http://127.1/",
            "http://2130706433/",
            "http://0177.0.0.1/",
            "http://0x7f000001/",
            "http://0x7f.0.0.1/",
        ] {
            assert_eq!(
                Url::parse(input).unwrap_err(),
                UrlError::InvalidAuthority,
                "ambiguous numeric host must be rejected: {input}"
            );
        }

        assert_eq!(Url::parse("http://127.0.0.1/").unwrap().host(), "127.0.0.1");
    }

    #[test]
    fn accepts_valid_userinfo_and_fully_qualified_hostnames() {
        let url = Url::parse("https://user:pass%40word@example.org.:443/path").unwrap();
        assert_eq!(url.host(), "example.org.");
        assert_eq!(url.port(), Some(443));
    }

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
    fn file_urls_reject_unc_like_double_slash_paths() {
        assert_eq!(
            Url::parse("file:////server/share/secret.txt").unwrap_err(),
            UrlError::InvalidCharacter
        );
        assert_eq!(
            Url::parse("file://localhost//server/share/secret.txt").unwrap_err(),
            UrlError::InvalidCharacter
        );

        let base = Url::parse("file:///safe/index.html").unwrap();
        assert_eq!(
            base.resolve("//server/share/secret.txt").unwrap_err(),
            UrlError::UnsupportedFileAuthority
        );
    }

    #[test]
    fn file_urls_reject_percent_encoded_path_separators() {
        for input in [
            "file:///safe%2f..%2fprivate.txt",
            "file:///safe/%2e%2e%2fprivate.txt",
            "file:///safe%5c..%5cprivate.txt",
            "file:///safe/%5C..%5Cprivate.txt",
        ] {
            assert_eq!(
                Url::parse(input).unwrap_err(),
                UrlError::InvalidCharacter,
                "filesystem decoding must not create new path segments: {input}"
            );
        }

        let base = Url::parse("file:///safe/index.html").unwrap();
        for reference in ["%2f..%2fprivate.txt", "%5c..%5cprivate.txt"] {
            assert_eq!(
                base.resolve(reference).unwrap_err(),
                UrlError::InvalidCharacter,
                "relative file reference must not create decoded separators: {reference}"
            );
        }

        // Encoded separators in the query are not filesystem path syntax.
        assert!(Url::parse("file:///safe/index.html?next=%2fprivate").is_ok());
    }

    #[test]
    fn absolute_urls_normalize_literal_and_encoded_dot_segments() {
        for input in [
            "https://example.org/a/../private",
            "https://example.org/a/%2e%2e/private",
            "https://example.org/a/.%2E/private",
            "https://example.org/a/%2e./private",
            "https://example.org/a/%2E%2e/private",
        ] {
            let url = Url::parse(input).unwrap();
            assert_eq!(
                url.path(),
                "/private",
                "path must have one canonical interpretation: {input}"
            );
        }

        assert_eq!(
            Url::parse("file:///safe/%2e%2e/private.txt")
                .unwrap()
                .path(),
            "/private.txt"
        );
    }

    #[test]
    fn same_document_ignores_only_the_fragment() {
        let first = Url::parse("https://example.org/docs/index.html#top").unwrap();
        let second = Url::parse("https://example.org/docs/index.html#features").unwrap();
        let different_query = Url::parse("https://example.org/docs/index.html?q=1").unwrap();

        assert!(first.same_document(&second));
        assert!(!first.same_document(&different_query));
    }

    #[test]
    fn same_document_normalizes_default_ports_and_host_case() {
        let implicit = Url::parse("https://EXAMPLE.org/page?q=1#top").unwrap();
        let explicit = Url::parse("https://example.ORG:443/page?q=1#next").unwrap();
        let other_port = Url::parse("https://example.org:8443/page?q=1").unwrap();

        assert!(implicit.same_document(&explicit));
        assert!(!implicit.same_document(&other_port));
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
    fn preserves_repeated_slashes_when_resolving_paths() {
        let base = Url::parse("https://example.org/index.html").unwrap();

        assert_eq!(
            base.resolve("/assets//app.js").unwrap().path(),
            "/assets//app.js"
        );
        assert_eq!(
            base.resolve("/assets/a/../app.js").unwrap().path(),
            "/assets/app.js"
        );
        assert_eq!(base.resolve("/assets//").unwrap().path(), "/assets//");
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

    #[test]
    fn file_scheme_validation_is_case_insensitive() {
        let local = Url::parse("FiLe:///tmp/index.html").unwrap();
        assert_eq!(local.scheme(), "file");
        assert_eq!(local.to_string(), "file:///tmp/index.html");

        assert_eq!(
            Url::parse("FILE://example.org/index.html").unwrap_err(),
            UrlError::UnsupportedFileAuthority
        );
    }

    #[test]
    fn custom_schemes_do_not_inherit_http_default_ports() {
        let implicit = Url::parse("custom://example.org/resource").unwrap();
        let explicit_http_port = Url::parse("custom://example.org:80/resource").unwrap();

        assert_eq!(implicit.effective_port(), 0);
        assert_eq!(explicit_http_port.effective_port(), 80);
        assert!(!implicit.same_document(&explicit_http_port));
    }

    #[test]
    fn non_http_document_comparison_distinguishes_missing_port_from_zero() {
        let implicit = Url::parse("custom://example.org/resource").unwrap();
        let explicit_zero = Url::parse("custom://example.org:0/resource").unwrap();

        assert!(!implicit.same_document(&explicit_zero));
    }

    #[test]
    fn same_document_ignores_host_case_but_not_path_or_query() {
        let upper = Url::parse("https://EXAMPLE.org/page?x=1#first").unwrap();
        let lower_same = Url::parse("https://example.org/page?x=1#second").unwrap();
        let different_path = Url::parse("https://example.org/other?x=1").unwrap();
        let different_query = Url::parse("https://example.org/page?x=2").unwrap();

        assert!(upper.same_document(&lower_same));
        assert!(!upper.same_document(&different_path));
        assert!(!upper.same_document(&different_query));
    }
}
