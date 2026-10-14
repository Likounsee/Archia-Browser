use std::time::{Duration, Instant};

use crate::core::{
    cache::ResourceCache,
    memory::{MemoryBudget, MemoryReservation},
};
use std::sync::Arc;

use super::{HttpMethod, Request, Response};

const DEFAULT_CAPACITY: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone)]
struct CachedResponse {
    response: Response,
    stored_at: Instant,
    initial_age: u64,
    expires_at: Instant,
    _reservations: Vec<Arc<MemoryReservation>>,
}

#[derive(Debug)]
pub struct HttpCache {
    entries: ResourceCache<CachedResponse>,
    memory_budget: Option<Arc<MemoryBudget>>,
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
            memory_budget: None,
        }
    }

    /// Create a cache whose retained response bytes also count against a shared browser budget.
    pub fn with_memory_budget(capacity: usize, memory_budget: Arc<MemoryBudget>) -> Self {
        Self {
            entries: ResourceCache::new(capacity),
            memory_budget: Some(memory_budget),
        }
    }

    pub fn get(&mut self, request: &Request) -> Option<Response> {
        let key = cache_key(request);
        if request_forces_cache_bypass(request) {
            // Invalidate only when this request has the same public cache
            // identity. A credentialed request must not evict an unrelated
            // uncredentialed representation that happens to share its URL.
            if request_can_invalidate_cache_entry(request) {
                self.entries.remove(&key);
            }
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

        // RFC 9111 requires a cache to invalidate the target URI after a
        // successful response to an unsafe method. The cache is keyed by
        // method, so removing cache_key(request) here would only target the
        // POST/PUT/etc. key and leave the cached GET representation stale.
        // Invalidate the GET representation for 2xx/3xx responses; errors do
        // not prove that the resource was changed. CONNECT has tunnel semantics
        // and is deliberately excluded from resource invalidation.
        if matches!(
            request.method,
            HttpMethod::Post | HttpMethod::Put | HttpMethod::Delete | HttpMethod::Patch
        ) {
            if (200..400).contains(&response.status)
                && matches!(request.url.scheme(), "http" | "https")
                && !request.url.authority().contains('@')
            {
                let mut target = request.clone();
                target.method = HttpMethod::Get;
                self.entries.remove(&cache_key(&target));

                // RFC 9111 also invalidates same-origin Location and
                // Content-Location targets on successful unsafe responses.
                // Resolve references against the request URL, and fail closed
                // for malformed, non-network, user-info, or cross-origin URLs.
                for header in ["location", "content-location"] {
                    if let Some(reference) = response.header(header) {
                        if let Ok(url) = request.url.resolve(reference) {
                            if matches!(url.scheme(), "http" | "https")
                                && !url.authority().contains('@')
                                && request.url.same_origin(&url)
                            {
                                let mut related = request.clone();
                                related.url = url;
                                related.method = HttpMethod::Get;
                                self.entries.remove(&cache_key(&related));
                            }
                        }
                    }
                }
            }
            return;
        }

        let key = cache_key(request);
        if !request_can_store(request) {
            // Explicit cache-bypass directives invalidate the old entry, but
            // a credentialed request must not evict an unrelated public copy.
            if request_forces_cache_bypass(request) && request_can_invalidate_cache_entry(request) {
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

        let Some(max_age) = response_freshness_lifetime(response) else {
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
        // from the bounded network transport. Check the combined payload and
        // key estimate so an extreme size cannot overflow accounting arithmetic.
        // The normalized URL key is retained alongside the response. Long URLs
        // can exceed the fixed per-entry overhead, so account for their bytes too.
        let payload_bytes = accounted_response_bytes(response);
        let Some(bytes) = checked_entry_bytes(payload_bytes, key.len()) else {
            // An unrepresentable size must never saturate into a seemingly
            // valid reservation. Invalidate this representation, but leave
            // unrelated entries untouched.
            self.entries.remove(&key);
            return;
        };

        // Cache entries participate in the browser-wide budget when one is supplied.
        // The reservation follows the entry and is released on eviction or cache drop.
        let reservation = if let Some(budget) = &self.memory_budget {
            // An entry larger than the entire shared budget cannot ever fit; do not
            // evict useful entries for an impossible insertion.
            if bytes > budget.limit() {
                // The new representation cannot fit, but it still supersedes
                // the old one for this key. Invalidate only this entry; do not
                // evict unrelated cached responses for an impossible insertion.
                self.entries.remove(&key);
                return;
            }
            let previous = self.entries.remove(&key);
            let previous_reservations = previous
                .as_ref()
                .map(|entry| entry._reservations.clone())
                .unwrap_or_default();
            let previous_reserved = previous_reservations
                .iter()
                .fold(0usize, |total, reservation| {
                    total.saturating_add(reservation.bytes())
                });

            if previous_reserved >= bytes {
                // Reuse the old reservation(s) for same-sized or smaller responses.
                // Shrink them for smaller replacements so unused capacity returns
                // to the shared budget instead of remaining charged indefinitely.
                let mut remaining = bytes;
                for reservation in &previous_reservations {
                    let retained = remaining.min(reservation.bytes());
                    reservation.shrink_to(retained);
                    remaining = remaining.saturating_sub(retained);
                }
                drop(previous);
                previous_reservations
            } else {
                // Reserve only the growth above the old entry's reservation. Keeping
                // the old reservation avoids a transient double charge and lets larger
                // replacements fit whenever their incremental cost is available.
                let additional = bytes.saturating_sub(previous_reserved);
                let reservation = loop {
                    if budget.available() >= additional {
                        if let Some(reservation) =
                            MemoryReservation::try_new(Arc::clone(budget), additional)
                        {
                            break Arc::new(reservation);
                        }
                    }
                    // Shared allocations may race with this cache. Reclaim LRU entries
                    // until the incremental reservation succeeds or no entry remains.
                    if self.entries.remove_oldest().is_none() {
                        if let Some(previous) = previous {
                            // ResourceCache::insert accounts for the key internally, so pass
                            // only the response payload estimate when restoring this entry.
                            let previous_bytes = accounted_response_bytes(&previous.response);
                            self.entries.insert(key.clone(), previous, previous_bytes);
                        }
                        return;
                    }
                };
                let mut reservations = previous_reservations;
                reservations.push(reservation);
                drop(previous);
                reservations
            }
        } else {
            Vec::new()
        };

        self.entries.insert(
            key,
            CachedResponse {
                response: response.clone(),
                stored_at: Instant::now(),
                initial_age: age,
                expires_at,
                _reservations: reservation,
            },
            payload_bytes,
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

fn accounted_response_bytes(response: &Response) -> usize {
    response
        .headers
        .iter()
        .fold(response.body.len(), |total, (name, value)| {
            total.saturating_add(name.len()).saturating_add(value.len())
        })
        .saturating_add(64)
}

fn checked_entry_bytes(payload_bytes: usize, key_bytes: usize) -> Option<usize> {
    payload_bytes.checked_add(key_bytes)
}

fn canonical_cache_host(host: &str) -> String {
    // A single trailing dot marks an absolute DNS name; it does not identify
    // a different HTTP origin from the same name without that dot.
    host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase()
}

fn cache_key(request: &Request) -> String {
    // DNS host names are case-insensitive, and omitting an HTTP(S) default
    // port is equivalent to spelling it explicitly. Normalize both so these
    // URL spellings do not create duplicate entries for the same resource.
    // URL fragments are client-side identifiers and are never sent in an
    // HTTP request target, so they must not participate in the cache key.
    let host = canonical_cache_host(request.url.host());
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
            cache_control_is_malformed(value)
                || has_cache_directive(Some(value), "no-cache")
                || has_cache_directive(Some(value), "no-store")
                || has_cache_max_age_zero(Some(value))
        })
        || has_pragma_no_cache(request)
}

/// Whether a request explicitly forbids network access on a cache miss.
///
/// RFC cache directives are comma-separated, but commas inside quoted values
/// are not separators; reuse the cache-control tokenizer for consistent parsing.
pub(crate) fn request_only_if_cached(request: &Request) -> bool {
    request
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("cache-control"))
        .any(|(_, value)| {
            // If the directive list is malformed, fail closed: a hidden
            // only-if-cached directive must not accidentally permit network I/O.
            cache_control_is_malformed(value)
                || cache_control_directives(value)
                    .into_iter()
                    .any(|directive| {
                        let name = directive
                            .split_once('=')
                            .map_or(directive, |(name, _)| name);
                        trim_http_ows(name).eq_ignore_ascii_case("only-if-cached")
                    })
        })
}

/// Whether a request may invalidate an uncredentialed cache entry for its URL.
///
/// Cache-Control bypass directives are deliberately not considered here: they
/// are the reason for invalidation. Identity-changing or representation-specific
/// headers are considered, so private/conditional/range requests cannot evict a
/// public full-response entry merely because the URL matches.
fn request_can_invalidate_cache_entry(request: &Request) -> bool {
    matches!(request.method, HttpMethod::Get)
        && matches!(request.url.scheme(), "http" | "https")
        && !request.url.authority().contains('@')
        && !has_request_header(request, "authorization")
        && !has_request_header(request, "proxy-authorization")
        && !has_request_header(request, "cookie")
        && !has_request_header(request, "connection")
        && !has_request_header(request, "range")
        && !has_request_header(request, "if-range")
        && !request_has_conditional_headers(request)
}

fn request_can_use_cache(request: &Request) -> bool {
    matches!(request.method, HttpMethod::Get)
        && matches!(request.url.scheme(), "http" | "https")
        // Connection is hop-by-hop and can nominate additional fields. Until
        // request normalization strips them, do not let a cache hit bypass
        // transport-level validation or vary the request outside the cache key.
        && !has_request_header(request, "connection")
        // User-info is not sent by this transport and must not alias a public
        // cache entry for the same host/path before transport validation runs.
        && !request.url.authority().contains('@')
        && !has_request_header(request, "authorization")
        // Proxy credentials are also identity-sensitive, even though this
        // cache key intentionally does not include any credential headers.
        && !has_request_header(request, "proxy-authorization")
        && !has_request_header(request, "cookie")
        // Range responses have different representation semantics and this
        // cache does not implement partial-response merging or If-Range.
        && !has_request_header(request, "range")
        && !has_request_header(request, "if-range")
        && !request_has_conditional_headers(request)
        && !request
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("cache-control"))
            .any(|(_, value)| {
                cache_control_is_malformed(value)
                    || has_cache_directive(Some(value), "no-cache")
                    || has_cache_directive(Some(value), "no-store")
                    || has_cache_max_age_zero(Some(value))
            })
        && !has_pragma_no_cache(request)
}

fn request_can_store(request: &Request) -> bool {
    matches!(request.method, HttpMethod::Get)
        && matches!(request.url.scheme(), "http" | "https")
        && !has_request_header(request, "connection")
        && !request.url.authority().contains('@')
        && !has_request_header(request, "authorization")
        && !has_request_header(request, "proxy-authorization")
        && !has_request_header(request, "cookie")
        // Range responses have different representation semantics and this
        // cache does not implement partial-response merging or If-Range.
        && !has_request_header(request, "range")
        && !has_request_header(request, "if-range")
        && !request_has_conditional_headers(request)
        && !request
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("cache-control"))
            .any(|(_, value)| {
                cache_control_is_malformed(value)
                    || has_cache_directive(Some(value), "no-store")
                    || has_cache_directive(Some(value), "no-cache")
            })
        && !has_pragma_no_cache(request)
}

fn request_has_conditional_headers(request: &Request) -> bool {
    [
        "if-match",
        "if-none-match",
        "if-modified-since",
        "if-unmodified-since",
    ]
    .iter()
    .any(|name| has_request_header(request, name))
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

fn response_has_valid_content_length(response: &Response) -> bool {
    let mut values = response
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| trim_http_ows(value));

    let Some(value) = values.next() else {
        return true;
    };
    if values.next().is_some()
        || value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return false;
    }

    value
        .parse::<usize>()
        .is_ok_and(|length| length == response.body.len())
}

