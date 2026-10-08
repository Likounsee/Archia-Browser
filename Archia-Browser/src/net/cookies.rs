use crate::net::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CookieJar {
    cookies: Vec<Cookie>,
}

impl CookieJar {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn store(&mut self, url: &Url, set_cookie: &str) {
        let mut parts = set_cookie.split(';').map(str::trim);
        let Some(pair) = parts.next() else {
            return;
        };
        let Some((name, value)) = pair.split_once('=') else {
            return;
        };
        let name = name.trim();
        if name.is_empty() || !valid_cookie_name(name) {
            return;
        }
        let value = value.trim();
        if !valid_cookie_value(value) {
            return;
        }

        let mut cookie = Cookie {
            name: name.to_owned(),
            value: value.to_owned(),
            domain: url.host().to_ascii_lowercase(),
            path: "/".to_owned(),
            secure: false,
        };

        for attribute in parts {
            let mut pieces = attribute.splitn(2, '=');
            let key = pieces
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            match key.as_str() {
                "domain" => {
                    if let Some(value) = pieces.next() {
                        let domain = value.trim().trim_start_matches('.').to_ascii_lowercase();
                        let host = url.host().to_ascii_lowercase();
                        if domain.is_empty()
                            || !(host == domain || host.ends_with(&format!(".{domain}")))
                        {
                            return;
                        }
                        cookie.domain = domain;
                    }
                }
                "path" => {
                    if let Some(value) = pieces.next() {
                        let value = value.trim();
                        if value.starts_with('/') {
                            cookie.path = value.to_owned();
                        }
                    }
                }
                "max-age" => {
                    if pieces
                        .next()
                        .and_then(|value| value.trim().parse::<i64>().ok())
                        .is_some_and(|seconds| seconds <= 0)
                    {
                        self.cookies.retain(|existing| {
                            !(existing.name == cookie.name
                                && existing.domain == cookie.domain
                                && existing.path == cookie.path)
                        });
                        return;
                    }
                }
                "secure" => cookie.secure = true,
                _ => {}
            }
        }

        self.cookies.retain(|existing| {
            !(existing.name == cookie.name
                && existing.domain == cookie.domain
                && existing.path == cookie.path)
        });
        self.cookies.push(cookie);
    }

    fn valid_cookie_name(value: &str) -> bool {
    value.bytes().all(|byte| {
        byte.is_ascii_graphic()
            && !matches!(
                byte,
                b'(' | b')'
                    | b'<'
                    | b'>'
                    | b'@'
                    | b','
                    | b';'
                    | b':'
                    | b'\\'
                    | b'"'
                    | b'/'
                    | b'['
                    | b']'
                    | b'?'
                    | b'='
                    | b'{'
                    | b'}'
            )
    })
}

    fn valid_cookie_value(value: &str) -> bool {
    !value.bytes().any(|byte| matches!(byte, b'\r' | b'\n' | b';'))
}

    pub fn header_for(&self, url: &Url) -> Option<String> {
        let host = url.host().to_ascii_lowercase();
        let path = url.path();
        let secure = url.is_secure();
        let values = self
            .cookies
            .iter()
            .filter(|cookie| {
                let domain_matches =
                    host == cookie.domain || host.ends_with(&format!(".{}", cookie.domain));
                let path_matches = path == cookie.path
                    || path.starts_with(&(cookie.path.trim_end_matches('/').to_owned() + "/"));
                domain_matches && path_matches && (!cookie.secure || secure)
            })
            .map(|cookie| format!("{}={}", cookie.name, cookie.value))
            .collect::<Vec<_>>();
        (!values.is_empty()).then(|| values.join("; "))
    }

    pub fn len(&self) -> usize {
        self.cookies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_cookie_values_that_could_inject_headers() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();

        jar.store(&url, "sid=ok\r\nX-Injected: yes");
        jar.store(&url, "sid=also;bad");

        assert!(jar.is_empty());
    }

    #[test]
    fn rejects_invalid_cookie_names() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();

        jar.store(&url, "bad name=value");
        jar.store(&url, "bad;name=value");

        assert!(jar.is_empty());
    }

    #[test]
    fn stores_and_matches_scoped_cookies() {
        let url = Url::parse("https://example.org/account/page").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&url, "sid=abc; Path=/account; Secure");
        assert_eq!(jar.header_for(&url).as_deref(), Some("sid=abc"));
        assert_eq!(
            jar.header_for(&Url::parse("https://example.org/other").unwrap()),
            None
        );
        assert_eq!(
            jar.header_for(&Url::parse("http://example.org/account").unwrap()),
            None
        );
    }

    #[test]
    fn rejects_unrelated_cookie_domains() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&url, "sid=abc; Domain=evil.example");
        assert!(jar.is_empty());
    }

    #[test]
    fn accepts_parent_cookie_domains() {
        let url = Url::parse("https://sub.example.org/").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&url, "sid=abc; Domain=example.org");
        assert_eq!(jar.header_for(&url).as_deref(), Some("sid=abc"));
    }

    #[test]
    fn max_age_zero_deletes_cookie() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&url, "sid=one");
        jar.store(&url, "sid=gone; Max-Age=0");
        assert!(jar.header_for(&url).is_none());
    }

    #[test]
    fn secure_cookie_is_not_sent_to_http() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&url, "sid=secure; Secure");
        assert!(jar
            .header_for(&Url::parse("http://example.org/").unwrap())
            .is_none());
    }

    #[test]
    fn replaces_same_cookie_key() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&url, "sid=one");
        jar.store(&url, "sid=two");
        assert_eq!(jar.len(), 1);
        assert_eq!(jar.header_for(&url).as_deref(), Some("sid=two"));
    }
}
