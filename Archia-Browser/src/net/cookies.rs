use std::time::{Duration, SystemTime};

use crate::net::Url;

const MAX_COOKIE_PAIR_BYTES: usize = 4096;
const MAX_COOKIE_HEADER_BYTES: usize = 8192;
const MAX_COOKIES: usize = 3000;
const MAX_COOKIES_PER_DOMAIN: usize = 180;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub host_only: bool,
    pub expires_at: Option<SystemTime>,
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
        self.remove_expired();
        // Bound the complete attribute string, not just name=value: otherwise
        // a tiny cookie pair can carry a huge Path or Domain allocation.
        if set_cookie.len() > MAX_COOKIE_PAIR_BYTES {
            return;
        }
        let mut parts = set_cookie.split(';').map(str::trim);
        let Some(pair) = parts.next() else {
            return;
        };
        if pair.len() > MAX_COOKIE_PAIR_BYTES {
            return;
        }
        let Some((name, value)) = pair.split_once('=') else {
            return;
        };
        let name = name.trim();
        if name.is_empty() || !Self::valid_cookie_name(name) {
            return;
        }
        let value = value.trim();
        if !Self::valid_cookie_value(value) {
            return;
        }

        let mut cookie = Cookie {
            name: name.to_owned(),
            value: value.to_owned(),
            domain: url.host().to_ascii_lowercase(),
            path: default_cookie_path(url.path()),
            secure: false,
            host_only: true,
            expires_at: None,
        };
        let mut max_age = None;
        let mut expires_attribute = None;
        let mut path_attribute_set = false;

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
                        let raw_domain = value.trim();
                        // RFC 6265 ignores one leading dot, but accepting
                        // repeated dots after trimming them all is too lenient.
                        let domain = raw_domain
                            .strip_prefix('.')
                            .unwrap_or(raw_domain)
                            .to_ascii_lowercase();
                        let host = url.host().to_ascii_lowercase();
                        // A trailing dot makes the Domain attribute invalid.
                        // Also require a host boundary so "example.com" cannot
                        // set Domain=com and leak cookies to unrelated hosts.
                        if domain.is_empty()
                            || domain.starts_with('.')
                            || domain.ends_with('.')
                            || is_public_suffix(&domain)
                            || !cookie_domain_matches(&host, &domain)
                        {
                            return;
                        }
                        cookie.domain = domain;
                        cookie.host_only = false;
                    }
                }
                "path" => {
                    if let Some(value) = pieces.next() {
                        let value = value.trim();
                        if value.starts_with('/') {
                            cookie.path = value.to_owned();
                            path_attribute_set = true;
                        }
                    }
                }
                "expires" => {
                    if let Some(value) = pieces.next() {
                        expires_attribute = httpdate::parse_http_date(value.trim()).ok();
                    }
                }
                "max-age" => {
                    if let Some(seconds) = pieces
                        .next()
                        .and_then(|value| value.trim().parse::<i64>().ok())
                    {
                        max_age = Some(seconds);
                    }
                }
                "secure" => cookie.secure = true,
                // Ignore extension attributes (including HttpOnly and
                // SameSite) instead of rejecting an otherwise valid cookie.
                _ => {}
            }
        }

        if max_age.is_some_and(|seconds| seconds <= 0) {
            // Cookie identity depends on the fully parsed Domain and Path, so
            // apply deletion only after all attributes have been processed.
            // An insecure origin also cannot delete an existing Secure cookie.
            self.cookies.retain(|existing| {
                let same_key = existing.name == cookie.name
                    && existing.domain == cookie.domain
                    && existing.path == cookie.path;
                !same_key || (!url.is_secure() && existing.secure)
            });
            return;
        }
        if let Some(seconds) = max_age.filter(|seconds| *seconds > 0) {
            // An overflowing Max-Age must not silently turn a persistent
            // cookie into a session cookie (expires_at = None).
            let Some(expires_at) =
                SystemTime::now().checked_add(Duration::from_secs(seconds as u64))
            else {
                return;
            };
            cookie.expires_at = Some(expires_at);
        } else if let Some(expires_at) = expires_attribute {
            if expires_at <= SystemTime::now() {
                // Expires is a deletion signal when Max-Age is absent or
                // invalid. Preserve Secure cookies against insecure deletion.
                self.cookies.retain(|existing| {
                    let same_key = existing.name == cookie.name
                        && existing.domain == cookie.domain
                        && existing.path == cookie.path;
                    !same_key || (!url.is_secure() && existing.secure)
                });
                return;
            }
            cookie.expires_at = Some(expires_at);
        }

        // An insecure origin must not be able to plant a Secure cookie that
        // will later be sent over HTTPS. This prevents HTTP interception from
        // overwriting or shadowing a security-sensitive HTTPS cookie.
        if cookie.secure && !url.is_secure() {
            return;
        }

        // Enforce the cookie prefixes used by browsers to prevent insecure
        // origins and sibling subdomains from replacing sensitive cookies.
        if cookie.name.starts_with("__Secure-") && (!cookie.secure || !url.is_secure()) {
            return;
        }
        if cookie.name.starts_with("__Host-")
            && (!cookie.secure
                || !url.is_secure()
                || !cookie.host_only
                || cookie.path != "/"
                || !path_attribute_set)
        {
            return;
        }

        // Plain HTTP cookies must not overwrite an HTTPS Secure cookie with
        // the same storage key, even when the incoming cookie omits Secure.
        if !url.is_secure()
            && self.cookies.iter().any(|existing| {
                existing.secure
                    && existing.name == cookie.name
                    && domains_overlap(&existing.domain, &cookie.domain)
                    // Reject overlapping paths too: otherwise an HTTP cookie
                    // at "/" could shadow a Secure cookie scoped to "/account"
                    // (or vice versa) when both are sent on the same request.
                    && (cookie_path_matches(&existing.path, &cookie.path)
                        || cookie_path_matches(&cookie.path, &existing.path))
            })
        {
            return;
        }

        self.cookies.retain(|existing| {
            !(existing.name == cookie.name
                && existing.domain == cookie.domain
                && existing.path == cookie.path)
        });
        if self.cookies.len() >= MAX_COOKIES
            || self
                .cookies
                .iter()
                .filter(|existing| existing.domain == cookie.domain)
                .count()
                >= MAX_COOKIES_PER_DOMAIN
        {
            return;
        }
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
        // RFC 6265 cookie-octet excludes controls, whitespace, quotes,
        // commas, semicolons, and backslashes. Quoted values may wrap the
        // same restricted octets, but quotes are not allowed inside them.
        let value = if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            &value[1..value.len() - 1]
        } else {
            value
        };

        value.bytes().all(
            |byte| matches!(byte, 0x21 | 0x23..=0x2B | 0x2D..=0x3A | 0x3C..=0x5B | 0x5D..=0x7E),
        )
    }

    pub fn header_for(&self, url: &Url) -> Option<String> {
        let now = SystemTime::now();
        let host = url.host().to_ascii_lowercase();
        let path = url.path();
        let secure = url.is_secure();
        let mut matching = self
            .cookies
            .iter()
            .filter(|cookie| {
                if cookie
                    .expires_at
                    .is_some_and(|expires_at| expires_at <= now)
                {
                    return false;
                }
                let domain_matches = if cookie.host_only {
                    host == cookie.domain
                } else {
                    cookie_domain_matches(&host, &cookie.domain)
                };
                let path_matches = cookie_path_matches(path, &cookie.path);
                domain_matches && path_matches && (!cookie.secure || secure)
            })
            .collect::<Vec<_>>();
        // RFC 6265 sends longer, more-specific paths first. Keep insertion
        // order stable for cookies whose paths have equal lengths.
        matching.sort_by(|first, second| second.path.len().cmp(&first.path.len()));
        // Bound the request header independently from cookie count. A jar
        // can contain many individually valid cookies, but emitting all of
        // them can create multi-megabyte request headers.
        let mut header = String::new();
        for cookie in matching {
            let pair_len = cookie.name.len() + 1 + cookie.value.len();
            let separator_len = usize::from(!header.is_empty()) * 2;
            if header.len() + separator_len + pair_len > MAX_COOKIE_HEADER_BYTES {
                continue;
            }
            if !header.is_empty() {
                header.push_str("; ");
            }
            header.push_str(&cookie.name);
            header.push('=');
            header.push_str(&cookie.value);
        }
        (!header.is_empty()).then_some(header)
    }

    fn remove_expired(&mut self) {
        let now = SystemTime::now();
        self.cookies.retain(|cookie| {
            !cookie
                .expires_at
                .is_some_and(|expires_at| expires_at <= now)
        });
    }

    pub fn len(&self) -> usize {
        self.cookies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }
}

