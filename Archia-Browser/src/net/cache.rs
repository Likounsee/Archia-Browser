use std::time::{Duration, Instant};

use crate::core::cache::ResourceCache;

use super::{HttpMethod, Request, Response};

const DEFAULT_CAPACITY: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
struct CachedResponse {
    response: Response,
    expires_at: Instant,
}

#[derive(Debug)]
pub struct HttpCache {
    entries: ResourceCache<CachedResponse>,
}

impl Default for HttpCache {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl HttpCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: ResourceCache::new(capacity),
        }
    }

    pub fn get(&mut self, request: &Request) -> Option<Response> {
        let key = cache_key(request);
        if request_forces_cache_bypass(request) {
            // We cannot revalidate yet, so don't leave an older response
            // available for a later request after an explicit bypass.
            self.entries.remove(&key);
            return None;
        }
        if !request_can_use_cache(request) {
            return None;
        }
        let expired = self
            .entries
            .get(&key)
            .is_some_and(|entry| Instant::now() >= entry.expires_at);
        if expired {
            self.entries.remove(&key);
            return None;
        }

        self.entries.get(&key).map(|entry| entry.response.clone())
    }

    pub fn store(&mut self, request: &Request, response: &Response) {
        let key = cache_key(request);
        if !request_can_store(request) {
            // Explicit cache-bypass directives invalidate the old entry, but
            // a credentialed request must not evict an unrelated public copy.
            if request_forces_cache_bypass(request) {
                self.entries.remove(&key);
            }
            return;
        }
        if !response_can_store(response) {
            // A response that cannot be stored must not leave an older
            // representation available for a subsequent cache lookup.
            self.entries.remove(&key);
            return;
        }

        let Some(max_age) = response_max_age(response) else {
            self.entries.remove(&key);
            return;
        };
        let Some(age) = response_age(response) else {
            self.entries.remove(&key);
            return;
        };
        let remaining_age = max_age.saturating_sub(age);
        if remaining_age == 0 {
            self.entries.remove(&key);
            return;
        }
        // Cache-Control and Age are untrusted network input. Avoid overflowing
        // Instant when a server advertises an unrealistically large max-age.
        let Some(expires_at) = Instant::now().checked_add(Duration::from_secs(remaining_age))
        else {
            self.entries.remove(&key);
            return;
        };

        let bytes = response.body.len()
            + response
                .headers
                .iter()
                .map(|(name, value)| name.len() + value.len())
                .sum::<usize>()
            + 64;

        self.entries.insert(
            key,
            CachedResponse {
                response: response.clone(),
                expires_at,
            },
            bytes,
        );
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

fn cache_key(request: &Request) -> String {
    format!("{} {}", request.method.as_str(), request.url)
}

fn request_forces_cache_bypass(request: &Request) -> bool {
    request
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("cache-control"))
        .any(|(_, value)| {
            has_cache_directive(Some(value), "no-cache")
                || has_cache_directive(Some(value), "no-store")
                || has_cache_max_age_zero(Some(value))
        })
        || has_pragma_no_cache(request)
}

fn request_can_use_cache(request: &Request) -> bool {
    matches!(request.method, HttpMethod::Get)
        && matches!(request.url.scheme(), "http" | "https")
        && !has_request_header(request, "authorization")
        && !has_request_header(request, "cookie")
        && !request
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("cache-control"))
            .any(|(_, value)| {
                has_cache_directive(Some(value), "no-cache")
                    || has_cache_directive(Some(value), "no-store")
                    || has_cache_max_age_zero(Some(value))
            })
        && !has_pragma_no_cache(request)
}

fn request_can_store(request: &Request) -> bool {
    matches!(request.method, HttpMethod::Get)
        && matches!(request.url.scheme(), "http" | "https")
        && !has_request_header(request, "authorization")
        && !has_request_header(request, "cookie")
        && !request
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("cache-control"))
            .any(|(_, value)| {
                has_cache_directive(Some(value), "no-store")
                    || has_cache_directive(Some(value), "no-cache")
            })
        && !has_pragma_no_cache(request)
}

fn has_request_header(request: &Request, wanted: &str) -> bool {
    request
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case(wanted))
}

fn response_has_header(response: &Response, wanted: &str) -> bool {
    response
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case(wanted))
}