fn response_has_ambiguous_singleton_header(response: &Response, wanted: &str) -> bool {
    response
        .headers
        .keys()
        .filter(|name| name.eq_ignore_ascii_case(wanted))
        .take(2)
        .count()
        > 1
}

fn response_can_store(response: &Response) -> bool {
    response.status == 200
        && !response_has_ambiguous_singleton_header(response, "content-type")
        && response_has_valid_content_length(response)
        && response.set_cookie_headers().is_empty()
        && !response_has_header(response, "set-cookie")
        && !response
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("cache-control"))
            .any(|(_, value)| {
                cache_control_is_malformed(value)
                    || has_cache_directive(Some(value), "no-store")
                    || has_cache_directive(Some(value), "no-cache")
                    // This cache does not implement field-specific private semantics.
                    // Conservatively reject every private response to avoid cross-user reuse.
                    || has_cache_directive(Some(value), "private")
            })
        && !response
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("pragma"))
            .any(|(_, value)| {
                // Pragma is legacy, but malformed quoted directives must not
                // make an otherwise non-cacheable response reusable. Reuse the
                // quote-aware tokenizer shared with Cache-Control parsing.
                cache_control_is_malformed(value)
                    || cache_control_directives(value).into_iter().any(|directive| {
                        let name = directive
                            .split_once('=')
                            .map_or(directive, |(name, _)| name);
                        trim_http_ows(name).eq_ignore_ascii_case("no-cache")
                    })
            })
        // A 200 response carrying Content-Range is still a partial representation;
        // caching it under the full-resource key could serve truncated content later.
        && !response_has_header(response, "content-range")
        // Transfer framing and proxy/upgrade metadata describe one connection,
        // not a reusable representation. The body has already been decoded, so
        // retaining these fields in a cache entry would make its metadata lie.
        && ![
            "transfer-encoding",
            "trailer",
            "te",
            "connection",
            "keep-alive",
            "upgrade",
            "proxy-connection",
            // This cache does not merge partial representations. Never retain
            // range metadata even if a broken origin labels it as status 200.
            "content-range",
            "proxy-authenticate",
            "proxy-authorization",
        ]
        .iter()
        .any(|name| response_has_header(response, name))
        && !response_has_header(response, "vary")
}