fn is_public_suffix(domain: &str) -> bool {
    // Fail closed if the PSL cannot classify the supplied domain. Without
    // this check, Domain=co.uk on shop.example.co.uk would leak a cookie to
    // unrelated registrants under the same public suffix. The PSL crate
    // includes both ICANN and PRIVATE sections, matching browser cookie scope.
    match psl::suffix(domain.as_bytes()) {
        Some(suffix) => suffix.as_bytes() == domain.as_bytes(),
        None => true,
    }
}

fn cookie_domain_matches(host: &str, domain: &str) -> bool {
    if host == domain {
        return true;
    }

    // IP addresses are not DNS suffixes. Treating dotted IPv4 addresses as
    // domain names would let a cookie scoped to "0.0.1" match "127.0.0.1".
    if host.parse::<std::net::IpAddr>().is_ok() || domain.parse::<std::net::IpAddr>().is_ok() {
        return false;
    }

    domain.contains('.') && host.ends_with(&format!(".{domain}"))
}

fn domains_overlap(first: &str, second: &str) -> bool {
    cookie_domain_matches(first, second) || cookie_domain_matches(second, first)
}

fn cookie_path_matches(request_path: &str, cookie_path: &str) -> bool {
    request_path == cookie_path
        || (request_path.starts_with(cookie_path)
            && (cookie_path.ends_with('/')
                || request_path.as_bytes().get(cookie_path.len()) == Some(&b'/')))
}

