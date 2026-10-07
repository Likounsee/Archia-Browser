use crate::net::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    scheme: String,
    host: String,
    port: Option<u16>,
}

impl Origin {
    pub fn from_url(url: &Url) -> Self {
        let authority = url.authority();
        let (host, port) = authority
            .rsplit_once(':')
            .and_then(|(host, port)| port.parse::<u16>().ok().map(|port| (host, Some(port))))
            .unwrap_or((authority, None));
        Self {
            scheme: url.scheme().to_ascii_lowercase(),
            host: host.to_ascii_lowercase(),
            port,
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
    fn compares_web_origins() {
        let a = Origin::from_url(&Url::parse("https://example.org/a").unwrap());
        let b = Origin::from_url(&Url::parse("https://example.org/b").unwrap());
        let c = Origin::from_url(&Url::parse("http://example.org/b").unwrap());
        assert!(a.same_origin(&b));
        assert!(!a.same_origin(&c));
    }
}