fn trim_http_ows(value: &str) -> &str {
    value.trim_matches(|character| character == ' ' || character == '\t')
}

/// Reject malformed quoted Cache-Control values rather than accidentally
/// ignoring directives hidden behind an unterminated quoted string.
fn cache_control_is_malformed(value: &str) -> bool {
    let mut quoted = false;
    let mut escaped = false;

    for byte in value.bytes() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && byte == b'\\' {
            escaped = true;
        } else if byte == b'"' {
            quoted = !quoted;
        }
    }

    quoted || escaped
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
            directives.push(trim_http_ows(&value[start..index]));
            start = index + 1;
        }
    }
    directives.push(trim_http_ows(&value[start..]));
    directives
}

/// Parse RFC delta-seconds strictly: one or more ASCII decimal digits.
fn parse_delta_seconds(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok()
}

/// Determine the response freshness lifetime. Prefer Cache-Control max-age;
/// only fall back to Expires when no max-age directive was supplied. An invalid
/// or duplicate max-age must not be rescued by a more permissive fallback.
fn response_freshness_lifetime(response: &Response) -> Option<u64> {
    let has_max_age = response
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("cache-control"))
        .flat_map(|(_, value)| cache_control_directives(value))
        .any(|directive| {
            directive
                .split_once('=')
                .map_or(directive, |(name, _)| name)
                .trim_matches(|character| character == ' ' || character == '\t')
                .eq_ignore_ascii_case("max-age")
        });

    if has_max_age {
        return response_max_age(response);
    }

    let mut expires_headers = response
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("expires"));
    let expires_value = expires_headers.next()?.1;
    if expires_headers.next().is_some() {
        return None;
    }
    let expires = httpdate::parse_http_date(trim_http_ows(expires_value)).ok()?;

    let mut date_headers = response
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("date"));
    let date = match date_headers.next() {
        Some((_, value)) => {
            if date_headers.next().is_some() {
                return None;
            }
            httpdate::parse_http_date(trim_http_ows(value)).ok()?
        }
        None => std::time::SystemTime::now(),
    };

    expires
        .duration_since(date)
        .ok()
        .map(|duration| duration.as_secs())
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
                Some((name, value)) => (trim_http_ows(name), Some(trim_http_ows(value))),
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
            max_age = Some(parse_delta_seconds(value)?);
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
                Some((name, value)) => (trim_http_ows(name), Some(trim_http_ows(value))),
                None => (trim_http_ows(directive), None),
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
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(());
            }
            parsed = Some(parse_delta_seconds(value).ok_or(())?);
        }
    }
    Ok(parsed)
}

fn response_age(response: &Response) -> Option<u64> {
    let mut age_headers = response
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("age"));
    let age_value = match age_headers.next() {
        Some((_, value)) => {
            if age_headers.next().is_some() {
                return None;
            }
            let value = value.trim_matches(|character| character == ' ' || character == '\t');
            // RFC 9111 defines Age as delta-seconds (1*DIGIT). Integer parsing
            // alone is too permissive on some inputs; reject signs and every
            // non-ASCII digit so malformed intermediaries cannot influence
            // freshness calculations.
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            parse_delta_seconds(value)?
        }
        None => 0,
    };

    // Age is not the only measure of staleness. A response can arrive with an
    // old Date and no Age header (for example from an intermediary); treating
    // that response as age zero would extend its freshness by max-age again.
    let mut date_headers = response
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("date"));
    let apparent_age = match date_headers.next() {
        Some((_, value)) => {
            if date_headers.next().is_some() {
                return None;
            }
            let date = httpdate::parse_http_date(
                value.trim_matches(|character| character == ' ' || character == '\t'),
            )
            .ok()?;
            std::time::SystemTime::now()
                .duration_since(date)
                .unwrap_or_default()
                .as_secs()
        }
        None => 0,
    };

    Some(age_value.max(apparent_age))
}

fn has_cache_max_age_zero(header: Option<&str>) -> bool {
    header.is_some_and(|value| {
        cache_control_directives(value)
            .into_iter()
            .any(|directive| {
                let Some((name, value)) = directive.split_once('=') else {
                    return false;
                };
                trim_http_ows(name).eq_ignore_ascii_case("max-age")
                    && parse_delta_seconds(trim_http_ows(value).trim_matches('"')) == Some(0)
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
                trim_http_ows(name).eq_ignore_ascii_case(wanted)
            })
    })
}

fn has_pragma_no_cache(request: &Request) -> bool {
    request
        .headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("pragma"))
        .any(|(_, value)| {
            // A malformed quoted value is ambiguous. Fail closed and bypass
            // the cache rather than accidentally treating a hidden directive
            // as permission to reuse a stored response.
            cache_control_is_malformed(value)
                || cache_control_directives(value)
                    .into_iter()
                    .any(|directive| {
                        let name = directive
                            .split_once('=')
                            .map_or(directive, |(name, _)| name);
                        trim_http_ows(name).eq_ignore_ascii_case("no-cache")
                    })
        })
}

#[cfg(test)]
mod tests {
    #[test]
    fn cache_entry_size_overflow_is_rejected_without_saturating() {
        assert_eq!(super::checked_entry_bytes(10, 20), Some(30));
        assert_eq!(super::checked_entry_bytes(0, 0), Some(0));
        assert_eq!(super::checked_entry_bytes(usize::MAX, 1), None);
        assert_eq!(super::checked_entry_bytes(usize::MAX - 3, 4), None);
    }

    #[test]
    fn connection_requests_bypass_cache_lookup_and_storage() {
        let mut request =
            super::Request::new(super::super::super::super::Url::parse("https://example.org/resource").unwrap());
        request
            .headers
            .insert("connection".into(), "x-private".into());
        let response = super::Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(b"public copy".to_vec());
        let mut cache = super::HttpCache::default();

        cache.store(&request, &response);
        assert_eq!(cache.len(), 0);
        assert!(!super::request_can_use_cache(&request));
    }

