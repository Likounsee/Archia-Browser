use std::time::{Duration, Instant};

use crate::core::cache::ResourceCache;

use super::{HttpMethod, Request, Response};

const DEFAULT_CAPACITY: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
struct CachedResponse {
    response: Response,
    stored_at: Instant,
    initial_age: u64,
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
        let now = Instant::now();
        let expired = self
            .entries
            .get(&key)
            .is_some_and(|entry| now >= entry.expires_at);
        if expired {
            self.entries.remove(&key);
            return None;
        }

        // A request's max-age is a stricter freshness limit than the
        // origin's lifetime. Do not evict the response: another request may
        // still be allowed to use it under the origin's Cache-Control policy.
        let max_age = match request_cache_age_directive(request, "max-age") {
            Ok(value) => value,
            Err(()) => return None,
        };
        if let Some(max_age) = max_age {
            let too_old_for_request = self.entries.get(&key).is_some_and(|entry| {
                entry
                    .initial_age
                    .saturating_add(now.saturating_duration_since(entry.stored_at).as_secs())
                    > max_age
            });
            if too_old_for_request {
                return None;
            }
        }
        let min_fresh = match request_cache_age_directive(request, "min-fresh") {
            Ok(value) => value,
            Err(()) => return None,
        };
        if let Some(min_fresh) = min_fresh {
            let insufficient_freshness = self.entries.get(&key).is_some_and(|entry| {
                entry.expires_at.saturating_duration_since(now).as_secs() < min_fresh
            });
            if insufficient_freshness {
                return None;
            }
        }

        self.entries.get(&key).map(|entry| entry.response.clone())
    }

    pub fn store(&mut self, request: &Request, response: &Response) {
        let now = Instant::now();
        // Expired entries should not consume the budget or force live entries
        // out of the cache when a new response arrives.
        self.entries.retain(|entry| now < entry.value.expires_at);
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
        let Some(expires_at) = now.checked_add(Duration::from_secs(remaining_age)) else {
            self.entries.remove(&key);
            return;
        };

        // Response data can be constructed by callers as well as received
        // from the bounded network transport. Saturate accounting arithmetic
        // so an extreme size estimate cannot overflow and undercharge the cache.
        let bytes = response
            .headers
            .iter()
            .fold(response.body.len(), |total, (name, value)| {
                total
                    .saturating_add(name.len())
                    .saturating_add(value.len())
            })
            .saturating_add(64);

        self.entries.insert(
            key,
            CachedResponse {
                response: response.clone(),
                stored_at: Instant::now(),
                initial_age: age,
                expires_at,
            },
            bytes,
        );
    }

    /// Returns the number of cached HTTP responses.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the HTTP cache contains no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.len() == 0
    }

    /// Returns the approximate memory budget currently occupied by cached entries.
    ///
    /// This accounts for response bodies, header text, and a small per-entry
    /// overhead; it is a cache budget estimate, not an exact heap measurement.
    pub fn used_bytes(&self) -> usize {
        self.entries.used()
    }

    /// Returns the configured maximum accounted size of the HTTP cache.
    pub fn capacity_bytes(&self) -> usize {
        self.entries.capacity()
    }

    /// Invalidates the cached response associated with a request URL and method.
    ///
    /// Returns whether an entry was removed. This is useful after a resource is
    /// explicitly refreshed or modified by a higher-level browser component.
    pub fn invalidate(&mut self, request: &Request) -> bool {
        self.entries.remove(&cache_key(request)).is_some()
    }

    /// Removes all cached responses and releases the cache's accounted budget.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

