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
        if !request_can_use_cache(request) {
            return None;
        }

        let key = cache_key(request);
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
        if !request_can_store(request) || !response_can_store(response) {
            return;
        }

        let Some(max_age) = response_max_age(response) else {
            return;
        };
        if max_age == 0 {
            return;
        }

        let bytes = response.body.len()
            + response
                .headers
                .iter()
                .map(|(name, value)| name.len() + value.len())
                .sum::<usize>()
            + 64;

        self.entries.insert(
            cache_key(request),
            CachedResponse {
                response: response.clone(),
                expires_at: Instant::now() + Duration::from_secs(max_age),
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

fn request_can_use_cache(request: &Request) -> bool {
    matches!(request.method, HttpMethod::Get)
        && matches!(request.url.scheme(), "http" | "https")
        && request.header("authorization").is_none()
        && request.header("cookie").is_none()
        && !has_cache_directive(request.header("cache-control"), "no-cache")
        && !has_cache_directive(request.header("cache-control"), "no-store")
}

fn request_can_store(request: &Request) -> bool {
    matches!(request.method, HttpMethod::Get)
        && matches!(request.url.scheme(), "http" | "https")
        && request.header("authorization").is_none()
        && request.header("cookie").is_none()
        && !has_cache_directive(request.header("cache-control"), "no-store")
}

fn response_can_store(response: &Response) -> bool {
    response.status == 200
        && response.set_cookie_headers().is_empty()
        && !has_cache_directive(response.header("cache-control"), "no-store")
        && !has_cache_directive(response.header("cache-control"), "no-cache")
        && response.header("vary").is_none()
}

fn response_max_age(response: &Response) -> Option<u64> {
    response
        .header("cache-control")?
        .split(',')
        .map(str::trim)
        .find_map(|directive| {
            let (name, value) = directive.split_once('=')?;
            if !name.trim().eq_ignore_ascii_case("max-age") {
                return None;
            }
            value.trim().trim_matches('"').parse().ok()
        })
}

fn has_cache_directive(header: Option<&str>, wanted: &str) -> bool {
    header.is_some_and(|value| {
        value
            .split(',')
            .any(|directive| directive.trim().eq_ignore_ascii_case(wanted))
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
}