    #[test]
    fn proxy_credentials_and_connection_bypass_cache_without_evicting_public_entry() {
        let url = super::super::super::super::Url::parse("https://example.org/resource").unwrap();
        let public_request = super::Request::new(url.clone());
        let public_response = super::Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(b"public copy".to_vec());
        let mut cache = super::HttpCache::default();
        cache.store(&public_request, &public_response);
        assert_eq!(cache.len(), 1);

        for header in ["proxy-authorization", "connection"] {
            let mut private_request =
                super::Request::new(url.clone()).with_header("cache-control", "no-cache");
            private_request.headers.insert(
                header.to_owned(),
                if header == "connection" {
                    "close".to_owned()
                } else {
                    "Basic secret".to_owned()
                },
            );
            assert!(cache.get(&private_request).is_none());
            assert_eq!(
                cache.len(),
                1,
                "{header} must not evict a public entry through cache bypass"
            );
            cache.store(&private_request, &public_response);
            assert_eq!(cache.len(), 1, "{header} request must not be stored");
        }
    }

    #[test]
    fn cache_rejects_ambiguous_content_type_metadata() {
        let request =
            super::Request::new(super::super::super::super::Url::parse("https://example.org/resource").unwrap());
        let mut response = super::Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_header("content-type", "text/html")
            .with_body(b"<p>safe</p>".to_vec());
        response
            .headers
            .insert("Content-Type".into(), "application/json".into());

        let mut cache = super::HttpCache::default();
        cache.store(&request, &response);
        assert_eq!(
            cache.len(),
            0,
            "ambiguous representation metadata must not enter the shared cache"
        );
    }

    #[test]
    fn cache_rejects_invalid_or_conflicting_content_length_metadata() {
        let valid = super::Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_header("content-length", "4")
            .with_body(b"data".to_vec());
        assert!(super::response_can_store(&valid));

        for response in [
            super::Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_header("content-length", "invalid")
                .with_body(b"data".to_vec()),
            super::Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_header("content-length", "3")
                .with_body(b"data".to_vec()),
        ] {
            assert!(
                !super::response_can_store(&response),
                "invalid or mismatched Content-Length must not be cached"
            );
        }

        let mut duplicate = super::Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_header("content-length", "4")
            .with_body(b"data".to_vec());
        duplicate
            .headers
            .insert("Content-Length".into(), "4".into());
        assert!(!super::response_can_store(&duplicate));
    }

    #[test]
    fn cache_rejects_connection_specific_response_metadata() {
        let request =
            super::Request::new(super::super::super::super::Url::parse("https://example.org/resource").unwrap());

        for name in [
            "Transfer-Encoding",
            "Trailer",
            "TE",
            "Connection",
            "Keep-Alive",
            "Upgrade",
            "Proxy-Connection",
            "Content-Range",
            "Proxy-Authenticate",
            "Proxy-Authorization",
        ] {
            let response = super::Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_header(name, "connection-specific")
                .with_body(b"decoded body".to_vec());
            assert!(
                !super::response_can_store(&response),
                "connection-specific header {name} must not enter the reusable cache"
            );

            let mut cache = super::HttpCache::default();
            cache.store(&request, &response);
            assert_eq!(cache.len(), 0, "{name} response must not be cached");
        }
    }

    #[test]
    fn malformed_pragma_quoted_value_fails_closed_for_cache_reuse() {
        let mut request =
            super::Request::new(super::super::super::super::Url::parse("https://example.org/resource").unwrap());
        request.headers.insert(
            "Pragma".into(),
            r#"extension="unterminated,no-cache"#.into(),
        );
        assert!(
            super::has_pragma_no_cache(&request),
            "malformed Pragma must not permit cache reuse"
        );

        request
            .headers
            .insert("Pragma".into(), "extension=value, No-Cache".into());
        assert!(super::has_pragma_no_cache(&request));

        request
            .headers
            .insert("Pragma".into(), "extension=value".into());
        assert!(!super::has_pragma_no_cache(&request));
    }

    #[test]
    fn cache_key_uses_canonical_host_default_port_and_ignores_fragment() {
        let implicit = super::cache_key(&super::Request::new(
            super::super::super::super::Url::parse("https://EXAMPLE.ORG:443/resource?q=1#top").unwrap(),
        ));
        let equivalent = super::cache_key(&super::Request::new(
            super::super::super::super::Url::parse("https://example.org/resource?q=1#other").unwrap(),
        ));

        assert_eq!(
            implicit, equivalent,
            "host case, explicit default port, and fragment must not split one cache resource"
        );

        let different_query = super::cache_key(&super::Request::new(
            super::super::super::super::Url::parse("https://example.org/resource?q=2").unwrap(),
        ));
        assert_ne!(
            implicit, different_query,
            "query strings must remain part of the cache resource identity"
        );
    }

