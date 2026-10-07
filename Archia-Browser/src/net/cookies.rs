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
        let Some(pair) = parts.next() else { return };
        let Some((name, value)) = pair.split_once('=') else { return };
        if name.trim().is_empty() { return; }

        let mut cookie = Cookie {
            name: name.trim().to_owned(),
            value: value.trim().to_owned(),
            domain: url.host().to_ascii_lowercase(),
            path: "/".to_owned(),
            secure: false,
        };

        for attribute in parts {
            let mut pieces = attribute.splitn(2, '=');
            let key = pieces.next().unwrap_or_default().trim().to_ascii_lowercase();
            match key.as_str() {
                "domain" => {
                    if let Some(value) = pieces.next() {
                        cookie.domain = value.trim().trim_start_matches('.').to_ascii_lowercase();
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

    pub fn header_for(&self, url: &Url) -> Option<String> {
        let host = url.host().to_ascii_lowercase();
        let path = url.path();
        let secure = url.is_secure();
        let values = self.cookies.iter().filter(|cookie| {
            let domain_matches = host == cookie.domain || host.ends_with(&format!(".{}", cookie.domain));
            let path_matches = path == cookie.path || path.starts_with(&(cookie.path.trim_end_matches('/').to_owned() + "/"));
            domain_matches && path_matches && (!cookie.secure || secure)
        }).map(|cookie| format!("{}={}", cookie.name, cookie.value)).collect::<Vec<_>>();
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
    fn stores_and_matches_scoped_cookies() {
        let url = Url::parse("https://example.org/account/page").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&url, "sid=abc; Path=/account; Secure");
        assert_eq!(jar.header_for(&url).as_deref(), Some("sid=abc"));
        assert_eq!(jar.header_for(&Url::parse("https://example.org/other").unwrap()), None);
        assert_eq!(jar.header_for(&Url::parse("http://example.org/account").unwrap()), None);
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