fn cache_key(request: &Request) -> String {
    // DNS host names are case-insensitive, and omitting an HTTP(S) default
    // port is equivalent to spelling it explicitly. Normalize both so these
    // URL spellings do not create duplicate entries for the same resource.
    // URL fragments are client-side identifiers and are never sent in an
    // HTTP request target, so they must not participate in the cache key.
    let host = request.url.host().to_ascii_lowercase();
    let authority = if host.contains(':') {
        format!("[{host}]:{}", request.url.effective_port())
    } else {
        format!("{host}:{}", request.url.effective_port())
    };
    let mut key = format!(
        "{} {}://{}{}",
        request.method.as_str(),
        request.url.scheme(),
        authority,
        request.url.path()
    );
    if let Some(query) = request.url.query() {
        key.push('?');
        key.push_str(query);
    }
    key
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
        if quoted && byte == b'\\' {
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

fn request_cache_age_directive(request: &Request, wanted: &str) -> Result<Option<u64>, ()> {
    let mut parsed = None;
    for (_, header) in request
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("cache-control"))
    {
        for directive in cache_control_directives(header) {
            let (name, value) = match directive.split_once('=') {
                Some((name, value)) => (name.trim(), Some(value.trim())),
                None => (directive.trim(), None),
            };
            if !name.eq_ignore_ascii_case(wanted) {
                continue;
            }
            // A malformed or repeated constraint must not silently become
            // "no constraint", which could permit a stale cache hit.
            if parsed.is_some() {
                return Err(());
            }
            let value = value.ok_or(())?;
            let value = if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
                &value[1..value.len() - 1]
            } else {
                value
            };
            parsed = Some(value.parse::<u64>().map_err(|_| ())?);
        }
    }
    Ok(parsed)
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
        cache_control_directives(value)
            .into_iter()
            .any(|directive| {
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
        cache_control_directives(value)
            .into_iter()
            .any(|directive| {
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
    fn cache_management_reports_usage_and_clear_releases_entries() {
        let mut cache = HttpCache::new(4096);
        assert!(cache.is_empty());
        assert_eq!(cache.capacity_bytes(), 4096);
        assert_eq!(cache.used_bytes(), 0);

        let request = make_request("https://example.org/resource");
        cache.store(
            &request,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(b"cached body".to_vec()),
        );

        assert_eq!(cache.len(), 1);
        assert!(!cache.is_empty());
        assert!(cache.used_bytes() > 0);
        assert!(cache.get(&request).is_some());

        cache.clear();

        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.used_bytes(), 0);
        assert!(cache.get(&request).is_none());
    }

    #[test]
    fn invalidate_removes_only_the_requested_cache_entry() {
        let mut cache = HttpCache::new(4096);
        let first = make_request("https://example.org/first");
        let second = make_request("https://example.org/second");
        let response = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(b"cached".to_vec());

        cache.store(&first, &response);
        cache.store(&second, &response);
        assert_eq!(cache.len(), 2);

        assert!(cache.invalidate(&first));
        assert!(!cache.invalidate(&first));
        assert_eq!(cache.len(), 1);
        assert_eq!(
            cache.used_bytes(),
            response.body.len() + 64 + "cache-control".len() + "max-age=60".len()
        );
        assert!(cache.get(&first).is_none());
        assert!(cache.get(&second).is_some());
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
    fn fragment_only_url_changes_reuse_the_same_cached_resource() {
        let mut cache = HttpCache::default();
        let original = make_request("https://example.org/page?lang=fr#top");
        let response = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(b"cached".to_vec());

        cache.store(&original, &response);

        let different_fragment = make_request("https://example.org/page?lang=fr#details");
        assert_eq!(cache.get(&different_fragment).unwrap().body, b"cached");
        assert_eq!(cache.len(), 1);

        let different_query = make_request("https://example.org/page?lang=en#details");
        assert!(cache.get(&different_query).is_none());
    }

    #[test]
    fn equivalent_host_case_and_default_ports_share_cache_entries() {
        for (stored_url, lookup_url) in [
            (
                "https://EXAMPLE.org/resource",
                "https://example.org:443/resource",
            ),
            (
                "http://Example.org:80/resource",
                "http://example.org/resource",
            ),
            (
                "https://[2001:DB8::1]/resource",
                "https://[2001:db8::1]:443/resource",
            ),
        ] {
            let mut cache = HttpCache::default();
            let request = make_request(stored_url);
            cache.store(
                &request,
                &Response::new(200)
                    .with_header("cache-control", "max-age=60")
                    .with_body(b"cached".to_vec()),
            );

            let equivalent = make_request(lookup_url);
            assert_eq!(
                cache.get(&equivalent).unwrap().body,
                b"cached",
                "{stored_url} should match {lookup_url}"
            );
            assert_eq!(cache.len(), 1);
        }
    }

    #[test]
    fn non_default_ports_remain_distinct_cache_entries() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org:8443/resource");
        cache.store(
            &request,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(b"alternate port".to_vec()),
        );

        assert!(cache
            .get(&make_request("https://example.org/resource"))
            .is_none());
        assert_eq!(
            cache
                .get(&make_request("https://example.org:8443/resource"))
                .unwrap()
                .body,
            b"alternate port"
        );
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
    fn request_max_age_limits_reuse_without_evicting_the_entry() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/resource");
        let response = Response::new(200)
            .with_header("cache-control", "max-age=300")
            .with_body(b"cached".to_vec());
        cache.entries.insert(
            cache_key(&request),
            CachedResponse {
                response,
                stored_at: Instant::now() - Duration::from_secs(45),
                initial_age: 0,
                expires_at: Instant::now() + Duration::from_secs(255),
            },
            128,
        );

        let strict = request.clone().with_header("cache-control", "max-age=30");
        assert!(cache.get(&strict).is_none());
        assert_eq!(
            cache.len(),
            1,
            "request freshness limits must not evict the entry"
        );

        let permissive = request.with_header("cache-control", "max-age=60");
        assert_eq!(cache.get(&permissive).unwrap().body, b"cached");
    }

    #[test]
    fn request_max_age_includes_origin_age_header() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/resource");
        let response = Response::new(200)
            .with_header("cache-control", "max-age=300")
            .with_header("age", "45")
            .with_body(b"cached".to_vec());
        cache.store(&request, &response);

        let strict = request.with_header("cache-control", "max-age=30");
        assert!(cache.get(&strict).is_none());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn request_min_fresh_requires_enough_remaining_freshness() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/resource");
        cache.store(
            &request,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(b"cached".to_vec()),
        );

        let acceptable = request.clone().with_header("cache-control", "min-fresh=30");
        assert_eq!(cache.get(&acceptable).unwrap().body, b"cached");

        let too_strict = request.with_header("cache-control", "min-fresh=120");
        assert!(cache.get(&too_strict).is_none());
        assert_eq!(
            cache.len(),
            1,
            "min-fresh must not evict a still-fresh entry"
        );
    }

    #[test]
    fn malformed_or_duplicate_request_freshness_directives_never_use_cache() {
        for directive in [
            "max-age=invalid",
            "max-age",
            "max-age=30, max-age=60",
            "min-fresh=invalid",
            "min-fresh",
            "min-fresh=30, min-fresh=60",
        ] {
            let mut cache = HttpCache::default();
            let request = make_request("https://example.org/resource");
            cache.store(
                &request,
                &Response::new(200)
                    .with_header("cache-control", "max-age=300")
                    .with_body(b"cached".to_vec()),
            );

            let constrained = request.clone().with_header("cache-control", directive);
            assert!(
                cache.get(&constrained).is_none(),
                "malformed freshness directive must not permit a cache hit: {directive}"
            );
            assert_eq!(
                cache.len(),
                1,
                "request constraints must not evict the entry"
            );
        }
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
    fn quoted_cache_control_text_is_not_treated_as_a_no_store_directive() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/resource");
        let response = Response::new(200)
            .with_header(
                "cache-control",
                r#"extension="text, no-store, text", max-age=60"#,
            )
            .with_body(b"cached".to_vec());

        cache.store(&request, &response);

        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn ignores_commas_inside_quoted_cache_control_extensions() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/resource");
        let response = Response::new(200)
            .with_header(
                "cache-control",
                r#"extension="text, max-age=0", max-age=60"#,
            )
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