    #[test]
    fn trailing_dot_host_uses_the_same_http_cache_entry() {
        let mut cache = HttpCache::default();
        let dotted = Request::new(super::super::Url::parse("https://example.org./resource").unwrap());
        let ordinary = Request::new(super::super::Url::parse("https://example.org/resource").unwrap());
        let response = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(b"canonical host".to_vec());

        cache.store(&dotted, &response);

        assert_eq!(
            cache.get(&ordinary).unwrap().body,
            b"canonical host",
            "an absolute DNS name and its undotted spelling must share a cache entry"
        );
        assert_eq!(
            cache_key(&dotted),
            cache_key(&ordinary),
            "canonical host spelling must not create duplicate cache keys"
        );
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn shared_budget_accounts_for_http_cache_key_bytes() {
        use crate::core::memory::MemoryBudget;
        use std::sync::Arc;

        let budget = Arc::new(MemoryBudget::new(1024));
        let request = Request::new(super::super::super::super::Url::parse("https://example.org/").unwrap());
        let response = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(vec![b'x'; 8]);
        let expected = super::accounted_response_bytes(&response)
            .saturating_add(super::cache_key(&request).len());
        let mut cache = super::HttpCache::with_memory_budget(2048, Arc::clone(&budget));

        cache.store(&request, &response);

        assert_eq!(cache.len(), 1);
        assert_eq!(budget.used(), expected);
        cache.clear();
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn replacing_shared_budget_entry_does_not_evict_unrelated_lru_entry() {
        use crate::core::memory::MemoryBudget;
        use std::sync::Arc;

        let budget = Arc::new(MemoryBudget::new(600));
        let first = Request::new(super::super::super::super::Url::parse("https://example.org/first").unwrap());
        let second = Request::new(super::super::super::super::Url::parse("https://example.org/second").unwrap());
        let response = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_header("content-type", "text/plain")
            .with_body(vec![b'x'; 128]);
        let replacement = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_header("content-type", "text/plain")
            .with_body(vec![b'y'; 128]);
        let mut cache = HttpCache::with_memory_budget(2048, Arc::clone(&budget));

        cache.store(&first, &response);
        cache.store(&second, &response);
        assert_eq!(cache.len(), 2);
        // Make first newer, so second is the LRU entry.
        assert!(cache.get(&first).is_some());

        cache.store(&first, &replacement);

        assert_eq!(
            cache.len(),
            2,
            "replacement should preserve the unrelated entry"
        );
        assert_eq!(cache.get(&first).unwrap().body, replacement.body);
        assert!(cache.get(&second).is_some());
        assert!(budget.used() <= budget.limit());
    }

    #[test]
    fn shared_budget_does_not_evict_live_resources_when_cache_cannot_fit() {
        use crate::core::{memory::MemoryBudget, resource::ResourceManager};
        use std::sync::Arc;

        let budget = Arc::new(MemoryBudget::new(256));
        let mut resources = ResourceManager::with_budget(Arc::clone(&budget));
        let live_resource = resources.allocate(200).unwrap();
        let request = Request::new(super::super::super::super::Url::parse("https://example.org/").unwrap());
        let response = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_header("content-type", "text/plain")
            .with_body(vec![b'x'; 128]);
        let mut cache = HttpCache::with_memory_budget(2048, Arc::clone(&budget));

        cache.store(&request, &response);

        assert_eq!(cache.len(), 0, "cache entry must not exceed shared budget");
        assert_eq!(
            budget.used(),
            200,
            "live resource reservation must remain intact"
        );
        assert_eq!(budget.available(), 56);
        drop(live_resource);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn shared_budget_pressure_evicts_oldest_cached_response() {
        use crate::core::memory::MemoryBudget;
        use std::sync::Arc;

        let budget = Arc::new(MemoryBudget::new(320));
        let first = Request::new(super::super::super::super::Url::parse("https://example.org/first").unwrap());
        let second = Request::new(super::super::super::super::Url::parse("https://example.org/second").unwrap());
        let response = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_header("content-type", "text/plain")
            .with_body(vec![b'x'; 128]);
        let mut cache = HttpCache::with_memory_budget(2048, Arc::clone(&budget));

        cache.store(&first, &response);
        assert_eq!(cache.len(), 1);
        assert!(budget.used() <= budget.limit());

        cache.store(&second, &response);
        assert_eq!(cache.len(), 1);
        assert!(cache.get(&first).is_none());
        assert!(cache.get(&second).is_some());
        assert!(budget.used() <= budget.limit());
    }

    #[test]
    fn larger_replacement_reserves_only_incremental_shared_memory() {
        use crate::core::memory::MemoryBudget;
        use std::sync::Arc;

        let budget = Arc::new(MemoryBudget::new(512));
        let request = make_request("https://example.org/resource");
        let original = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(vec![b'o'; 64]);
        let replacement = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(vec![b'n'; 128]);
        let mut cache = HttpCache::with_memory_budget(4096, Arc::clone(&budget));

        cache.store(&request, &original);
        let before = budget.used();
        cache.store(&request, &replacement);

        assert_eq!(cache.get(&request).unwrap().body, replacement.body);
        assert_eq!(cache.len(), 1);
        assert!(budget.used() > before);
        assert!(budget.used() <= budget.limit());
    }

    #[test]
    fn smaller_replacement_releases_excess_shared_memory() {
        use crate::core::memory::MemoryBudget;
        use std::sync::Arc;

        let budget = Arc::new(MemoryBudget::new(4096));
        let request = make_request("https://example.org/resource");
        let original = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(vec![b'o'; 512]);
        let replacement = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(vec![b'n'; 32]);
        let mut cache = HttpCache::with_memory_budget(4096, Arc::clone(&budget));

        cache.store(&request, &original);
        let used_before = budget.used();
        cache.store(&request, &replacement);

        assert_eq!(cache.get(&request).unwrap().body, replacement.body);
        assert_eq!(cache.len(), 1);
        assert!(
            budget.used() < used_before,
            "a smaller replacement must release the old entry's excess reservation"
        );
        assert_eq!(
            budget.used(),
            accounted_response_bytes(&replacement) + cache_key(&request).len(),
            "shared accounting must match the replacement's retained response and key"
        );
    }

    #[test]
    fn cache_accounting_includes_long_retained_url_keys() {
        let url = format!("https://example.org/{}", "a".repeat(512));
        let request = Request::new(super::super::Url::parse(&url).unwrap());
        let response = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(b"x".to_vec());
        let budget = Arc::new(MemoryBudget::new(4096));
        let mut cache = HttpCache::with_memory_budget(4096, Arc::clone(&budget));

        cache.store(&request, &response);

        assert_eq!(cache.len(), 1);
        assert!(
            budget.used() >= cache_key(&request).len() + accounted_response_bytes(&response),
            "shared budget must account for both the retained URL key and response"
        );
    }

    #[test]
    fn oversized_shared_budget_replacement_invalidates_only_same_key() {
        let budget = Arc::new(MemoryBudget::new(512));
        let mut cache = HttpCache::with_memory_budget(4096, Arc::clone(&budget));
        let replaced = make_request("https://example.org/replaced");
        let unrelated = make_request("https://example.org/unrelated");
        cache.store(
            &replaced,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(vec![b'a'; 64]),
        );
        cache.store(
            &unrelated,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(vec![b'b'; 64]),
        );
        assert_eq!(cache.len(), 2);

        cache.store(
            &replaced,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(vec![b'x'; 1024]),
        );

        assert!(cache.get(&replaced).is_none());
        assert_eq!(cache.get(&unrelated).unwrap().body, vec![b'b'; 64]);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn same_sized_replacement_works_when_shared_budget_is_full() {
        use crate::core::memory::MemoryBudget;
        use std::sync::Arc;

        let budget = Arc::new(MemoryBudget::new(512));
        let request = make_request("https://example.org/resource");
        let original = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(vec![b'o'; 64]);
        let replacement = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(vec![b'n'; 64]);
        let mut cache = HttpCache::with_memory_budget(4096, Arc::clone(&budget));

        cache.store(&request, &original);
        let used_before = budget.used();
        assert!(budget.try_reserve(budget.available()));

        cache.store(&request, &replacement);

        assert_eq!(cache.get(&request).unwrap().body, replacement.body);
        assert_eq!(cache.len(), 1);
        assert_eq!(budget.used(), 512);
        assert!(budget.used() >= used_before);
    }

    #[test]
    fn failed_shared_budget_replacement_preserves_previous_response() {
        use crate::core::memory::MemoryBudget;
        use std::sync::Arc;

        let budget = Arc::new(MemoryBudget::new(512));
        let request = make_request("https://example.org/resource");
        let original = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(vec![b'o'; 64]);
        let replacement = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(vec![b'n'; 256]);
        let mut cache = HttpCache::with_memory_budget(4096, Arc::clone(&budget));

        cache.store(&request, &original);
        assert_eq!(cache.get(&request).unwrap().body, original.body);

        // Consume the remaining shared budget so the larger replacement cannot
        // be reserved, even after every other cache entry has been evicted.
        let remaining = budget.available();
        assert!(remaining > 1);
        assert!(budget.try_reserve(remaining - 1));

        cache.store(&request, &replacement);

        assert_eq!(
            cache.get(&request).unwrap().body,
            original.body,
            "failed replacement must retain the old cached representation"
        );
        assert_eq!(cache.len(), 1);
        assert_eq!(
            cache.used_bytes(),
            accounted_response_bytes(&original).saturating_add(cache_key(&request).len()),
            "restoring the old entry must preserve cache-capacity accounting for its URL key"
        );
    }

    #[test]
    fn shared_memory_budget_tracks_cached_response_lifetime() {
        use crate::core::memory::MemoryBudget;
        use std::sync::Arc;

        let budget = Arc::new(MemoryBudget::new(4096));
        let request = Request::new(super::super::super::super::Url::parse("https://example.org/").unwrap());
        let response = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_header("content-type", "text/plain")
            .with_body(vec![b'x'; 128]);

        {
            let mut cache = HttpCache::with_memory_budget(2048, Arc::clone(&budget));
            cache.store(&request, &response);
            assert_eq!(cache.len(), 1);
            assert!(budget.used() > 0);
        }

        assert_eq!(budget.used(), 0);
    }

    use super::*;
    use crate::net::Url;

    fn make_request(url: &str) -> Request {
        Request::new(super::super::Url::parse(url).unwrap())
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
            response.body.len()
                + 64
                + "cache-control".len()
                + "max-age=60".len()
                + super::cache_key(&second).len()
        );
        assert!(cache.get(&first).is_none());
        assert!(cache.get(&second).is_some());
    }

    #[test]
    fn malformed_cache_control_never_reuses_or_stores_a_response() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/malformed-cache-control");
        let valid = Response::new(200)
            .with_header("cache-control", "public, max-age=60")
            .with_body(b"older public response".to_vec());
        cache.store(&request, &valid);
        assert!(cache.get(&request).is_some());

        // The second header has an unterminated quoted value. A naive
        // comma-split parser could miss the no-store directive inside it.
        let malformed = Response::new(200)
            .with_header("cache-control", "public, max-age=60")
            .with_header("Cache-Control", r#"extension="unterminated, no-store"#)
            .with_body(b"must not replace cached content".to_vec());
        cache.store(&request, &malformed);

        assert!(cache.is_empty());
        assert!(cache.get(&request).is_none());
    }

    #[test]
    fn malformed_cache_control_is_treated_as_only_if_cached() {
        let mut request = make_request("https://example.org/cache-only");
        request.headers.insert(
            "cache-control".to_owned(),
            r#"extension="unterminated, only-if-cached"#.to_owned(),
        );

        assert!(request_only_if_cached(&request));
    }

    #[test]
    fn malformed_request_cache_control_bypasses_cache() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/malformed-request");
        cache.store(
            &request,
            &Response::new(200)
                .with_header("cache-control", "public, max-age=60")
                .with_body(b"cached".to_vec()),
        );

        let mut malformed_request = request.clone();
        malformed_request.headers.insert(
            "cache-control".to_owned(),
            r#"extension="unterminated, max-age=0"#.to_owned(),
        );

        assert!(cache.get(&malformed_request).is_none());
        assert!(cache.is_empty());
    }

    #[test]
    fn refuses_non_ascii_whitespace_in_max_age_header() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/invalid-max-age");
        let response = Response::new(200)
            .with_header("cache-control", "public, max-age=\u{00a0}60\u{00a0}")
            .with_body(b"ambiguous max-age".to_vec());

        cache.store(&request, &response);

        assert!(cache.is_empty());
        assert!(cache.get(&request).is_none());
    }

    #[test]
    fn refuses_non_ascii_whitespace_in_age_header() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/invalid-age");
        let response = Response::new(200)
            .with_header("cache-control", "public, max-age=60")
            .with_header("age", "\u{00a0}0\u{00a0}")
            .with_body(b"ambiguous age".to_vec());

        cache.store(&request, &response);

        assert!(cache.is_empty());
        assert!(cache.get(&request).is_none());
    }