fn default_cookie_path(request_path: &str) -> String {
    if !request_path.starts_with('/') || request_path.matches('/').count() <= 1 {
        return "/".to_owned();
    }

    request_path
        .rsplit_once('/')
        .map(|(directory, _)| {
            if directory.is_empty() {
                "/".to_owned()
            } else {
                directory.to_owned()
            }
        })
        .unwrap_or_else(|| "/".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_request_header_is_bounded_even_with_many_cookies() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();

        for index in 0..MAX_COOKIES_PER_DOMAIN {
            jar.store(&url, &format!("cookie{index}={}", "x".repeat(100)));
        }

        let header = jar.header_for(&url).unwrap();
        assert!(header.len() <= MAX_COOKIE_HEADER_BYTES);
        assert!(header.starts_with("cookie"));
        assert!(header.contains("; "));
    }

    #[test]
    fn rejects_cookie_values_that_could_inject_headers() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();

        jar.store(&url, "sid=ok\r\nX-Injected: yes");

        assert!(jar.is_empty());
    }

    #[test]
    fn rejects_cookie_values_with_whitespace_or_invalid_quoted_octets() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();

        jar.store(&url, "space=not allowed");
        jar.store(&url, "tab=not\tallowed");
        jar.store(&url, "quoted=\"bad\\value\"");

        assert!(jar.is_empty());
    }

    #[test]
    fn accepts_valid_quoted_cookie_values() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();

        jar.store(&url, "quoted=\"safe-value_123\"");

        assert_eq!(
            jar.header_for(&url).as_deref(),
            Some("quoted=\"safe-value_123\"")
        );
    }

    #[test]
    fn rejects_oversized_cookie_attributes() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();
        let oversized_path = format!("sid=x; Path=/{}", "a".repeat(MAX_COOKIE_PAIR_BYTES));

        jar.store(&url, &oversized_path);

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
    fn expired_expires_attribute_deletes_existing_cookie() {
        let url = Url::parse("https://example.org/account").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&url, "sid=old; Path=/");

        jar.store(
            &url,
            "sid=expired; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
        );

        assert!(jar.header_for(&url).is_none());
        assert!(jar.is_empty());
    }

    #[test]
    fn insecure_expired_cookie_cannot_delete_secure_cookie() {
        let https = Url::parse("https://example.org/").unwrap();
        let http = Url::parse("http://example.org/").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&https, "sid=secure; Secure; Path=/");

        jar.store(
            &http,
            "sid=expired; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
        );

        assert_eq!(jar.header_for(&https).as_deref(), Some("sid=secure"));
    }

    #[test]
    fn max_age_takes_precedence_over_expires() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();

        jar.store(
            &url,
            "sid=live; Max-Age=3600; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
        );

        assert_eq!(jar.header_for(&url).as_deref(), Some("sid=live"));
        assert!(jar.cookies[0].expires_at.is_some());
    }

    #[test]
    fn host_only_cookie_does_not_cross_to_subdomains() {
        let url = Url::parse("https://example.org/account").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&url, "sid=host-only");

        assert_eq!(jar.header_for(&url).as_deref(), Some("sid=host-only"));
        assert_eq!(
            jar.header_for(&Url::parse("https://sub.example.org/account").unwrap()),
            None
        );
    }

    #[test]
    fn rejects_public_and_private_suffix_cookie_domains() {
        let mut public_suffix_jar = CookieJar::new();
        let public_suffix_host = Url::parse("https://shop.example.co.uk/").unwrap();
        public_suffix_jar.store(&public_suffix_host, "sid=bad; Domain=co.uk");
        assert!(public_suffix_jar.is_empty());

        let mut private_suffix_jar = CookieJar::new();
        let private_suffix_host = Url::parse("https://alice.github.io/").unwrap();
        private_suffix_jar.store(&private_suffix_host, "sid=bad; Domain=github.io");
        assert!(private_suffix_jar.is_empty());
    }

    #[test]
    fn accepts_registrable_domain_cookie_scope() {
        let url = Url::parse("https://shop.example.co.uk/").unwrap();
        let mut jar = CookieJar::new();

        jar.store(&url, "sid=shared; Domain=example.co.uk");

        assert_eq!(
            jar.header_for(&Url::parse("https://cdn.example.co.uk/").unwrap()).as_deref(),
            Some("sid=shared")
        );
    }

    #[test]
    fn explicit_domain_cookie_reaches_subdomains() {
        let url = Url::parse("https://example.org/account").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&url, "sid=shared; Domain=example.org");

        assert_eq!(
            jar.header_for(&Url::parse("https://sub.example.org/account").unwrap())
                .as_deref(),
            Some("sid=shared")
        );
    }

    #[test]
    fn omitted_path_uses_request_directory() {
        let url = Url::parse("https://example.org/account/page").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&url, "sid=directory");

        assert_eq!(
            jar.header_for(&Url::parse("https://example.org/account/next").unwrap())
                .as_deref(),
            Some("sid=directory")
        );
        assert_eq!(
            jar.header_for(&Url::parse("https://example.org/other").unwrap()),
            None
        );
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
    fn cookie_path_matching_respects_slash_boundaries() {
        let origin = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&origin, "double=slash; Path=//");
        jar.store(&origin, "account=path; Path=/account");

        assert_eq!(
            jar.header_for(&Url::parse("https://example.org/accounting").unwrap())
                .as_deref(),
            None
        );
        assert_eq!(
            jar.header_for(&Url::parse("https://example.org/account/page").unwrap())
                .as_deref(),
            Some("account=path")
        );
        assert_eq!(
            jar.header_for(&Url::parse("https://example.org//nested").unwrap())
                .as_deref(),
            Some("double=slash")
        );
    }

    #[test]
    fn sends_longer_cookie_paths_before_root_path() {
        let root = Url::parse("https://example.org/").unwrap();
        let account = Url::parse("https://example.org/account/page").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&root, "sid=root; Path=/");
        jar.store(&account, "sid=scoped; Path=/account");

        assert_eq!(
            jar.header_for(&account).as_deref(),
            Some("sid=scoped; sid=root")
        );
    }

    #[test]
    fn accepts_cookies_with_http_only_and_same_site_attributes() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();

        jar.store(&url, "sid=abc; HttpOnly; SameSite=Lax; Secure");

        assert_eq!(jar.header_for(&url).as_deref(), Some("sid=abc"));
    }

    #[test]
    fn rejects_unrelated_cookie_domains() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&url, "sid=abc; Domain=evil.example");
        assert!(jar.is_empty());
    }

    #[test]
    fn rejects_single_label_parent_domains() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();

        jar.store(&url, "sid=bad; Domain=org");
        jar.store(&url, "sid=bad; Domain=com");

        assert!(jar.is_empty());
    }

    #[test]
    fn ip_cookie_domains_do_not_use_dns_suffix_matching() {
        let url = Url::parse("https://127.0.0.1/").unwrap();
        let mut jar = CookieJar::new();

        jar.store(&url, "sid=bad; Domain=0.0.1");
        assert!(jar.is_empty());

        jar.store(&url, "sid=exact; Domain=127.0.0.1");
        assert_eq!(jar.header_for(&url).as_deref(), Some("sid=exact"));
        assert_eq!(
            jar.header_for(&Url::parse("https://127.0.0.2/").unwrap()),
            None
        );
    }

    #[test]
    fn accepts_exact_single_label_host_domain() {
        let url = Url::parse("https://localhost/").unwrap();
        let mut jar = CookieJar::new();

        jar.store(&url, "sid=local; Domain=localhost");

        assert_eq!(jar.header_for(&url).as_deref(), Some("sid=local"));
    }

    #[test]
    fn ignores_one_leading_dot_in_domain_attribute() {
        let url = Url::parse("https://sub.example.org/").unwrap();
        let mut jar = CookieJar::new();

        jar.store(&url, "sid=valid; Domain=.example.org");

        assert_eq!(jar.header_for(&url).as_deref(), Some("sid=valid"));
        assert_eq!(
            jar.header_for(&Url::parse("https://other.example.org/").unwrap())
                .as_deref(),
            Some("sid=valid")
        );
    }

    #[test]
    fn rejects_repeated_or_trailing_dots_in_domain_attribute() {
        let url = Url::parse("https://sub.example.org/").unwrap();
        let mut jar = CookieJar::new();

        jar.store(&url, "repeated=bad; Domain=..example.org");
        jar.store(&url, "trailing=bad; Domain=example.org.");

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
    fn overflowing_max_age_does_not_create_a_session_cookie() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();

        jar.store(&url, "sid=too-far; Max-Age=9223372036854775807");

        assert!(jar.is_empty());
    }

    #[test]
    fn positive_max_age_expires_cookie() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&url, "short=life; Max-Age=1");
        assert_eq!(jar.header_for(&url).as_deref(), Some("short=life"));

        std::thread::sleep(Duration::from_millis(1100));

        assert_eq!(jar.header_for(&url), None);
        jar.store(&url, "fresh=value");
        assert_eq!(
            jar.len(),
            1,
            "expired cookies must be pruned before storage"
        );
    }

    #[test]
    fn last_valid_max_age_attribute_takes_precedence() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&url, "sid=alive; Max-Age=0; Max-Age=60");
        assert_eq!(jar.header_for(&url).as_deref(), Some("sid=alive"));
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
    fn max_age_deletion_uses_path_parsed_after_max_age() {
        let url = Url::parse("https://example.org/account/page").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&url, "sid=one; Path=/");

        jar.store(&url, "sid=gone; Max-Age=0; Path=/");

        assert!(jar.header_for(&url).is_none());
    }

    #[test]
    fn insecure_origin_cannot_overwrite_secure_cookie_without_secure_attribute() {
        let secure_url = Url::parse("https://example.org/").unwrap();
        let insecure_url = Url::parse("http://example.org/").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&secure_url, "sid=trusted; Secure");

        jar.store(&insecure_url, "sid=attacker");

        assert_eq!(jar.header_for(&secure_url).as_deref(), Some("sid=trusted"));
    }

    #[test]
    fn insecure_origin_cannot_shadow_secure_cookie_with_overlapping_path() {
        let secure_url = Url::parse("https://example.org/account/profile").unwrap();
        let insecure_root = Url::parse("http://example.org/").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&secure_url, "sid=trusted; Path=/account; Secure");

        jar.store(&insecure_root, "sid=attacker; Path=/");

        assert_eq!(jar.header_for(&secure_url).as_deref(), Some("sid=trusted"));
        assert_eq!(
            jar.header_for(&Url::parse("https://example.org/other").unwrap()),
            None
        );
    }

    #[test]
    fn insecure_origin_cannot_shadow_root_secure_cookie_with_narrow_path() {
        let secure_url = Url::parse("https://example.org/account/profile").unwrap();
        let insecure_url = Url::parse("http://example.org/account/").unwrap();
        let mut jar = CookieJar::new();
        jar.store(
            &Url::parse("https://example.org/").unwrap(),
            "sid=trusted; Path=/; Secure",
        );

        jar.store(&insecure_url, "sid=attacker; Path=/account");

        assert_eq!(jar.header_for(&secure_url).as_deref(), Some("sid=trusted"));
    }

    #[test]
    fn insecure_subdomain_cannot_shadow_parent_domain_secure_cookie() {
        let secure_url = Url::parse("https://example.org/").unwrap();
        let insecure_subdomain = Url::parse("http://sub.example.org/").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&secure_url, "sid=trusted; Domain=example.org; Secure");

        jar.store(&insecure_subdomain, "sid=attacker");

        assert_eq!(jar.header_for(&insecure_subdomain).as_deref(), None);
        assert_eq!(jar.header_for(&secure_url).as_deref(), Some("sid=trusted"));
    }

    #[test]
    fn insecure_origin_cannot_delete_existing_secure_cookie() {
        let secure_url = Url::parse("https://example.org/").unwrap();
        let insecure_url = Url::parse("http://example.org/").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&secure_url, "sid=trusted; Secure");

        jar.store(&insecure_url, "sid=gone; Max-Age=0");

        assert_eq!(jar.header_for(&secure_url).as_deref(), Some("sid=trusted"));
    }

    #[test]
    fn rejects_secure_cookie_set_by_insecure_origin() {
        let url = Url::parse("http://example.org/").unwrap();
        let mut jar = CookieJar::new();

        jar.store(&url, "sid=attacker; Secure");

        assert!(jar.is_empty());
        assert!(jar
            .header_for(&Url::parse("https://example.org/").unwrap())
            .is_none());
    }

    #[test]
    fn insecure_origin_cannot_overwrite_existing_secure_cookie() {
        let secure_url = Url::parse("https://example.org/").unwrap();
        let insecure_url = Url::parse("http://example.org/").unwrap();
        let mut jar = CookieJar::new();
        jar.store(&secure_url, "sid=trusted; Secure");

        jar.store(&insecure_url, "sid=attacker; Secure");

        assert_eq!(jar.header_for(&secure_url).as_deref(), Some("sid=trusted"));
    }

    #[test]
    fn enforces_secure_and_host_cookie_prefixes() {
        let https = Url::parse("https://example.org/account").unwrap();
        let http = Url::parse("http://example.org/").unwrap();
        let mut jar = CookieJar::new();

        jar.store(&http, "__Secure-sid=bad; Secure");
        jar.store(&https, "__Secure-sid=good");
        jar.store(&https, "__Host-sid=missing-secure; Path=/");
        jar.store(&https, "__Host-sid=missing-path");
        jar.store(
            &https,
            "__Host-sid=domain; Path=/; Secure; Domain=example.org",
        );
        assert_eq!(jar.len(), 0);

        jar.store(&https, "__Secure-sid=good; Secure");
        jar.store(
            &Url::parse("https://example.org/").unwrap(),
            "__Host-sid=host; Secure; Path=/",
        );
        assert_eq!(
            jar.header_for(&Url::parse("https://example.org/").unwrap())
                .as_deref(),
            Some("__Secure-sid=good; __Host-sid=host")
        );
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
    fn rejects_oversized_cookie_pairs_and_caps_per_domain_storage() {
        let url = Url::parse("https://example.org/").unwrap();
        let mut jar = CookieJar::new();

        jar.store(
            &url,
            &format!("large={}", "x".repeat(MAX_COOKIE_PAIR_BYTES)),
        );
        assert!(jar.is_empty());

        for index in 0..=MAX_COOKIES_PER_DOMAIN {
            jar.store(&url, &format!("cookie{index}=value"));
        }
        assert_eq!(jar.len(), MAX_COOKIES_PER_DOMAIN);
    }

    #[test]
    fn cookie_jar_enforces_global_storage_cap_across_domains() {
        let mut jar = CookieJar::new();

        for domain in 0..(MAX_COOKIES / MAX_COOKIES_PER_DOMAIN + 2) {
            let url = Url::parse(&format!("https://host{domain}.example.org/")).unwrap();
            for index in 0..MAX_COOKIES_PER_DOMAIN {
                jar.store(&url, &format!("cookie{index}=value"));
            }
        }

        assert_eq!(jar.len(), MAX_COOKIES);
        assert!(jar.len() <= MAX_COOKIES);
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