fn response_can_store(response: &Response) -> bool {
    response.status == 200
        && response.set_cookie_headers().is_empty()
        && !response_has_header(response, "set-cookie")
        && !response
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("cache-control"))
            .any(|(_, value)| {
                has_cache_directive(Some(value), "no-store")
                    || has_cache_directive(Some(value), "no-cache")
            })
        && !response
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("pragma"))
            .any(|(_, value)| {
                value
                    .split(',')
                    .any(|d| d.trim().eq_ignore_ascii_case("no-cache"))
            })
        && !response_has_header(response, "vary")
}

fn cache_control_directives(value: &str) -> Vec<&str> {
    let bytes = value.as_bytes();
    let mut directives = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;

    for (index, byte) in bytes.iter().copied().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && byte == b'\\\\' {
            escaped = true;
        } else if byte == b'"' {
            quoted = !quoted;
        } else if byte == b',' && !quoted {
            directives.push(value[start..index].trim());
            start = index + 1;
        }
    }
    directives.push(value[start..].trim());
    directives
}

fn response_max_age(response: &Response) -> Option<u64> {
    let mut max_age = None;
    for (_, value) in response
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("cache-control"))
    {
        for directive in cache_control_directives(value) {
            let (name, value) = match directive.split_once('=') {
                Some((name, value)) => (name.trim(), Some(value.trim())),
                None => (directive, None),
            };
            if !name.eq_ignore_ascii_case("max-age") {
                continue;
            }
            // Duplicate or malformed freshness directives are ambiguous. Do
            // not guess which lifetime the origin intended.
            if max_age.is_some() {
                return None;
            }
            let value = value?;
            let value = if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
                &value[1..value.len() - 1]
            } else {
                value
            };
            max_age = Some(value.parse::<u64>().ok()?);
        }
    }
    max_age
}

fn response_age(response: &Response) -> Option<u64> {
    let mut age_headers = response
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("age"));
    let Some((_, value)) = age_headers.next() else {
        return Some(0);
    };
    if age_headers.next().is_some() {
        return None;
    }
    value.trim().parse().ok()
}

fn has_cache_max_age_zero(header: Option<&str>) -> bool {
    header.is_some_and(|value| {
        cache_control_directives(value).into_iter().any(|directive| {
            let Some((name, value)) = directive.split_once('=') else {
                return false;
            };
            name.trim().eq_ignore_ascii_case("max-age")
                && value.trim().trim_matches('"').parse::<u64>().ok() == Some(0)
        })
    })
}

fn has_cache_directive(header: Option<&str>, wanted: &str) -> bool {
    header.is_some_and(|value| {
        value.split(',').any(|directive| {
            let name = directive
                .split_once('=')
                .map_or(directive, |(name, _)| name);
            name.trim().eq_ignore_ascii_case(wanted)
        })
    })
}