    #[test]
    fn does_not_extend_freshness_of_response_with_old_date() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/stale");
        let old_date =
            httpdate::fmt_http_date(std::time::SystemTime::now() - Duration::from_secs(120));
        let response = Response::new(200)
            .with_header("cache-control", "public, max-age=60")
            .with_header("date", old_date)
            .with_body(b"must not be cached as fresh".to_vec());

        cache.store(&request, &response);

        assert!(cache.is_empty());
        assert!(cache.get(&request).is_none());
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
    fn userinfo_url_cannot_reuse_or_replace_public_cache_entry() {
        let mut cache = HttpCache::default();
        let public = make_request("https://example.org/account");
        let response = Response::new(200)
            .with_header("cache-control", "public, max-age=60")
            .with_body(b"public representation".to_vec());
        cache.store(&public, &response);

        let userinfo = make_request("https://alice:secret@example.org/account");
        assert!(cache.get(&userinfo).is_none());
        cache.store(
            &userinfo,
            &Response::new(200)
                .with_header("cache-control", "public, max-age=60")
                .with_body(b"credential-specific".to_vec()),
        );

        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&public).unwrap().body, b"public representation");
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
    fn content_range_response_is_not_cached_and_evicts_older_full_response() {
        let mut cache = HttpCache::new(4096);
        let request = make_request("https://example.org/resource");
        cache.store(
            &request,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(b"complete representation".to_vec()),
        );
        assert!(cache.get(&request).is_some());

        cache.store(
            &request,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_header("content-range", "bytes 0-3/100")
                .with_body(b"part".to_vec()),
        );

        assert!(cache.get(&request).is_none());
        assert!(cache.is_empty());
    }

    #[test]
    fn vary_response_evicts_an_older_representation_for_the_same_key() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/language");
        cache.store(
            &request,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(b"english".to_vec()),
        );
        assert_eq!(cache.get(&request).unwrap().body, b"english");

        // Since this cache does not implement Vary matching, a later response
        // with Vary must invalidate the older representation rather than
        // leave a potentially incorrect language-specific response reusable.
        cache.store(
            &request,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_header("vary", "accept-language")
                .with_body(b"localized".to_vec()),
        );

        assert!(cache.get(&request).is_none());
        assert!(cache.is_empty());
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
    fn private_responses_are_not_cached_and_evict_older_entries() {
        for private_directive in ["private", "private=\"set-cookie\""] {
            let mut cache = HttpCache::default();
            let request = make_request("https://example.org/account");
            cache.store(
                &request,
                &Response::new(200)
                    .with_header("cache-control", "public, max-age=60")
                    .with_body(b"previous public response".to_vec()),
            );
            assert!(cache.get(&request).is_some());

            // This implementation cannot safely strip named private fields,
            // so both bare and field-qualified private directives are non-cacheable.
            cache.store(
                &request,
                &Response::new(200)
                    .with_header("cache-control", format!("{private_directive}, max-age=60"))
                    .with_body(b"private account response".to_vec()),
            );

            assert!(
                cache.get(&request).is_none(),
                "private response must not be cached: {private_directive}"
            );
            assert!(cache.is_empty());
        }
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
                _reservations: Vec::new(),
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
            "max-age=+1",
            "max-age=+0",
            "max-age=１２",
            "max-age=18446744073709551616",
            "min-fresh=+1",
            "min-fresh=18446744073709551616",
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
    fn credentialed_bypass_requests_do_not_evict_public_cache_entries() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/resource");
        let public_response = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(b"public representation".to_vec());
        cache.store(&request, &public_response);
        assert_eq!(cache.len(), 1);

        let cookie_bypass = request
            .clone()
            .with_header("Cookie", "session=secret")
            .with_header("Cache-Control", "no-cache");
        assert!(cache.get(&cookie_bypass).is_none());
        assert_eq!(
            cache.len(),
            1,
            "a credentialed cache bypass must not evict a public entry"
        );

        let authorization_bypass = request
            .clone()
            .with_header("Authorization", "Bearer secret")
            .with_header("Cache-Control", "no-store");
        cache.store(
            &authorization_bypass,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(b"private representation".to_vec()),
        );
        assert_eq!(
            cache.get(&request).unwrap().body,
            b"public representation",
            "storing a credentialed bypass response must preserve public cached data"
        );
        assert_eq!(cache.len(), 1);
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
        let request = Request::new(super::super::Url::parse("file:///tmp/index.html").unwrap());
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
    fn expires_header_provides_freshness_when_max_age_is_absent() {
        let request = make_request("https://example.org/expires");
        let date = std::time::SystemTime::now();
        let expires = date + Duration::from_secs(120);
        let response = Response::new(200)
            .with_header("date", httpdate::fmt_http_date(date))
            .with_header("expires", httpdate::fmt_http_date(expires))
            .with_body(b"expires cached".to_vec());
        let mut cache = HttpCache::default();

        cache.store(&request, &response);

        assert_eq!(
            cache.get(&request).unwrap().body,
            b"expires cached",
            "a valid Expires lifetime should be usable without Cache-Control max-age"
        );
    }

    #[test]
    fn invalid_or_ambiguous_expires_never_reuses_an_old_response() {
        let request = make_request("https://example.org/expires-invalid");
        let mut cache = HttpCache::default();
        cache.store(
            &request,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(b"older response".to_vec()),
        );

        let malformed = Response::new(200)
            .with_header("expires", "not an HTTP date")
            .with_body(b"malformed expiry".to_vec());
        // Response::with_header normalizes names into a BTreeMap, so construct
        // the mixed-case duplicate explicitly to exercise the conservative
        // validation path for responses assembled by non-transport callers.
        let mut duplicate = Response::new(200)
            .with_header("expires", "Wed, 21 Oct 2037 07:28:00 GMT")
            .with_body(b"ambiguous expiry".to_vec());
        duplicate.headers.insert(
            "Expires".to_owned(),
            "Wed, 21 Oct 2037 07:28:00 GMT".to_owned(),
        );

        for response in [malformed, duplicate] {
            cache.store(&request, &response);
            assert!(
                cache.get(&request).is_none(),
                "invalid or duplicate Expires values must invalidate an older representation"
            );
        }
    }

    #[test]
    fn duplicate_date_headers_do_not_create_ambiguous_expires_freshness() {
        let request = make_request("https://example.org/expires-date-duplicate");
        let now = std::time::SystemTime::now();
        let mut response = Response::new(200)
            .with_header("date", httpdate::fmt_http_date(now))
            .with_header(
                "expires",
                httpdate::fmt_http_date(now + Duration::from_secs(120)),
            )
            .with_body(b"ambiguous date".to_vec());
        response
            .headers
            .insert("Date".to_owned(), httpdate::fmt_http_date(now));

        let mut cache = HttpCache::default();
        cache.store(&request, &response);

        assert!(
            cache.get(&request).is_none(),
            "case-insensitive duplicate Date headers must not create a reusable cache entry"
        );
    }

    #[test]
    fn invalid_max_age_is_not_rescued_by_a_valid_expires_header() {
        let request = make_request("https://example.org/expires-max-age");
        for cache_control in ["max-age=invalid", "max-age=60, max-age=120"] {
            let response = Response::new(200)
                .with_header("cache-control", cache_control)
                .with_header("date", "Wed, 21 Oct 2015 07:28:00 GMT")
                .with_header("expires", "Wed, 21 Oct 2037 07:28:00 GMT")
                .with_body(b"must not cache".to_vec());
            let mut cache = HttpCache::default();

            cache.store(&request, &response);

            assert!(
                cache.get(&request).is_none(),
                "invalid or ambiguous max-age must not fall back to Expires: {cache_control}"
            );
        }
    }

    #[test]
    fn expires_freshness_still_obeys_age_header() {
        let request = make_request("https://example.org/expires-age");
        let date = std::time::SystemTime::now();
        let expires = date + Duration::from_secs(120);
        let response = Response::new(200)
            .with_header("date", httpdate::fmt_http_date(date))
            .with_header("expires", httpdate::fmt_http_date(expires))
            .with_header("age", "120")
            .with_body(b"already stale".to_vec());
        let mut cache = HttpCache::default();

        cache.store(&request, &response);

        assert!(
            cache.get(&request).is_none(),
            "Age must reduce Expires-based freshness just as it reduces max-age"
        );
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
            vec![("cache-control", "max-age=+1")],
            vec![("cache-control", "max-age=１２")],
            vec![("cache-control", "max-age=18446744073709551616")],
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
        for age in ["unknown", "-1", "+1", "1, 2", "１２"] {
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

    #[test]
    fn only_if_cached_is_detected_without_matching_quoted_values() {
        let request = make_request("https://example.org/resource").with_header(
            "cache-control",
            r#"private="only-if-cached, no-store", only-if-cached"#,
        );
        assert!(request_only_if_cached(&request));

        let ordinary = make_request("https://example.org/resource")
            .with_header("cache-control", r#"private="only-if-cached""#);
        assert!(!request_only_if_cached(&ordinary));
    }

    #[test]
    fn range_requests_bypass_cache_without_poisoning_full_representation() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/resource");
        let full_response = Response::new(200)
            .with_header("cache-control", "max-age=60")
            .with_body(b"full representation".to_vec());
        cache.store(&request, &full_response);
        assert_eq!(cache.len(), 1);

        let range_request = request.clone().with_header("Range", "bytes=0-3");
        assert!(
            cache.get(&range_request).is_none(),
            "a full cached representation must not satisfy a range request"
        );

        // A server response to a range request must not replace the cached
        // full representation under the same URL-only key.
        cache.store(
            &range_request,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(b"part".to_vec()),
        );
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&request).unwrap().body, b"full representation");
    }

    #[test]
    fn if_range_requests_bypass_cache() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/resource");
        cache.store(
            &request,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(b"full representation".to_vec()),
        );

        let conditional_range = request
            .clone()
            .with_header("Range", "bytes=0-3")
            .with_header("If-Range", "\"etag-value\"");
        assert!(cache.get(&conditional_range).is_none());
        cache.store(
            &conditional_range,
            &Response::new(206).with_body(b"part".to_vec()),
        );
        assert_eq!(cache.get(&request).unwrap().body, b"full representation");
    }

    #[test]
    fn conditional_requests_bypass_cache_until_revalidation_is_supported() {
        let mut cache = HttpCache::default();
        let request = make_request("https://example.org/resource");
        cache.store(
            &request,
            &Response::new(200)
                .with_header("cache-control", "max-age=60")
                .with_body(b"cached full representation".to_vec()),
        );

        for (name, value) in [
            ("If-Match", "\"etag\""),
            ("If-None-Match", "\"etag\""),
            ("If-Modified-Since", "Sun, 06 Nov 1994 08:49:37 GMT"),
            ("If-Unmodified-Since", "Sun, 06 Nov 1994 08:49:37 GMT"),
        ] {
            let conditional = request.clone().with_header(name, value);
            assert!(
                cache.get(&conditional).is_none(),
                "{name} must not be ignored"
            );
            cache.store(
                &conditional,
                &Response::new(304).with_header("cache-control", "max-age=60"),
            );
            assert_eq!(
                cache.get(&request).unwrap().body,
                b"cached full representation",
                "{name} must not poison the unconditional cached representation"
            );
        }
    }

    #[test]
    fn malformed_response_pragma_fails_closed_for_cache_storage() {
        let request = make_request("https://example.org/malformed-pragma-response");
        let response = Response::new(200)
            .with_header("cache-control", "max-age=3600")
            .with_header("pragma", r#"extension="unterminated,no-cache"#);
        let mut cache = HttpCache::new(4096);

        cache.store(&request, &response);

        assert!(
            cache.get(&request).is_none(),
            "malformed Pragma must not allow a response to be stored for reuse"
        );
        assert!(cache.is_empty());
    }
    #[test]
    fn successful_unsafe_methods_invalidate_cached_get_representation() {
        for method in [
            HttpMethod::Post,
            HttpMethod::Put,
            HttpMethod::Patch,
            HttpMethod::Delete,
        ] {
            let url = super::super::Url::parse("https://example.org/resource?item=1").unwrap();
            let get = Request::new(url.clone());
            let mut cache = HttpCache::default();
            cache.store(
                &get,
                &Response::new(200)
                    .with_header("cache-control", "max-age=3600")
                    .with_body(b"old representation".to_vec()),
            );
            assert!(cache.get(&get).is_some());

            let unsafe_request = Request::new(url).with_method(method);
            cache.store(
                &unsafe_request,
                &Response::new(204).with_header("cache-control", "no-store"),
            );

            assert!(
                cache.get(&get).is_none(),
                "{method:?} success must invalidate the cached GET representation"
            );
        }
    }

    #[test]
    fn successful_unsafe_response_invalidates_same_origin_location_targets() {
        for header in ["location", "content-location"] {
            let origin = super::super::Url::parse("https://example.org/api/update").unwrap();
            let related_url = super::super::Url::parse("https://example.org/api/related?view=full").unwrap();
            let related = Request::new(related_url.clone());
            let mutation = Request::new(origin.clone()).with_method(HttpMethod::Post);
            let mut cache = HttpCache::default();
            cache.store(
                &related,
                &Response::new(200)
                    .with_header("cache-control", "max-age=3600")
                    .with_body(b"stale related representation".to_vec()),
            );
            assert!(cache.get(&related).is_some());

            let response = Response::new(204).with_header(header, "../api/related?view=full");
            cache.store(&mutation, &response);

            assert!(
                cache.get(&related).is_none(),
                "successful unsafe response must invalidate same-origin {header} target"
            );
        }
    }

    #[test]
    fn successful_unsafe_response_does_not_invalidate_cross_origin_location() {
        let related_url = super::super::Url::parse("https://other.example/related").unwrap();
        let related = Request::new(related_url);
        let mutation = Request::new(super::super::Url::parse("https://example.org/update").unwrap())
            .with_method(HttpMethod::Post);
        let mut cache = HttpCache::default();
        cache.store(
            &related,
            &Response::new(200)
                .with_header("cache-control", "max-age=3600")
                .with_body(b"other origin remains cached".to_vec()),
        );

        cache.store(
            &mutation,
            &Response::new(204).with_header("location", "https://other.example/related"),
        );

        assert_eq!(
            cache.get(&related).unwrap().body,
            b"other origin remains cached",
            "a response must not invalidate a cache entry on another origin"
        );
    }

    #[test]
    fn unsuccessful_unsafe_response_preserves_cached_get_representation() {
        let url = super::super::Url::parse("https://example.org/resource").unwrap();
        let get = Request::new(url.clone());
        let mut cache = HttpCache::default();
        cache.store(
            &get,
            &Response::new(200)
                .with_header("cache-control", "max-age=3600")
                .with_body(b"still valid".to_vec()),
        );

        let post = Request::new(url).with_method(HttpMethod::Post);
        cache.store(&post, &Response::new(500));

        assert_eq!(
            cache.get(&get).unwrap().body,
            b"still valid",
            "a 5xx response must not invalidate the cached representation"
        );
    }

    #[test]
    fn unsafe_method_invalidation_is_scoped_to_exact_target_uri() {
        let target_url = super::super::Url::parse("https://example.org/resource?a=1").unwrap();
        let other_url = super::super::Url::parse("https://example.org/resource?a=2").unwrap();
        let target = Request::new(target_url.clone());
        let other = Request::new(other_url.clone());
        let mut cache = HttpCache::default();
        for request in [&target, &other] {
            cache.store(
                request,
                &Response::new(200)
                    .with_header("cache-control", "max-age=3600")
                    .with_body(request.url.query().unwrap().as_bytes().to_vec()),
            );
        }

        cache.store(
            &Request::new(target_url).with_method(HttpMethod::Put),
            &Response::new(200),
        );

        assert!(cache.get(&target).is_none());
        assert_eq!(cache.get(&other).unwrap().body, b"a=2");
    }
}
