use crate::net::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    scheme: String,
    host: String,
    port: Option<u16>,
}

impl Origin {
    pub fn from_url(url: &Url) -> Self {
        Self {
            scheme: url.scheme().to_ascii_lowercase(),
            host: url.host().to_ascii_lowercase(),
            port: url.port(),
        }
    }

    pub fn scheme(&self) -> &str {
        &self.scheme
    }
    pub fn host(&self) -> &str {
        &self.host
    }
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    pub fn same_origin(&self, other: &Self) -> bool {
        self.scheme == other.scheme
            && self.host == other.host
            && self.effective_port() == other.effective_port()
    }

    fn effective_port(&self) -> u16 {
        self.port.unwrap_or(match self.scheme.as_str() {
            "http" => 80,
            "https" => 443,
            _ => 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::Url;

    #[test]
    fn equivalent_ipv6_spellings_have_the_same_origin() {
        let compressed = Origin::from_url(&Url::parse("https://[2001:db8::1]/a").unwrap());
        let expanded = Origin::from_url(&Url::parse("https://[2001:0DB8:0:0:0:0:0:1]/b").unwrap());

        assert!(compressed.same_origin(&expanded));
        assert_eq!(compressed.host(), "2001:db8::1");
        assert_eq!(expanded.host(), "2001:db8::1");
    }

    #[test]
    fn compares_web_origins() {
        let a = Origin::from_url(&Url::parse("https://example.org/a").unwrap());
        let b = Origin::from_url(&Url::parse("https://example.org/b").unwrap());
        let c = Origin::from_url(&Url::parse("http://example.org/b").unwrap());
        let explicit = Origin::from_url(&Url::parse("https://example.org:443/b").unwrap());
        assert!(a.same_origin(&b));
        assert!(!a.same_origin(&c));
        assert!(a.same_origin(&explicit));
    }
}