fn has_pragma_no_cache(request: &Request) -> bool {
    request
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("pragma"))
        .any(|(_, value)| {
            value
                .split(',')
                .any(|d| d.trim().eq_ignore_ascii_case("no-cache"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::Url;

    fn make_request(url: &str) -> Request {
        Request::new(Url::parse(url).unwrap())
    }

    #[test]
    fn caches_fresh_get_responses_with_max_age() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/index.html");
        let response = Response::new(200)
            .with_header("cache-control", "public, max-age=60")
            .with_body(b"cached".to_vec());

        cache.store(&request, &response);

        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&request).unwrap().body, b"cached");
    }

    #[test]
    fn ignores_unrepresentable_cache_max_age_without_panicking() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/resource");
        let response =
            Response::new(200).with_header("cache-control", format!("max-age={}", u64::MAX));

        cache.store(&request, &response);

        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn does_not_cache_cookie_or_vary_responses() {
        let mut cache = HttpCache::default();
        let mut request = make_request("https://example.org/");
        request.headers.insert("cookie".into(), "sid=1".into());
        let response = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_header("vary", "accept-language");

        cache.store(&request, &response);
        assert_eq!(cache.len(), 0);

        request.headers.remove("cookie");
        cache.store(
            &request,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_header("set-cookie", "sid=1"),
        );
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn no_store_and_no_cache_bypass_reuse() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/");
        let response = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(b"cached".to_vec());
        cache.store(&request, &response);

        let no_cache = request.clone().with_header("cache-control", "no-cache");
        assert!(cache.get(&no_cache).is_none());
        let no_store = request.with_header("cache-control", "no-store");
        assert!(cache.get(&no_store).is_none());
    }

    #[test]
    fn request_max_age_zero_bypasses_and_evicts_cached_response() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/resource");
        cache.store(
            &request,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(b"old".to_vec()),
        );
        assert_eq!(cache.len(), 1);

        let forced_refresh = request.clone().with_header("cache-control", "max-age=0");
        assert!(cache.get(&forced_refresh).is_none());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn request_max_age_nonzero_does_not_force_bypass() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/resource");
        cache.store(
            &request,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(b"cached".to_vec()),
        );

        let request = request.with_header("cache-control", "max-age=30");
        assert_eq!(cache.get(&request).unwrap().body, b"cached");
    }

    #[test]
    fn explicit_bypass_evicts_previously_cached_response() {
        for header in [
            ("cache-control", "no-cache"),
            ("cache-control", "no-store"),
            ("pragma", "no-cache"),
        ] {
            let mut cache = HttpCache::default();
            let request = make_request("https://example.org/resource");
            cache.store(
                &request,
                &Response::new(200)
                    .with_header("cache-control", "max-age=60")
                    .with_body(b"old".to_vec()),
            );
            assert_eq!(cache.len(), 1);

            let bypass = request.clone().with_header(header.0, header.1);
            assert!(cache.get(&bypass).is_none());
            assert_eq!(cache.len(), 0, "bypass {header:?} should evict old entry");
        }
    }

    #[test]
    fn recognizes_parameterized_no_cache_directives() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/");
        let response = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(b"cached".to_vec());
        cache.store(&request, &response);

        let conditional = request
            .clone()
            .with_header("cache-control", "no-cache=\"etag\"");
        assert!(cache.get(&conditional).is_none());

        let mut cache = HttpCache::default();
        cache.store(
            &request,
            &Response::new(200).with_header("cache-control", "max-age=60, no-cache=\"etag\""),
        );
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn pragma_no_cache_prevents_cache_reuse_and_storage() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/");
        let response = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(b"cached".to_vec());
        cache.store(&request, &response);

        let pragma_request = request.clone().with_header("pragma", "no-cache");
        assert!(cache.get(&pragma_request).is_none());

        let mut cache = HttpCache::default();
        cache.store(
            &request,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_header("pragma", "no-cache"),
        );
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn does_not_cache_non_http_or_non_success_responses() {
        let mut cache = HttpCache::default();
        let request = Request::new(Url::parse("file:///tmp/index.html").unwrap());
        cache.store(
            &request,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(b"file".to_vec()),
        );
        assert_eq!(cache.len(), 0);

        let request = make_request("https://example.org/");
        cache.store(
            &request,
            &Response::new(404).with_header("cache-control", "max-age=60"),
        );
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn mixed_case_credentials_never_enter_or_read_the_cache() {
        for sensitive_header in ["Authorization", "aUtHoRiZaTiOn", "Cookie", "cOoKiE"] {
            let mut cache = HttpCache::default();
            let public_request = make_request("https://example.org/private");
            let public_response = Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(b"public".to_vec());
            cache.store(&public_request, &public_response);
            assert_eq!(cache.len(), 1);

            let mut private_request = public_request.clone();
            private_request
                .headers
                .insert(sensitive_header.to_owned(), "secret".to_owned());
            assert!(cache.get(&private_request).is_none(), "{sensitive_header}");
            cache.store(&private_request, &public_response);
            assert_eq!(cache.len(), 1, "{sensitive_header}");
        }
    }

    #[test]
    fn mixed_case_response_headers_preserve_cache_safety() {
        for (name, value) in [
            ("Set-Cookie", "sid=private"),
            ("CACHE-Control", "no-store, max-age=60"),
            ("Cache-Control", "no-cache, max-age=60"),
            ("Vary", "Accept-Language"),
            ("Pragma", "no-cache"),
        ] {
            let mut cache = HttpCache::default();
            let request = make_request("https://example.org/resource");
            let mut response = Response::new(200).with_body(b"body".to_vec());
            response.headers.insert(name.to_owned(), value.to_owned());
            cache.store(&request, &response);
            assert_eq!(cache.len(), 0, "{name}: {value}");
        }
    }

    #[test]
    fn rejects_duplicate_or_malformed_response_max_age() {
        let request = make_request("https://example.org/resource");
        for headers in [
            vec![("cache-control", "max-age=60, max-age=120")],
            vec![("cache-control", "max-age=bad, max-age=60")],
            vec![("cache-control", "max-age, max-age=60")],
            vec![
                ("cache-control", "public, max-age=60"),
                ("Cache-Control", "max-age=120"),
            ],
            vec![("cache-control", "max-age=")],
            vec![("cache-control", "max-age=-1")],
        ] {
            let mut cache = HttpCache::default();
            let mut response = Response::new(200);
            for (name, value) in headers {
                response.headers.insert(name.to_owned(), value.to_owned());
            }

            cache.store(&request, &response);

            assert_eq!(
                cache.len(),
                0,
                "ambiguous Cache-Control: {:?}",
                response.headers
            );
        }
    }

    #[test]
    fn ignores_commas_inside_quoted_cache_control_extensions() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/resource");
        let response = Response::new(200)
            .with_header("cache-control", r#"extension="text, max-age=0", max-age=60"#)
            .with_body(b"cached".to_vec());

        cache.store(&request, &response);

        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&request).unwrap().body, b"cached");
    }

    #[test]
    fn accepts_a_single_quoted_response_max_age() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/resource");
        let response = Response::new(200).with_header("cache-control", "public, max-age=\"60\"");

        cache.store(&request, &response);

        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn response_age_reduces_remaining_cache_freshness() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/resource");
        let response = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_header("age", "59")
            .with_body(b"nearly stale".to_vec());

        cache.store(&request, &response);

        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&request).unwrap().body, b"nearly stale");
    }

    #[test]
    fn response_at_or_beyond_max_age_is_not_cached() {
        for age in ["60", "120"] {
            let mut cache = HttpCache::default();
            let request = make_request("https://example.org/resource");
            let response = Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_header("Age", age);

            cache.store(&request, &response);

            assert_eq!(cache.len(), 0, "Age {age} must exhaust max-age");
        }
    }

    #[test]
    fn malformed_or_duplicate_age_headers_prevent_caching() {
        let request = make_request("https://example.org/resource");
        for age in ["unknown", "-1", "1, 2"] {
            let mut cache = HttpCache::default();
            let response = Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_header("age", age);
            cache.store(&request, &response);
            assert_eq!(cache.len(), 0, "invalid Age: {age}");
        }

        let mut cache = HttpCache::default();
        let mut response = Response::new(200).with_header("cache-control", "max-age=60");
        response.headers.insert("Age".into(), "1".into());
        response.headers.insert("age".into(), "2".into());
        cache.store(&request, &response);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn unstorable_replacement_evicts_previous_cached_response() {
        let request = make_request("https://example.org/resource");
        let cached = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(b"old".to_vec());

        for response in [
            Response::new(200).with_header("cache-control", "no-store, max-age=60"),
            Response::new(200).with_header("cache-control", "no-cache, max-age=60"),
            Response::new(200).with_header("set-cookie", "sid=private"),
            Response::new(200).with_header("vary", "accept-language"),
            Response::new(200).with_header("cache-control", "max-age=0"),
            Response::new(200),
        ] {
            let mut cache = HttpCache::default();
            cache.store(&request, &cached);
            assert_eq!(cache.len(), 1);

            cache.store(&request, &response);

            assert_eq!(cache.len(), 0, "unstorable response must evict old data");
            assert!(cache.get(&request).is_none());
        }
    }

    #[test]
    fn request_that_cannot_be_stored_evicts_same_cache_key() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/resource");
        cache.store(
            &request,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(b"old".to_vec()),
        );
        assert_eq!(cache.len(), 1);

        let private_request = request.clone().with_header("cache-control", "no-store");
        cache.store(
            &private_request,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(b"private".to_vec()),
        );

        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn reads_mixed_case_cache_control_max_age() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/resource");
        let mut response = Response::new(200).with_body(b"cached".to_vec());
        response
            .headers
            .insert("Cache-Control".into(), "max-age=60".into());
        cache.store(&request, &response);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&request).unwrap().body, b"cached");
    }
}
