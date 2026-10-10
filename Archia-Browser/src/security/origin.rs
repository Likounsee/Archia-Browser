use crate::net::Url;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_OPAQUE_ORIGIN_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    scheme: String,
    host: String,
    port: Option<u16>,
    opaque_id: Option<u64>,
}

impl Origin {
    pub fn from_url(url: &Url) -> Self {
        Self {
            scheme: url.scheme().to_ascii_lowercase(),
            host: url.host().to_ascii_lowercase(),
            port: url.port(),
            // Local-file URLs must not all collapse to the same tuple origin.
            // Give each constructed file origin a distinct identity; clones
            // retain that identity, while independently created origins do not.
            opaque_id: (url.scheme() == "file")
                .then(|| NEXT_OPAQUE_ORIGIN_ID.fetch_add(1, Ordering::Relaxed)),
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
        if self.opaque_id.is_some() || other.opaque_id.is_some() {
            return self.opaque_id.is_some() && self.opaque_id == other.opaque_id;
        }

        self.scheme == other.scheme
            && self.host == other.host
            && self.effective_port() == other.effective_port()
    }

    fn effective_port(&self) -> Option<u16> {
        self.port.or(match self.scheme.as_str() {
            "http" => Some(80),
            "https" => Some(443),
            _ => None,
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
    fn file_urls_have_distinct_opaque_origins() {
        let first_url = Url::parse("file:///private/first.html").unwrap();
        let second_url = Url::parse("file:///private/second.html").unwrap();
        let first = Origin::from_url(&first_url);
        let second = Origin::from_url(&second_url);
        let first_clone = first.clone();

        assert!(!first.same_origin(&second));
        assert!(first.same_origin(&first_clone));
    }

    #[test]
    fn non_http_origins_distinguish_missing_port_from_port_zero() {
        let implicit = Origin::from_url(&Url::parse("custom://example.org/resource").unwrap());
        let explicit_zero =
            Origin::from_url(&Url::parse("custom://example.org:0/resource").unwrap());

        assert!(!implicit.same_origin(&explicit_zero));
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

    #[test]
    fn origin_comparison_matrix_covers_scheme_host_port_and_opaque_files() {
        let cases = [
            ("http://example.com/a", "http://example.com/b", true),
            ("http://example.com/", "http://example.com:80/", true),
            ("https://example.com/", "https://example.com:443/", true),
            ("HTTP://EXAMPLE.COM/a", "http://example.com/b", true),
            ("https://example.com/", "https://EXAMPLE.COM:443/x", true),
            ("https://[2001:db8::1]/", "https://[2001:0db8:0:0:0:0:0:1]/x", true),
            ("custom://example.com/a", "custom://example.com/b", true),
            ("http://user@example.com/a", "http://example.com/b", true),
            ("http://example.com:8080/", "http://example.com:8080/x", true),
            ("https://example.com:444/", "https://example.com:444/x", true),
            ("http://127.0.0.1/", "http://127.0.0.1:80/x", true),
            ("https://[::1]/", "https://[0:0:0:0:0:0:0:1]:443/x", true),
            ("http://example.com/", "https://example.com/", false),
            ("http://example.com/", "http://other.example/", false),
            ("http://example.com/", "http://sub.example.com/", false),
            ("http://example.com:80/", "http://example.com:81/", false),
            ("https://example.com:443/", "https://example.com:444/", false),
            ("http://example.com:0/", "http://example.com/", false),
            ("custom://example.com/", "custom://example.com:0/", false),
            ("http://127.0.0.1/", "http://127.0.0.2/", false),
            ("https://[2001:db8::1]/", "https://[2001:db8::2]/", false),
            ("http://example.com/", "custom://example.com/", false),
            ("file:///private/a.html", "file:///private/a.html", false),
            ("file:///private/a.html", "file:///private/b.html", false),
            ("file://localhost/private/a.html", "file:///private/a.html", false),
            ("file:///C:/Users/a.html", "file:///C:/Users/a.html", false),
            ("http://example.com:8080/", "http://example.com:8081/", false),
            ("https://example.com/", "https://example.org/", false),
            ("custom://example.com:1/", "custom://example.com:2/", false),
            ("http://example.com/", "http://example.com:443/", false),
        ];

        assert_eq!(cases.len(), 30);
        for (left, right, expected) in cases {
            let left = Origin::from_url(&Url::parse(left).unwrap());
            let right = Origin::from_url(&Url::parse(right).unwrap());
            assert_eq!(
                left.same_origin(&right),
                expected,
                "unexpected origin comparison for {left:?} and {right:?}"
            );
        }
    }

}
