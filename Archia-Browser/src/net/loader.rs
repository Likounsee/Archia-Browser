use super::cache::request_only_if_cached;
use super::{
    pipeline::{NetworkPipeline, RequestPolicy, ResourceKind},
    HttpMethod, Request, Response, Transport, TransportError,
};
use crate::{
    document::Page,
    html::{parse, HtmlTokenizer, Node},
    layout::LayoutViewport,
};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentLoadError {
    Network(TransportError),
    HttpStatus(u16),
    UnsupportedContentType,
    RedirectLimitExceeded,
    RedirectLoopDetected,
    InvalidRedirect,
    NoCurrentDocument,
    InternalStatePoisoned,
    MemoryBudgetExceeded,
    DocumentTooLarge,
}

const MAX_REDIRECTS: usize = 10;
const MAX_HTML_DOCUMENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_STYLESHEET_BYTES_PER_DOCUMENT: usize = 2 * 1024 * 1024;
// Input bytes expand into several retained structures (DOM, styled tree, layout,
// and display list). These conservative multipliers are estimates, not exact heap
// measurements; keep them centralized so tests and future profiling can refine them.
const ESTIMATED_HTML_HEAP_MULTIPLIER: usize = 12;
const ESTIMATED_STYLESHEET_HEAP_MULTIPLIER: usize = 6;

pub struct DocumentLoader<P, T> {
    pipeline: NetworkPipeline<P>,
    transport: T,
    max_redirects: usize,
    cookies: Mutex<super::CookieJar>,
    cache: Mutex<super::HttpCache>,
    memory_budget: Option<Arc<crate::core::memory::MemoryBudget>>,
}

impl<P, T> DocumentLoader<P, T> {
    pub fn new(pipeline: NetworkPipeline<P>, transport: T) -> Self {
        Self {
            pipeline,
            transport,
            max_redirects: MAX_REDIRECTS,
            cookies: Mutex::new(super::CookieJar::new()),
            cache: Mutex::new(super::HttpCache::default()),
            memory_budget: None,
        }
    }

    pub const fn with_max_redirects(mut self, max_redirects: usize) -> Self {
        self.max_redirects = max_redirects;
        self
    }

    pub fn with_cache_capacity(mut self, capacity: usize) -> Self {
        let cache = match &self.memory_budget {
            Some(budget) => super::HttpCache::with_memory_budget(capacity, Arc::clone(budget)),
            None => super::HttpCache::new(capacity),
        };
        self.cache = Mutex::new(cache);
        self
    }

    /// Charge cached response storage to the same budget as browser-owned resources.
    pub fn with_shared_memory_budget(
        mut self,
        budget: Arc<crate::core::memory::MemoryBudget>,
    ) -> Self {
        let capacity = self
            .cache
            .get_mut()
            .map_or(16 * 1024 * 1024, |cache| cache.capacity_bytes());
        self.cache = Mutex::new(super::HttpCache::with_memory_budget(
            capacity,
            Arc::clone(&budget),
        ));
        self.memory_budget = Some(budget);
        self
    }

    /// Report the configured cache ceiling, or None if its lock is poisoned.
    pub fn cache_capacity_bytes(&self) -> Option<usize> {
        self.cache.lock().ok().map(|cache| cache.capacity_bytes())
    }
}

impl<P, T> DocumentLoader<P, T>
where
    P: super::pipeline::RequestPolicyEngine,
    T: Transport,
{
    pub fn load(
        &self,
        request: &Request,
        viewport: LayoutViewport,
    ) -> Result<Page, DocumentLoadError> {
        // A direct navigation without an explicit initiator is same-site
        // with its destination. Preserve an explicit first-party URL when the
        // caller supplies one, and reuse that context across redirect hops.
        let cookie_first_party = Some(
            request
                .policy
                .first_party
                .clone()
                .unwrap_or_else(|| request.url.clone()),
        );
        let mut current = request.clone();
        current.policy.resource_kind = ResourceKind::Document;
        current.policy.top_level_navigation = true;
        if current.policy.first_party.is_none() {
            current.policy.first_party = Some(current.url.clone());
        }
        // Rebuild the jar-managed Cookie header after the navigation context is
        // known, rather than attaching every matching cookie before SameSite
        // policy can be evaluated.
        remove_header_case_insensitive(&mut current.headers, "cookie");
        if let Some(cookie) = self
            .cookies
            .lock()
            .map_err(|_| DocumentLoadError::InternalStatePoisoned)?
            .header_for_context(
                &current.url,
                cookie_first_party.as_ref(),
                true,
                current.method,
            )
        {
            current.headers.insert("cookie".into(), cookie);
        }

        let mut visited_redirect_targets = HashSet::new();
        for redirect_count in 0..=self.max_redirects {
            // Fragments are not sent in HTTP request targets. Exclude them from
            // loop detection so a server cannot evade the guard by alternating
            // fragment-only redirects.
            if !visited_redirect_targets.insert(redirect_loop_key(&current.url)) {
                return Err(DocumentLoadError::RedirectLoopDetected);
            }

            // A cached response is still subject to the current request policy.
            // Never let cache hits bypass filtering decisions.
            self.pipeline
                .check_policy(&current)
                .map_err(DocumentLoadError::Network)?;

            let cached_response = {
                let mut cache = self
                    .cache
                    .lock()
                    .map_err(|_| DocumentLoadError::InternalStatePoisoned)?;
                cache.get(&current)
            };
            let response = if let Some(response) = cached_response {
                response
            } else if request_only_if_cached(&current) {
                // Cache-only requests must not fall through to the transport.
                // 504 is the HTTP-defined result when no usable cached response exists.
                return Err(DocumentLoadError::HttpStatus(504));
            } else {
                let response = self
                    .pipeline
                    .execute(&self.transport, &current)
                    .map_err(DocumentLoadError::Network)?;
                // Do not transiently retain an HTML document that the loader
                // will reject for exceeding its parser input limit. In particular,
                // avoid charging the shared cache budget for a response that cannot
                // become a page; the validation path below also invalidates any old
                // representation for the same key.
                // Cache only representations this loader can actually consume as
                // documents. Caching an arbitrary 200 response before MIME validation
                // can poison a later request for the same URL (for example, CSS served
                // to a navigation).
                if is_html_response(&response) && response.body.len() <= MAX_HTML_DOCUMENT_BYTES {
                    self.cache
                        .lock()
                        .map_err(|_| DocumentLoadError::InternalStatePoisoned)?
                        .store(&current, &response);
                }
                response
            };

            for set_cookie in response.set_cookie_headers() {
                self.cookies
                    .lock()
                    .map_err(|_| DocumentLoadError::InternalStatePoisoned)?
                    .store(&current.url, set_cookie);
            }

            if is_redirect(response.status) {
                if redirect_count == self.max_redirects {
                    return Err(DocumentLoadError::RedirectLimitExceeded);
                }

                let location = response
                    .header("location")
                    .filter(|value| !value.trim().is_empty())
                    .ok_or(DocumentLoadError::InvalidRedirect)?;
                let mut url = current
                    .url
                    .resolve(location)
                    .map_err(|_| DocumentLoadError::InvalidRedirect)?;
                // Per HTTP redirect semantics, a Location without a fragment
                // inherits the fragment from the original URL. An explicit
                // fragment in Location always takes precedence.
                if url.fragment().is_none() {
                    if let Some(fragment) = current.url.fragment() {
                        url = url
                            .resolve(&format!("#{fragment}"))
                            .map_err(|_| DocumentLoadError::InvalidRedirect)?;
                    }
                }

                // Do not allow redirects to cross the local-file/network trust
                // boundary in either direction. Remote responses must not open
                // local files, and a local document must not silently initiate a
                // network navigation through its redirect response.
                if (current.url.scheme() == "file") != (url.scheme() == "file") {
                    return Err(DocumentLoadError::InvalidRedirect);
                }
                // Do not silently downgrade a secure top-level navigation to
                // cleartext HTTP. Check every redirect hop, not just the initial URL.
                if current.url.is_secure() && url.scheme() == "http" {
                    return Err(DocumentLoadError::InvalidRedirect);
                }

                let mut next = current.clone();
                let had_referrer = has_header_case_insensitive(&next.headers, "referer")
                    || next.policy.referrer.is_some();
                let cross_origin = !same_origin(&current.url, &url);
                next.url = url;
                if had_referrer {
                    if let Some(referrer) = referrer_for_target(&current.url, &next.url) {
                        remove_header_case_insensitive(&mut next.headers, "referer");
                        next.headers.insert("referer".into(), referrer.to_string());
                        next.policy.referrer = Some(referrer);
                    } else {
                        remove_header_case_insensitive(&mut next.headers, "referer");
                        next.policy.referrer = None;
                    }
                }
                remove_header_case_insensitive(&mut next.headers, "cookie");
                if cross_origin {
                    // Do not forward arbitrary caller-supplied headers to another
                    // origin. Custom headers frequently carry API keys or session
                    // material and cannot be classified reliably by their names.
                    // Retain only common non-credential request metadata and the
                    // entity headers needed when a 307/308 preserves the request body.
                    retain_safe_cross_origin_headers(&mut next.headers);
                }
                if should_switch_to_get(current.method, response.status) {
                    next.method = HttpMethod::Get;
                    next.body.clear();
                    remove_header_case_insensitive(&mut next.headers, "content-length");
                    remove_header_case_insensitive(&mut next.headers, "content-type");
                }
                if let Some(cookie) = self
                    .cookies
                    .lock()
                    .map_err(|_| DocumentLoadError::InternalStatePoisoned)?
                    .header_for_context(&next.url, cookie_first_party.as_ref(), true, next.method)
                {
                    next.headers.insert("cookie".into(), cookie);
                }
                current = next;
                continue;
            }

            if !(200..300).contains(&response.status) {
                return Err(DocumentLoadError::HttpStatus(response.status));
            }

            if !is_html_response(&response) {
                return Err(DocumentLoadError::UnsupportedContentType);
            }

            // Reject rather than silently parse a truncated document. A partial
            // HTML tree can change security-relevant structure (including base URLs
            // and resource links), and the rejected response must not remain cached.
            if response.body.len() > MAX_HTML_DOCUMENT_BYTES {
                self.cache
                    .lock()
                    .map_err(|_| DocumentLoadError::InternalStatePoisoned)?
                    .invalidate(&current);
                return Err(DocumentLoadError::DocumentTooLarge);
            }
            let html_bytes = response.body.as_slice();

            // Reserve before UTF-8 decoding (which may allocate a replacement
            // string for invalid input) and before linked-stylesheet discovery
            // builds a temporary DOM. The multiplier estimates the retained DOM,
            // styled tree, layout, and display-list expansion; it is not an exact
            // heap measurement.
            let mut reservation = if let Some(budget) = &self.memory_budget {
                let estimated_html_bytes = html_bytes
                    .len()
                    .saturating_mul(ESTIMATED_HTML_HEAP_MULTIPLIER)
                    .max(1);
                let mut reservation = crate::core::memory::MemoryReservation::try_new(
                    Arc::clone(budget),
                    estimated_html_bytes,
                );
                if reservation.is_none() {
                    // Cached responses are reclaimable; retained page data takes
                    // priority over cache reuse when the shared budget is tight.
                    self.cache
                        .lock()
                        .map_err(|_| DocumentLoadError::InternalStatePoisoned)?
                        .clear();
                    reservation = crate::core::memory::MemoryReservation::try_new(
                        Arc::clone(budget),
                        estimated_html_bytes,
                    );
                }
                Some(reservation.ok_or(DocumentLoadError::MemoryBudgetExceeded)?)
            } else {
                None
            };

            let html = String::from_utf8_lossy(html_bytes);

            // Charge retained stylesheet bytes before the per-resource temporary
            // reservation is released, so the decoded CSS is never unaccounted for
            // during the hand-off into the document's accumulated stylesheet.
            let stylesheet =
                self.load_linked_stylesheets(&current.url, &html, reservation.as_ref());

            let mut page =
                Page::from_html_at(Some(current.url.clone()), &html, &stylesheet, viewport);
            if let Some(reservation) = reservation.take() {
                page.attach_memory_reservation(Arc::new(reservation));
            }

            return Ok(page);
        }

        Err(DocumentLoadError::RedirectLimitExceeded)
    }

    fn load_linked_stylesheets(
        &self,
        document_url: &super::Url,
        html: &str,
        document_reservation: Option<&crate::core::memory::MemoryReservation>,
    ) -> String {
        let document = parse(&HtmlTokenizer::tokenize(html));
        let mut links = Vec::new();
        collect_stylesheet_links(&document, &mut links);
        let base_url = document
            .find_first_element("base")
            .and_then(|base| base.attribute("href"))
            .map(str::trim)
            .filter(|href| !href.is_empty())
            .and_then(|href| document_url.resolve(href).ok())
            .unwrap_or_else(|| document_url.clone());

        let mut stylesheet = String::new();
        // Bound both network work and the expanded CSS retained for one
        // document. Imports share these budgets with linked stylesheets.
        let mut request_budget = 64;
        let mut remaining_css_bytes = MAX_STYLESHEET_BYTES_PER_DOCUMENT;
        for href in links {
            if request_budget == 0 || remaining_css_bytes == 0 {
                break;
            }
            let Ok(url) = base_url.resolve(&href) else {
                continue;
            };
            let Some(css) = self.load_stylesheet_resource(
                document_url,
                url,
                &mut request_budget,
                &mut remaining_css_bytes,
                document_reservation,
            ) else {
                continue;
            };
            if !stylesheet.is_empty() && remaining_css_bytes > 0 {
                stylesheet.push('\n');
                remaining_css_bytes = remaining_css_bytes.saturating_sub(1);
            }
            stylesheet.push_str(&css);
        }

        stylesheet
    }

    fn load_stylesheet_resource(
        &self,
        document_url: &super::Url,
        url: super::Url,
        request_budget: &mut usize,
        remaining_css_bytes: &mut usize,
        document_reservation: Option<&crate::core::memory::MemoryReservation>,
    ) -> Option<String> {
        // Keep each decoded resource's temporary charge alive through recursive
        // @import expansion. The top-level result is charged to the document
        // reservation before these temporary charges are released.
        let mut temporary_reservations = Vec::new();
        self.load_stylesheet_resource_at(
            document_url,
            url,
            0,
            request_budget,
            remaining_css_bytes,
            document_reservation,
            &mut temporary_reservations,
        )
    }

    #[allow(clippy::too_many_arguments)] // Recursive loading carries shared per-document budgets and reservations.
    fn load_stylesheet_resource_at(
        &self,
        document_url: &super::Url,
        url: super::Url,
        import_depth: usize,
        request_budget: &mut usize,
        remaining_css_bytes: &mut usize,
        document_reservation: Option<&crate::core::memory::MemoryReservation>,
        temporary_reservations: &mut Vec<crate::core::memory::MemoryReservation>,
    ) -> Option<String> {
        const MAX_IMPORT_DEPTH: usize = 8;
        if import_depth > MAX_IMPORT_DEPTH || *request_budget == 0 || *remaining_css_bytes == 0 {
            return None;
        }

        // A remote document must not load local files as CSS subresources,
        // including through a <base> element or @import.
        if document_url.scheme() != "file" && url.scheme() == "file" {
            return None;
        }
        // HTTPS documents must not load active stylesheets over cleartext HTTP.
        // Check the initial URL and every redirect hop to prevent mixed content.
        if document_url.is_secure() && url.scheme() == "http" {
            return None;
        }

        let mut current = url;
        let mut visited_redirect_targets = HashSet::new();

        // Keep stylesheet redirects under the same per-loader limit as document redirects.
        for _ in 0..=self.max_redirects {
            if *request_budget == 0 || !matches!(current.scheme(), "http" | "https" | "file") {
                return None;
            }
            if !visited_redirect_targets.insert(redirect_loop_key(&current)) {
                return None;
            }

            // Check every hop, not only the original URL: a remote server must
            // not redirect a stylesheet request into the local filesystem.
            if document_url.scheme() != "file" && current.scheme() == "file" {
                return None;
            }
            if document_url.is_secure() && current.scheme() == "http" {
                return None;
            }

            let mut request = Request::new(current.clone());
            let referrer = referrer_for_target(document_url, &current);
            request.policy = RequestPolicy {
                priority: super::pipeline::RequestPriority::Normal,
                resource_kind: ResourceKind::Stylesheet,
                referrer: referrer.clone(),
                first_party: Some(document_url.clone()),
                top_level_navigation: false,
            };
            request.headers.insert("accept".into(), "text/css".into());
            if let Some(referrer) = referrer {
                request
                    .headers
                    .insert("referer".into(), referrer.to_string());
            }
            if let Some(cookie) = self.cookies.lock().ok()?.header_for_context(
                &current,
                Some(document_url),
                false,
                HttpMethod::Get,
            ) {
                request.headers.insert("cookie".into(), cookie);
            }

            // Filtering must run even when this stylesheet is cached.
            self.pipeline.check_policy(&request).ok()?;

            *request_budget -= 1;
            let cached_response = {
                let mut cache = self.cache.lock().ok()?;
                cache.get(&request)
            };
            let response_was_cached = cached_response.is_some();
            let response = if let Some(response) = cached_response {
                response
            } else {
                let response = match self.pipeline.execute(&self.transport, &request) {
                    Ok(response) => response,
                    Err(_) => return None,
                };
                // Do not retain a stylesheet response that is already larger
                // than the remaining document CSS budget. The transport has a
                // broader generic response ceiling; caching this body here
                // would let one CSS resource consume far more than its share.
                // Do not retain a successful but unusable representation as a
                // stylesheet. In particular, an HTML response with a long freshness
                // lifetime must not poison later stylesheet loads for this URL.
                if is_css_response(&response) && response.body.len() <= *remaining_css_bytes {
                    self.cache.lock().ok()?.store(&request, &response);
                }
                response
            };

            for set_cookie in response.set_cookie_headers() {
                self.cookies.lock().ok()?.store(&current, set_cookie);
            }

            if is_redirect(response.status) {
                let location = response
                    .header("location")
                    .filter(|value| !value.trim().is_empty())?;
                let next = current.resolve(location).ok()?;
                // Keep stylesheet redirects within the document's local-file
                // or network trust domain, in both directions.
                if (current.scheme() == "file") != (next.scheme() == "file") {
                    return None;
                }
                current = next;
                continue;
            }

            // Network stylesheets require an explicit CSS MIME type. Local files
            // may not carry HTTP metadata, so keep their existing file-loading path.
            let implicit_local_css_type =
                current.scheme() == "file" && response.content_type.is_none();
            if !(200..300).contains(&response.status)
                || (!implicit_local_css_type && !is_css_response(&response))
            {
                return None;
            }

            // Bound the conversion's input before UTF-8 replacement can
            // expand malformed byte sequences into a much larger temporary
            // String. Charge that temporary allocation before conversion; the
            // final expanded stylesheet is charged separately by the document
            // reservation before the page tree is constructed.
            // A cached response must fit in full: parsing a truncated cached
            // representation would make cache behavior depend on request order.
            // A fresh oversized network response may still contribute a bounded
            // prefix, but is never retained in the cache above.
            if response_was_cached && response.body.len() > *remaining_css_bytes {
                if let Ok(mut cache) = self.cache.lock() {
                    cache.invalidate(&request);
                }
                return None;
            }
            let bounded_body = &response.body[..response.body.len().min(*remaining_css_bytes)];
            let temporary_css_reservation = if let Some(budget) = &self.memory_budget {
                let estimated_temporary_bytes = bounded_body.len().saturating_mul(4);
                let reservation = crate::core::memory::MemoryReservation::try_new(
                    Arc::clone(budget),
                    estimated_temporary_bytes,
                )
                .or_else(|| {
                    // Cache entries are reclaimable; temporary parsing work must
                    // take precedence over retaining responses when memory is tight.
                    self.cache.lock().ok()?.clear();
                    crate::core::memory::MemoryReservation::try_new(
                        Arc::clone(budget),
                        estimated_temporary_bytes,
                    )
                });
                let reservation = reservation?;
                Some(reservation)
            } else {
                None
            };
            if let Some(reservation) = temporary_css_reservation {
                temporary_reservations.push(reservation);
            }
            let css = String::from_utf8_lossy(bounded_body).into_owned();
            let expanded = self.expand_stylesheet_imports(
                document_url,
                &current,
                &css,
                import_depth,
                request_budget,
                remaining_css_bytes,
                document_reservation,
                temporary_reservations,
            );
            // Only top-level linked stylesheets charge the document reservation;
            // nested imports are included in that top-level expanded result.
            // Reserve before this resource's temporary charge is dropped.
            if import_depth == 0 {
                if let Some(reservation) = document_reservation {
                    if !reservation.try_grow_by(
                        expanded
                            .len()
                            .saturating_mul(ESTIMATED_STYLESHEET_HEAP_MULTIPLIER),
                    ) {
                        return None;
                    }
                }
            }
            return Some(expanded);
        }

        None
    }

    #[allow(clippy::too_many_arguments)] // Recursive imports share the same request, byte, and memory accounting.
    fn expand_stylesheet_imports(
        &self,
        document_url: &super::Url,
        stylesheet_url: &super::Url,
        css: &str,
        import_depth: usize,
        request_budget: &mut usize,
        remaining_css_bytes: &mut usize,
        document_reservation: Option<&crate::core::memory::MemoryReservation>,
        temporary_reservations: &mut Vec<crate::core::memory::MemoryReservation>,
    ) -> String {
        let mut output = String::with_capacity(css.len().min(*remaining_css_bytes));
        let mut statement_start = 0;
        while statement_start < css.len() {
            let Some((statement, next_statement_start)) = next_css_statement(css, statement_start)
            else {
                break;
            };
            statement_start = next_statement_start;
            let trimmed = statement.trim();
            let Some(reference) = parse_import_reference(trimmed) else {
                append_css_with_budget(&mut output, statement, remaining_css_bytes);
                if *remaining_css_bytes == 0 {
                    break;
                }
                continue;
            };

            if let Ok(url) = stylesheet_url.resolve(&reference) {
                if let Some(imported) = self.load_stylesheet_resource_at(
                    document_url,
                    url,
                    import_depth + 1,
                    request_budget,
                    remaining_css_bytes,
                    document_reservation,
                    temporary_reservations,
                ) {
                    output.push_str(&imported);
                    if *remaining_css_bytes > 0 {
                        output.push('\n');
                        *remaining_css_bytes = (*remaining_css_bytes).saturating_sub(1);
                    }
                    if *remaining_css_bytes == 0 {
                        break;
                    }
                    continue;
                }
            }
            append_css_with_budget(&mut output, statement, remaining_css_bytes);
            if *remaining_css_bytes == 0 {
                break;
            }
        }
        output
    }
}

/// Return the next CSS statement, ending at a semicolon outside strings and comments.
/// This prevents semicolons inside comments from making commented-out @import text
/// look like a real resource request.
fn next_css_statement(css: &str, start: usize) -> Option<(&str, usize)> {
    if start >= css.len() || !css.is_char_boundary(start) {
        return None;
    }

    let bytes = css.as_bytes();
    let mut index = start;
    let mut quote = None;
    let mut in_comment = false;
    let mut block_depth = 0_usize;

    while index < bytes.len() {
        let byte = bytes[index];
        let next = bytes.get(index + 1).copied();

        if in_comment {
            if byte == b'*' && next == Some(b'/') {
                in_comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }

        if let Some(active_quote) = quote {
            if byte == b'\\' {
                // CSS escapes protect the following byte from quote handling.
                index = (index + 2).min(bytes.len());
            } else {
                if byte == active_quote {
                    quote = None;
                }
                index += 1;
            }
            continue;
        }

        if byte == b'/' && next == Some(b'*') {
            in_comment = true;
            index += 2;
        } else if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
            index += 1;
        } else if byte == b'{' {
            block_depth = block_depth.saturating_add(1);
            index += 1;
        } else if byte == b'}' {
            block_depth = block_depth.saturating_sub(1);
            index += 1;
        } else if byte == b';' && block_depth == 0 {
            let end = index + 1;
            return Some((&css[start..end], end));
        } else {
            index += 1;
        }
    }

    Some((&css[start..], css.len()))
}

fn append_css_with_budget(output: &mut String, text: &str, remaining: &mut usize) {
    if *remaining == 0 {
        return;
    }
    let mut end = text.len().min(*remaining);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    output.push_str(&text[..end]);
    *remaining -= end;
}

fn has_header_case_insensitive(
    headers: &std::collections::BTreeMap<String, String>,
    name: &str,
) -> bool {
    headers.keys().any(|key| key.eq_ignore_ascii_case(name))
}

fn retain_safe_cross_origin_headers(headers: &mut std::collections::BTreeMap<String, String>) {
    headers.retain(|name, _| {
        [
            "accept",
            "accept-language",
            "cache-control",
            "content-length",
            "content-type",
            "dnt",
            "pragma",
            "referer",
            "user-agent",
            "upgrade-insecure-requests",
        ]
        .iter()
        .any(|safe| name.eq_ignore_ascii_case(safe))
    });
}

fn remove_header_case_insensitive(
    headers: &mut std::collections::BTreeMap<String, String>,
    name: &str,
) {
    headers.retain(|key, _| !key.eq_ignore_ascii_case(name));
}

fn collect_stylesheet_links(node: &Node, links: &mut Vec<String>) {
    if node.tag_name() == Some("link")
        && node.attribute("rel").is_some_and(|rel| {
            rel.split_whitespace()
                .any(|token| token.eq_ignore_ascii_case("stylesheet"))
        })
    {
        if let Some(href) = node
            .attribute("href")
            .map(str::trim)
            .filter(|href| !href.is_empty())
        {
            links.push(href.to_owned());
        }
    }

    for child in node.children() {
        collect_stylesheet_links(child, links);
    }
}

fn parse_import_reference(statement: &str) -> Option<String> {
    let statement = strip_css_comments(statement);
    parse_import_reference_without_comments(&statement)
}

/// Remove CSS comments without changing quoted strings or escaped quote handling.
/// Replacing comments with a space prevents adjacent tokens from being joined.
fn strip_css_comments(css: &str) -> String {
    let mut chars = css.chars().peekable();
    let mut output = String::with_capacity(css.len());
    let mut quote = None;

    while let Some(character) = chars.next() {
        if let Some(active_quote) = quote {
            output.push(character);
            if character == '\\' {
                if let Some(escaped) = chars.next() {
                    output.push(escaped);
                }
            } else if character == active_quote {
                quote = None;
            }
            continue;
        }

        if character == '/' && chars.peek() == Some(&'*') {
            chars.next();
            output.push(' ');
            while let Some(comment_character) = chars.next() {
                if comment_character == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
        } else {
            if matches!(character, '\'' | '"') {
                quote = Some(character);
            }
            output.push(character);
        }
    }

    output
}

fn parse_import_reference_without_comments(statement: &str) -> Option<String> {
    let statement = statement.trim();
    let import = statement.get(..7)?;
    if !import.eq_ignore_ascii_case("@import") {
        return None;
    }
    let remainder = &statement[7..];
    // Require a token boundary so malformed at-keywords such as @imported
    // cannot be interpreted as a resource URL.
    if !remainder.starts_with(char::is_whitespace)
        && !remainder.starts_with('"')
        && !remainder.starts_with('\'')
        && !remainder.to_ascii_lowercase().starts_with("url(")
    {
        return None;
    }
    let statement = remainder.trim_start();
    if !statement.ends_with(';') {
        return None;
    }
    let statement = statement[..statement.len() - 1].trim();
    if statement.contains('{') || statement.contains('}') {
        return None;
    }

    let reference = if statement
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("url("))
    {
        let value = &statement[4..];
        let end = value.find(')')?;
        if !value[end + 1..].trim().is_empty() {
            return None;
        }
        let value = value[..end].trim();
        if value.is_empty() {
            return None;
        }
        if let Some(quote) = value
            .chars()
            .next()
            .filter(|quote| matches!(quote, '"' | '\''))
        {
            if value.len() < 2 || !value.ends_with(quote) {
                return None;
            }
            let inner = &value[1..value.len() - 1];
            if inner.is_empty() || inner.contains(quote) {
                return None;
            }
            inner.to_owned()
        } else {
            if value.contains('"') || value.contains('\'') {
                return None;
            }
            value.to_owned()
        }
    } else {
        let quote = statement.chars().next()?;
        if !matches!(quote, '"' | '\'') || !statement.ends_with(quote) {
            return None;
        }
        let inner = &statement[1..statement.len() - 1];
        if inner.is_empty() || inner.contains(quote) {
            return None;
        }
        inner.to_owned()
    };

    (!reference.trim().is_empty()).then_some(reference)
}

fn is_css_response(response: &Response) -> bool {
    let Some(content_type) = response.content_type.as_deref() else {
        return false;
    };
    let media_type = content_type
        .split(';')
        .next()
        .map(str::trim)
        .unwrap_or_default();
    media_type.eq_ignore_ascii_case("text/css")
}

fn redirect_loop_key(url: &super::Url) -> String {
    let serialized = url.to_string();
    match serialized.split_once('#') {
        Some((without_fragment, _)) => without_fragment.to_owned(),
        None => serialized,
    }
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn same_origin(left: &super::Url, right: &super::Url) -> bool {
    left.same_origin(right)
}

/// Apply a conservative strict-origin-when-cross-origin referrer policy.
/// Never disclose a local file path to a network origin or send a secure
/// referrer over an insecure connection.
fn referrer_for_target(source: &super::Url, target: &super::Url) -> Option<super::Url> {
    if source.scheme() == "https" && target.scheme() == "http" {
        return None;
    }
    // Local-file URLs have opaque origins here; do not synthesize a referrer
    // from their paths or authority, including for file-to-file requests.
    if source.scheme() == "file" || target.scheme() == "file" {
        return None;
    }

    let reference = if same_origin(source, target) {
        source.to_string().split('#').next()?.to_owned()
    } else {
        format!("{}://{}/", source.scheme(), source.authority())
    };
    super::Url::parse(&reference).ok()
}

fn should_switch_to_get(method: HttpMethod, status: u16) -> bool {
    // Historically, user agents rewrite POST to GET for 301/302. Other
    // methods are preserved for those statuses; 303 switches every method
    // except HEAD to GET. Rewriting PUT/PATCH/DELETE on 301/302 can silently
    // turn a state-changing request into a different operation.
    match status {
        301 | 302 => method == HttpMethod::Post,
        303 => method != HttpMethod::Head,
        _ => false,
    }
}

fn is_html_response(response: &Response) -> bool {
    let Some(content_type) = response.content_type.as_deref() else {
        return true;
    };

    let media_type = content_type
        .split(';')
        .next()
        .map(str::trim)
        .unwrap_or_default();

    media_type.eq_ignore_ascii_case("text/html")
        || media_type.eq_ignore_ascii_case("application/xhtml+xml")
}

#[cfg(test)]
mod tests {

    #[test]
    fn redirect_method_rewriting_follows_status_specific_rules() {
        for status in [301, 302] {
            assert!(should_switch_to_get(HttpMethod::Post, status));
            for method in [
                HttpMethod::Get,
                HttpMethod::Head,
                HttpMethod::Put,
                HttpMethod::Delete,
                HttpMethod::Patch,
                HttpMethod::Options,
                HttpMethod::Connect,
            ] {
                assert!(
                    !should_switch_to_get(method, status),
                    "{method:?} must be preserved for status {status}"
                );
            }
        }

        for method in [
            HttpMethod::Get,
            HttpMethod::Post,
            HttpMethod::Put,
            HttpMethod::Delete,
            HttpMethod::Patch,
            HttpMethod::Options,
            HttpMethod::Connect,
        ] {
            assert!(
                should_switch_to_get(method, 303),
                "{method:?} should switch to GET for 303"
            );
        }
        assert!(!should_switch_to_get(HttpMethod::Head, 303));
        for status in [300, 304, 307, 308] {
            for method in [HttpMethod::Post, HttpMethod::Put, HttpMethod::Patch] {
                assert!(
                    !should_switch_to_get(method, status),
                    "{method:?} must be preserved for status {status}"
                );
            }
        }
    }

    #[test]
    fn secure_navigation_rejects_https_to_http_redirect_without_following_it() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        #[derive(Debug)]
        struct DowngradeTransport(Arc<AtomicUsize>);

        impl Transport for DowngradeTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                assert_eq!(request.url.scheme(), "https");
                Ok(Response::new(302).with_header("location", "http://example.org/insecure"))
            }
        }

        let sends = Arc::new(AtomicUsize::new(0));
        let loader = DocumentLoader::new(
            NetworkPipeline::new(AllowAll),
            DowngradeTransport(Arc::clone(&sends)),
        );
        let request = Request::new(Url::parse("https://example.org/secure").unwrap());

        assert!(matches!(
            loader.load(&request, LayoutViewport::new(320, 200)),
            Err(DocumentLoadError::InvalidRedirect)
        ));
        assert_eq!(
            sends.load(Ordering::SeqCst),
            1,
            "the cleartext redirect target must never reach the transport"
        );
    }

    use super::*;
    use crate::net::{
        pipeline::{PolicyDecision, RequestPolicyEngine},
        Response, Url,
    };

    #[test]
    fn oversized_html_is_rejected_and_not_retained_in_shared_cache() {
        use crate::core::memory::MemoryBudget;
        use std::sync::Arc;

        #[derive(Debug)]
        struct OversizedHtmlTransport;
        impl Transport for OversizedHtmlTransport {
            fn send(&self, _: &Request) -> Result<Response, TransportError> {
                Ok(Response::new(200)
                    .with_header("cache-control", "max-age=60")
                    .with_header("content-type", "text/html")
                    .with_body(vec![b'a'; MAX_HTML_DOCUMENT_BYTES + 1]))
            }
        }

        let budget = Arc::new(MemoryBudget::new(16 * 1024 * 1024));
        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), OversizedHtmlTransport)
            .with_shared_memory_budget(Arc::clone(&budget));
        let request = Request::new(Url::parse("https://example.org/oversized").unwrap());

        assert!(matches!(
            loader.load(&request, LayoutViewport::new(64, 32)),
            Err(DocumentLoadError::DocumentTooLarge)
        ));
        assert_eq!(
            budget.used(),
            0,
            "an oversized document must not remain charged to the cache budget"
        );
    }

    #[test]
    fn page_memory_budget_rejects_oversized_page_and_releases_cache_charge() {
        use crate::core::memory::MemoryBudget;
        use std::sync::Arc;

        #[derive(Debug)]
        struct SmallHtmlTransport;

        impl Transport for SmallHtmlTransport {
            fn send(&self, _: &Request) -> Result<Response, TransportError> {
                Ok(Response::new(200)
                    .with_header("cache-control", "max-age=60")
                    .with_header("content-type", "text/html")
                    .with_body(b"<html><body>budget</body></html>".to_vec()))
            }
        }

        let budget = Arc::new(MemoryBudget::new(64));
        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), SmallHtmlTransport)
            .with_shared_memory_budget(Arc::clone(&budget));
        let request = Request::new(Url::parse("https://example.org/").unwrap());

        assert!(matches!(
            loader.load(&request, LayoutViewport::new(64, 32)),
            Err(DocumentLoadError::MemoryBudgetExceeded)
        ));
        assert_eq!(
            budget.used(),
            0,
            "a rejected page must not leave its cache reservation charged"
        );
    }

    #[test]
    fn changing_cache_capacity_after_shared_budget_keeps_cache_accounted() {
        use crate::core::memory::MemoryBudget;
        use std::sync::Arc;

        #[derive(Debug)]
        struct CacheableTransport;
        impl Transport for CacheableTransport {
            fn send(&self, _: &Request) -> Result<Response, TransportError> {
                Ok(Response::new(200)
                    .with_header("cache-control", "max-age=60")
                    .with_header("content-type", "text/html")
                    .with_body(b"<html><body>Accounted</body></html>".to_vec()))
            }
        }

        let budget = Arc::new(MemoryBudget::new(4096));
        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), CacheableTransport)
            .with_shared_memory_budget(Arc::clone(&budget))
            .with_cache_capacity(2048);
        let request = Request::new(Url::parse("https://example.org/").unwrap());
        let page = loader.load(&request, LayoutViewport::new(64, 32)).unwrap();

        assert_eq!(loader.cache_capacity_bytes(), Some(2048));
        let retained_bytes = budget.used();
        assert!(retained_bytes > 0);
        drop(page);
        assert!(
            budget.used() > 0,
            "the cached response must keep its own reservation after the page is dropped"
        );
        assert!(budget.used() < retained_bytes);
    }

    #[test]
    fn redirect_loops_are_rejected_before_repeating_the_request() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        struct LoopTransport(Arc<AtomicUsize>);

        impl Transport for LoopTransport {
            fn send(&self, _: &Request) -> Result<Response, TransportError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(Response::new(302).with_header("location", "/loop#alternate"))
            }
        }

        let sends = Arc::new(AtomicUsize::new(0));
        let loader = DocumentLoader::new(
            NetworkPipeline::new(AllowAll),
            LoopTransport(Arc::clone(&sends)),
        );
        let request = Request::new(Url::parse("https://example.org/loop").unwrap());

        assert!(matches!(
            loader.load(&request, LayoutViewport::new(320, 200)),
            Err(DocumentLoadError::RedirectLoopDetected)
        ));
        assert_eq!(sends.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn poisoned_cookie_state_fails_closed_without_panicking() {
        use std::panic::{catch_unwind, AssertUnwindSafe};

        struct NeverTransport;
        impl Transport for NeverTransport {
            fn send(&self, _: &Request) -> Result<Response, TransportError> {
                panic!("transport must not be reached after state poisoning");
            }
        }

        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), NeverTransport);
        let poisoned = catch_unwind(AssertUnwindSafe(|| {
            let _guard = loader.cookies.lock().unwrap();
            panic!("simulate an interrupted cookie-state update");
        }));
        assert!(poisoned.is_err());

        let request = Request::new(Url::parse("https://example.org/").unwrap());
        assert!(matches!(
            loader.load(&request, LayoutViewport::new(320, 200)),
            Err(DocumentLoadError::InternalStatePoisoned)
        ));
    }

    #[test]
    fn only_if_cached_miss_returns_504_without_network_access() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        struct CountingTransport(Arc<AtomicUsize>);
        impl Transport for CountingTransport {
            fn send(&self, _: &Request) -> Result<Response, TransportError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_body(b"<html>network</html>".to_vec()))
            }
        }

        let sends = Arc::new(AtomicUsize::new(0));
        let loader = DocumentLoader::new(
            NetworkPipeline::new(AllowAll),
            CountingTransport(Arc::clone(&sends)),
        );
        let request = Request::new(Url::parse("https://example.org/missing").unwrap())
            .with_header("cache-control", "only-if-cached");

        assert!(matches!(
            loader.load(&request, LayoutViewport::new(320, 200)),
            Err(DocumentLoadError::HttpStatus(504))
        ));
        assert_eq!(sends.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn blocked_only_if_cached_request_is_denied_before_cache_fallback() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        struct BlockAll;
        impl RequestPolicyEngine for BlockAll {
            fn decide(&self, _: &Request) -> PolicyDecision {
                PolicyDecision::Block
            }
        }

        struct CountingTransport(Arc<AtomicUsize>);
        impl Transport for CountingTransport {
            fn send(&self, _: &Request) -> Result<Response, TransportError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_body(b"<html>must not be fetched</html>".to_vec()))
            }
        }

        let sends = Arc::new(AtomicUsize::new(0));
        let loader = DocumentLoader::new(
            NetworkPipeline::new(BlockAll),
            CountingTransport(Arc::clone(&sends)),
        );
        let request = Request::new(Url::parse("https://blocked.example/private").unwrap())
            .with_header("cache-control", "only-if-cached");

        assert!(matches!(
            loader.load(&request, LayoutViewport::new(320, 200)),
            Err(DocumentLoadError::Network(TransportError::BlockedByPolicy))
        ));
        assert_eq!(
            sends.load(Ordering::SeqCst),
            0,
            "policy denial must take precedence over cache-only fallback and transport"
        );
    }

    #[test]
    fn html_content_type_is_case_insensitive() {
        let html = Response::new(200).with_header("content-type", "TEXT/HTML; charset=UTF-8");
        let xhtml = Response::new(200).with_header("content-type", "Application/XHTML+XML");

        assert!(is_html_response(&html));
        assert!(is_html_response(&xhtml));
    }

    #[test]
    fn non_html_content_type_is_not_accepted_as_document() {
        let response = Response::new(200).with_header("content-type", "IMAGE/PNG");

        assert!(!is_html_response(&response));
    }

    #[test]
    fn stylesheet_byte_budget_preserves_utf8_and_stops_at_limit() {
        let mut output = String::new();
        let mut remaining = 5;
        append_css_with_budget(&mut output, "abécd", &mut remaining);
        append_css_with_budget(&mut output, "ignored", &mut remaining);

        assert_eq!(output, "abéc");
        assert_eq!(remaining, 0);
        assert!(output.len() <= 5);
    }

    #[test]
    fn stylesheet_decode_respects_a_tight_shared_memory_budget() {
        #[derive(Debug)]
        struct StylesheetTransport;

        impl Transport for StylesheetTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                assert_eq!(request.url.path(), "/style.css");
                Ok(Response::new(200)
                    .with_header("content-type", "text/css")
                    .with_header("cache-control", "public, max-age=3600")
                    .with_body(vec![b'a'; 512 * 1024]))
            }
        }

        let memory_budget = Arc::new(crate::core::memory::MemoryBudget::new(1024 * 1024));
        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), StylesheetTransport)
            .with_shared_memory_budget(Arc::clone(&memory_budget));
        let document_url = Url::parse("https://example.org/index.html").unwrap();

        let css = loader.load_linked_stylesheets(
            &document_url,
            r#"<link rel="stylesheet" href="/style.css">"#,
            None,
        );

        assert!(
            css.is_empty(),
            "CSS decoding must be skipped when its temporary allocation cannot be reserved"
        );
        assert_eq!(
            memory_budget.used(),
            0,
            "failed CSS decoding must release any cache or temporary reservations"
        );
    }

    #[test]
    fn cached_stylesheet_is_not_truncated_to_fit_remaining_document_budget() {
        #[derive(Debug)]
        struct ReusedStylesheetTransport;

        impl Transport for ReusedStylesheetTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                assert_eq!(request.url.path(), "/style.css");
                Ok(Response::new(200)
                    .with_header("content-type", "text/css")
                    .with_header("cache-control", "public, max-age=3600")
                    .with_body(vec![b'a'; 1536 * 1024]))
            }
        }

        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), ReusedStylesheetTransport);
        let document_url = Url::parse("https://example.org/index.html").unwrap();
        let css = loader.load_linked_stylesheets(
            &document_url,
            r#"<link rel="stylesheet" href="/style.css"><link rel="stylesheet" href="/style.css">"#,
            None,
        );

        assert_eq!(
            css.len(),
            1536 * 1024,
            "the second cached stylesheet must be rejected whole, not partially appended"
        );
    }

    #[test]
    fn linked_stylesheets_are_capped_at_the_document_byte_budget() {
        #[derive(Debug)]
        struct OversizedStylesheetTransport;

        impl Transport for OversizedStylesheetTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                assert_eq!(request.url.path(), "/style.css");
                Ok(Response::new(200)
                    .with_header("content-type", "text/css")
                    .with_header("cache-control", "public, max-age=3600")
                    .with_body(vec![b'a'; MAX_STYLESHEET_BYTES_PER_DOCUMENT + 128]))
            }
        }

        let memory_budget = Arc::new(crate::core::memory::MemoryBudget::new(8 * 1024 * 1024));
        let loader =
            DocumentLoader::new(NetworkPipeline::new(AllowAll), OversizedStylesheetTransport)
                .with_shared_memory_budget(Arc::clone(&memory_budget));
        let document_url = Url::parse("https://example.org/index.html").unwrap();
        let css = loader.load_linked_stylesheets(
            &document_url,
            r#"<link rel="stylesheet" href="/style.css">"#,
            None,
        );

        assert_eq!(css.len(), MAX_STYLESHEET_BYTES_PER_DOCUMENT);
        assert_eq!(
            memory_budget.used(),
            0,
            "an oversized stylesheet must not be retained in the shared-budget cache"
        );
    }

    #[test]
    fn stylesheet_redirects_cannot_downgrade_https_or_cross_into_local_files() {
        use std::sync::Mutex;

        #[derive(Debug)]
        struct RedirectingStylesheetTransport {
            target: String,
            requests: Mutex<Vec<String>>,
        }

        impl Transport for RedirectingStylesheetTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                self.requests.lock().unwrap().push(request.url.to_string());
                if request.policy.resource_kind == ResourceKind::Document {
                    return Ok(Response::new(200)
                        .with_header("content-type", "text/html")
                        .with_body(
                            br#"<link rel="stylesheet" href="/style.css"><body>Safe</body>"#
                                .to_vec(),
                        ));
                }
                assert_eq!(request.policy.resource_kind, ResourceKind::Stylesheet);
                Ok(Response::new(302).with_header("location", self.target.clone()))
            }
        }

        for target in ["http://example.org/insecure.css", "file:///etc/passwd"] {
            let transport = RedirectingStylesheetTransport {
                target: target.to_owned(),
                requests: Mutex::new(Vec::new()),
            };
            let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
            let request = Request::new(Url::parse("https://example.org/page").unwrap());
            let page = loader
                .load(&request, LayoutViewport::new(320, 200))
                .unwrap();

            assert_eq!(page.document.text_content(), "Safe");
            let requests = loader.transport.requests.lock().unwrap();
            assert_eq!(
                requests.len(),
                2,
                "the redirect target must never be requested: {target}"
            );
            assert_eq!(requests[0], "https://example.org/page");
            assert_eq!(requests[1], "https://example.org/style.css");
        }
    }

    #[test]
    fn retained_stylesheet_memory_stays_charged_for_the_page_lifetime() {
        use crate::core::memory::MemoryBudget;
        use std::sync::Arc;

        #[derive(Debug)]
        struct PageAndStylesheetTransport;
        impl Transport for PageAndStylesheetTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                if request.policy.resource_kind == ResourceKind::Document {
                    Ok(Response::new(200)
                        .with_header("content-type", "text/html")
                        .with_header("cache-control", "no-store")
                        .with_body(
                            br#"<link rel="stylesheet" href="/style.css"><body>Budget</body>"#
                                .to_vec(),
                        ))
                } else if request.url.path() == "/style.css" {
                    Ok(Response::new(200)
                        .with_header("content-type", "text/css")
                        .with_header("cache-control", "no-store")
                        .with_body(br#"@import url("/nested.css"); body { color: red; }"#.to_vec()))
                } else {
                    assert_eq!(request.url.path(), "/nested.css");
                    Ok(Response::new(200)
                        .with_header("content-type", "text/css")
                        .with_header("cache-control", "no-store")
                        .with_body(b"p { color: blue; }".to_vec()))
                }
            }
        }

        let budget = Arc::new(MemoryBudget::new(16 * 1024));
        let loader =
            DocumentLoader::new(NetworkPipeline::new(AllowAll), PageAndStylesheetTransport)
                .with_shared_memory_budget(Arc::clone(&budget));
        let request = Request::new(Url::parse("https://example.org/page").unwrap());
        let page = loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();

        assert_eq!(page.document.text_content(), "Budget");
        assert!(
            budget.used() > 512,
            "HTML plus retained stylesheet bytes must both be charged"
        );
        drop(page);
        assert_eq!(
            budget.used(),
            0,
            "dropping the page must release its HTML and retained CSS reservation"
        );
    }

    #[derive(Debug, Default)]
    struct AllowAll;

    impl RequestPolicyEngine for AllowAll {
        fn decide(&self, _: &Request) -> PolicyDecision {
            PolicyDecision::Allow
        }
    }

    #[derive(Debug)]
    struct MockTransport {
        response: Response,
    }

    impl Transport for MockTransport {
        fn send(&self, _: &Request) -> Result<Response, TransportError> {
            Ok(self.response.clone())
        }
    }

    #[test]
    fn file_urls_are_not_collapsed_into_a_shared_loader_origin() {
        let first = Url::parse("file:///private/first.html").unwrap();
        let second = Url::parse("file:///private/second.html").unwrap();
        assert!(!same_origin(&first, &second));
        assert!(referrer_for_target(&first, &second).is_none());
    }

    #[test]
    fn non_http_origin_comparison_distinguishes_missing_port_from_zero() {
        let implicit = Url::parse("custom://example.org/resource").unwrap();
        let explicit_zero = Url::parse("custom://example.org:0/resource").unwrap();
        assert!(!same_origin(&implicit, &explicit_zero));
    }

    #[test]
    fn referrers_are_reduced_for_cross_origin_requests_and_never_downgraded() {
        let source = Url::parse("https://example.org/private/page?token=secret#fragment").unwrap();
        let same_origin = Url::parse("https://example.org/other").unwrap();
        let cross_origin = Url::parse("https://cdn.example.net/style.css").unwrap();
        let downgrade = Url::parse("http://example.org/").unwrap();
        let local_file = Url::parse("file:///home/user/private.html").unwrap();

        assert_eq!(
            referrer_for_target(&source, &same_origin)
                .unwrap()
                .to_string(),
            "https://example.org/private/page?token=secret"
        );
        assert_eq!(
            referrer_for_target(&source, &cross_origin)
                .unwrap()
                .to_string(),
            "https://example.org/"
        );
        assert!(referrer_for_target(&source, &downgrade).is_none());
        assert!(referrer_for_target(&local_file, &cross_origin).is_none());
    }

    #[test]
    fn cached_documents_still_obey_current_request_policy() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;

        struct TogglePolicy(Arc<AtomicBool>);

        impl RequestPolicyEngine for TogglePolicy {
            fn decide(&self, _: &Request) -> PolicyDecision {
                if self.0.load(Ordering::SeqCst) {
                    PolicyDecision::Block
                } else {
                    PolicyDecision::Allow
                }
            }
        }

        struct CountingTransport(Arc<AtomicUsize>);

        impl Transport for CountingTransport {
            fn send(&self, _: &Request) -> Result<Response, TransportError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_header("cache-control", "max-age=60")
                    .with_body(b"<body>Cached page</body>".to_vec()))
            }
        }

        let blocked = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(AtomicUsize::new(0));
        let loader = DocumentLoader::new(
            NetworkPipeline::new(TogglePolicy(Arc::clone(&blocked))),
            CountingTransport(Arc::clone(&requests)),
        );
        let request = Request::new(Url::parse("https://example.org/cached").unwrap());
        let viewport = LayoutViewport::new(320, 200);

        assert!(loader.load(&request, viewport).is_ok());
        assert_eq!(requests.load(Ordering::SeqCst), 1);

        blocked.store(true, Ordering::SeqCst);
        assert!(matches!(
            loader.load(&request, viewport),
            Err(DocumentLoadError::Network(TransportError::BlockedByPolicy))
        ));
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cached_stylesheets_still_obey_current_request_policy() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;

        struct StyleTogglePolicy(Arc<AtomicBool>);

        impl RequestPolicyEngine for StyleTogglePolicy {
            fn decide(&self, request: &Request) -> PolicyDecision {
                if request.policy.resource_kind == ResourceKind::Stylesheet
                    && self.0.load(Ordering::SeqCst)
                {
                    PolicyDecision::Block
                } else {
                    PolicyDecision::Allow
                }
            }
        }

        struct CountingStylesheetTransport(Arc<AtomicUsize>);

        impl Transport for CountingStylesheetTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                if request.url.path() == "/page" {
                    Ok(Response::new(200)
                        .with_header("content-type", "text/html")
                        .with_header("cache-control", "max-age=60")
                        .with_body(
                            br#"<link rel="stylesheet" href="/style.css"><body>Page</body>"#
                                .to_vec(),
                        ))
                } else {
                    assert_eq!(request.url.path(), "/style.css");
                    Ok(Response::new(200)
                        .with_header("content-type", "text/css")
                        .with_header("cache-control", "max-age=60")
                        .with_body(b"body { color: red; }".to_vec()))
                }
            }
        }

        let block_styles = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(AtomicUsize::new(0));
        let loader = DocumentLoader::new(
            NetworkPipeline::new(StyleTogglePolicy(Arc::clone(&block_styles))),
            CountingStylesheetTransport(Arc::clone(&requests)),
        );
        let request = Request::new(Url::parse("https://example.org/page").unwrap());
        let viewport = LayoutViewport::new(320, 200);

        assert!(loader.load(&request, viewport).is_ok());
        assert_eq!(requests.load(Ordering::SeqCst), 2);

        block_styles.store(true, Ordering::SeqCst);
        assert!(loader.load(&request, viewport).is_ok());
        assert_eq!(
            requests.load(Ordering::SeqCst),
            2,
            "blocked cached stylesheet should not be fetched again"
        );
    }

    #[test]
    fn direct_navigation_sends_same_site_strict_cookies_without_explicit_initiator() {
        #[derive(Debug)]
        struct CookieInspectTransport;

        impl Transport for CookieInspectTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                assert_eq!(
                    request.headers.get("cookie").map(String::as_str),
                    Some("session=secret"),
                    "a direct navigation should use its destination as the first-party site"
                );
                Ok(Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_body(b"<body>Welcome</body>".to_vec()))
            }
        }

        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), CookieInspectTransport);
        let url = Url::parse("https://example.org/account").unwrap();
        loader
            .cookies
            .lock()
            .unwrap()
            .store(&url, "session=secret; Secure; SameSite=Strict");

        let request = Request::new(url);
        assert!(loader.load(&request, LayoutViewport::new(320, 200)).is_ok());
    }

    #[test]
    fn cross_site_top_level_navigation_sends_lax_but_not_strict_cookies() {
        #[derive(Debug)]
        struct CookieInspectTransport;

        impl Transport for CookieInspectTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                assert_eq!(
                    request.headers.get("cookie").map(String::as_str),
                    Some("lax=allowed"),
                    "cross-site safe navigation must exclude Strict cookies"
                );
                Ok(Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_body(b"<body>Welcome</body>".to_vec()))
            }
        }

        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), CookieInspectTransport);
        let destination = Url::parse("https://example.org/account").unwrap();
        let initiator = Url::parse("https://attacker.test/").unwrap();
        {
            let mut jar = loader.cookies.lock().unwrap();
            jar.store(&destination, "strict=blocked; Secure; SameSite=Strict");
            jar.store(&destination, "lax=allowed; Secure; SameSite=Lax");
        }

        let mut request = Request::new(destination);
        request.policy.first_party = Some(initiator);
        assert!(loader.load(&request, LayoutViewport::new(320, 200)).is_ok());
    }

    #[test]
    fn document_loads_are_marked_as_document_resources() {
        #[derive(Debug)]
        struct InspectTransport;

        impl Transport for InspectTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                assert_eq!(request.policy.resource_kind, ResourceKind::Document);
                Ok(Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_body(b"<body>Hello</body>".to_vec()))
            }
        }

        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), InspectTransport);
        let request = Request::new(Url::parse("https://example.org/").unwrap());
        assert!(loader.load(&request, LayoutViewport::new(320, 200)).is_ok());
    }

    #[test]
    fn remote_documents_cannot_load_file_url_stylesheets() {
        #[derive(Debug)]
        struct DocumentOnlyTransport;

        impl Transport for DocumentOnlyTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                assert_eq!(request.policy.resource_kind, ResourceKind::Document);
                assert_eq!(request.url.scheme(), "https");
                Ok(Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_body(
                        br#"<base href="file:///tmp/"><link rel="stylesheet" href="secret.css"><body>Safe</body>"#
                            .to_vec(),
                    ))
            }
        }

        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), DocumentOnlyTransport);
        let request = Request::new(Url::parse("https://example.org/").unwrap());
        let page = loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();

        assert_eq!(page.document.text_content(), "Safe");
    }

    #[test]
    fn loads_html_into_the_page_pipeline() {
        let response = Response::new(200)
            .with_header("content-type", "text/html; charset=utf-8")
            .with_body(b"<body><h1>Hello</h1></body>".to_vec());
        let loader =
            DocumentLoader::new(NetworkPipeline::new(AllowAll), MockTransport { response });
        let request = Request::new(Url::parse("https://example.org/").unwrap());
        let page = loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();

        assert_eq!(page.document.text_content(), "Hello");
        assert!(!page.display_list.commands().is_empty());
    }

    // Keep URL syntax independent from the platform's native path separator.
    // The URL parser intentionally rejects backslashes as ambiguous separators.
    fn local_file_test_url(path: &std::path::Path) -> String {
        let path = path.to_string_lossy().replace('\\', "/");
        if cfg!(windows) {
            format!("file:///{}", path)
        } else {
            format!("file://{}", path)
        }
    }

    #[test]
    fn loads_a_local_file_through_the_document_pipeline() {
        let path =
            std::env::temp_dir().join(format!("archia-browser-loader-{}.html", std::process::id()));
        std::fs::write(&path, b"<title>Local</title><body><h1>Hello</h1></body>").unwrap();

        let url = local_file_test_url(&path);
        let request = Request::new(Url::parse(&url).unwrap());
        let loader = DocumentLoader::new(
            NetworkPipeline::new(AllowAll),
            super::super::LocalFileTransport::for_file_url(&request.url).unwrap(),
        );
        let page = loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();

        assert_eq!(page.title(), Some("Local".to_owned()));
        assert_eq!(page.document.text_content(), "LocalHello");
        assert!(!page.display_list.commands().is_empty());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn network_stylesheets_without_css_mime_type_are_ignored() {
        #[derive(Debug)]
        struct SequenceTransport {
            responses: std::sync::Mutex<Vec<Response>>,
        }

        impl Transport for SequenceTransport {
            fn send(&self, _: &Request) -> Result<Response, TransportError> {
                Ok(self.responses.lock().unwrap().remove(0))
            }
        }

        let document = Response::new(200)
            .with_header("content-type", "text/html")
            .with_body(
                br#"<head><link rel="stylesheet" href="/plain.txt"></head><body>Hello</body>"#
                    .to_vec(),
            );
        // A successful HTTP response is not automatically a stylesheet.
        let mislabeled =
            Response::new(200).with_body(b"body { background-color: #102030; }".to_vec());
        let loader = DocumentLoader::new(
            NetworkPipeline::new(AllowAll),
            SequenceTransport {
                responses: std::sync::Mutex::new(vec![document, mislabeled]),
            },
        );
        let request = Request::new(Url::parse("https://example.org/index.html").unwrap());

        let page = loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();
        assert_eq!(page.document.text_content(), "Hello");
        assert!(
            !page.display_list.commands().iter().any(|command| {
                matches!(
                    command,
                    crate::render::PaintCommand::FillRect {
                        color: 0x102030ff,
                        ..
                    }
                )
            }),
            "a network response without text/css must not be applied as a stylesheet"
        );
    }

    #[test]
    fn unusable_stylesheet_responses_are_not_retained_in_cache() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        struct CountingTransport(Arc<AtomicUsize>);

        impl Transport for CountingTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                if request.policy.resource_kind == ResourceKind::Stylesheet {
                    self.0.fetch_add(1, Ordering::SeqCst);
                    return Ok(Response::new(200)
                        .with_header("content-type", "text/html")
                        .with_header("cache-control", "max-age=60")
                        .with_body(b"<html>not CSS</html>".to_vec()));
                }
                Ok(Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_header("cache-control", "max-age=60")
                    .with_body(
                        br#"<link rel="stylesheet" href="/wrong.css"><body>Page</body>"#.to_vec(),
                    ))
            }
        }

        let stylesheet_requests = Arc::new(AtomicUsize::new(0));
        let loader = DocumentLoader::new(
            NetworkPipeline::new(AllowAll),
            CountingTransport(Arc::clone(&stylesheet_requests)),
        );
        let request = Request::new(Url::parse("https://example.org/index.html").unwrap());
        let viewport = LayoutViewport::new(320, 200);

        loader.load(&request, viewport).unwrap();
        loader.load(&request, viewport).unwrap();

        assert_eq!(
            stylesheet_requests.load(Ordering::SeqCst),
            2,
            "a response with the wrong MIME type must not poison the stylesheet cache"
        );
    }

    #[test]
    fn loads_linked_stylesheet_into_page_styles() {
        #[derive(Debug)]
        struct SequenceTransport {
            responses: std::sync::Mutex<Vec<Response>>,
        }

        impl Transport for SequenceTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                if request.policy.resource_kind == ResourceKind::Stylesheet {
                    assert_eq!(request.url.to_string(), "https://example.org/css/site.css");
                    assert_eq!(
                        request.policy.referrer.as_ref().map(ToString::to_string),
                        Some("https://example.org/index.html".to_owned())
                    );
                    assert_eq!(
                        request.policy.first_party.as_ref().map(ToString::to_string),
                        Some("https://example.org/index.html".to_owned())
                    );
                }
                Ok(self.responses.lock().unwrap().remove(0))
            }
        }

        let document = Response::new(200)
            .with_header("content-type", "text/html")
            .with_body(
                br#"<head><link rel="stylesheet" href="/css/site.css"></head><body><div class="hero">Hello</div></body>"#
                    .to_vec(),
            );
        let stylesheet = Response::new(200)
            .with_header("content-type", "text/css; charset=utf-8")
            .with_body(b".hero { background-color: #102030; }".to_vec());

        let loader = DocumentLoader::new(
            NetworkPipeline::new(AllowAll),
            SequenceTransport {
                responses: std::sync::Mutex::new(vec![document, stylesheet]),
            },
        );
        let request = Request::new(Url::parse("https://example.org/index.html").unwrap());
        let page = loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();

        assert!(page.display_list.commands().iter().any(|command| {
            matches!(
                command,
                crate::render::PaintCommand::FillRect {
                    color: 0x102030ff,
                    ..
                }
            )
        }));
    }

    #[test]
    fn ignores_failed_linked_stylesheets_without_failing_document_load() {
        #[derive(Debug)]
        struct SequenceTransport {
            responses: std::sync::Mutex<Vec<Response>>,
        }

        impl Transport for SequenceTransport {
            fn send(&self, _: &Request) -> Result<Response, TransportError> {
                Ok(self.responses.lock().unwrap().remove(0))
            }
        }

        let document = Response::new(200)
            .with_header("content-type", "text/html")
            .with_body(
                br#"<head><link rel="stylesheet" href="/missing.css"></head><body>Hello</body>"#
                    .to_vec(),
            );
        let missing = Response::new(404).with_header("content-type", "text/css");

        let loader = DocumentLoader::new(
            NetworkPipeline::new(AllowAll),
            SequenceTransport {
                responses: std::sync::Mutex::new(vec![document, missing]),
            },
        );
        let request = Request::new(Url::parse("https://example.org/index.html").unwrap());

        let page = loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();
        assert_eq!(page.document.text_content(), "Hello");
    }

    #[test]
    fn expands_local_stylesheet_imports_relative_to_the_importing_file() {
        let root =
            std::env::temp_dir().join(format!("archia-browser-css-import-{}", std::process::id()));
        let nested = root.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let main = root.join("main.css");
        let theme = nested.join("theme.css");
        std::fs::write(&main, b"@import \"nested/theme.css\"; body { color: red; }").unwrap();
        std::fs::write(&theme, b"body { background-color: #102030; }").unwrap();

        let html = format!(
            "<head><link rel=\"stylesheet\" href=\"{}\"></head><body>Hello</body>",
            local_file_test_url(&main)
        );
        let document = Response::new(200)
            .with_header("content-type", "text/html")
            .with_body(html.into_bytes());
        let request = Request::new(Url::parse("file:///local/index.html").unwrap());

        struct RootTransport {
            document: Response,
            allowed_root: std::path::PathBuf,
        }
        impl Transport for RootTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                if request.policy.resource_kind == ResourceKind::Stylesheet {
                    return super::super::LocalFileTransport::new()
                        .with_allowed_roots([self.allowed_root.clone()])
                        .send(request);
                }
                Ok(self.document.clone())
            }
        }

        let loader = DocumentLoader::new(
            NetworkPipeline::new(AllowAll),
            RootTransport {
                document,
                allowed_root: root.clone(),
            },
        );
        let page = loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();

        assert!(page.display_list.commands().iter().any(|command| {
            matches!(
                command,
                crate::render::PaintCommand::DrawText {
                    color: 0xff0000ff,
                    ..
                }
            )
        }));
        assert!(page.display_list.commands().iter().any(|command| {
            matches!(
                command,
                crate::render::PaintCommand::FillRect {
                    color: 0x102030ff,
                    ..
                }
            )
        }));

        let _ = std::fs::remove_file(main);
        let _ = std::fs::remove_file(theme);
        let _ = std::fs::remove_dir(nested);
        let _ = std::fs::remove_dir(root);
    }

    #[test]
    fn skips_non_stylesheet_links() {
        let document = Response::new(200)
            .with_header("content-type", "text/html")
            .with_body(
                br#"<head><link rel="icon" href="/favicon.ico"></head><body>Hello</body>"#.to_vec(),
            );
        let loader = DocumentLoader::new(
            NetworkPipeline::new(AllowAll),
            MockTransport { response: document },
        );
        let request = Request::new(Url::parse("https://example.org/index.html").unwrap());
        let page = loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();
        assert_eq!(page.document.text_content(), "Hello");
    }

    #[test]
    fn resolves_linked_stylesheets_against_base_element() {
        #[derive(Debug)]
        struct SequenceTransport {
            requests: std::sync::Mutex<Vec<String>>,
        }

        impl Transport for SequenceTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                let mut requests = self.requests.lock().unwrap();
                requests.push(request.url.to_string());
                if request.policy.resource_kind == ResourceKind::Stylesheet {
                    Ok(Response::new(200)
                        .with_header("content-type", "text/css")
                        .with_body(b"body { color: red; }".to_vec()))
                } else {
                    Ok(Response::new(200)
                        .with_header("content-type", "text/html")
                        .with_body(
                            br#"<head><base href="/assets/"><link rel="stylesheet" href="site.css"></head><body>Hello</body>"#.to_vec(),
                        ))
                }
            }
        }

        let transport = SequenceTransport {
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
        let request = Request::new(Url::parse("https://example.org/index.html").unwrap());

        let page = loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();
        assert!(page.display_list.commands().iter().any(|command| {
            matches!(
                command,
                crate::render::PaintCommand::DrawText {
                    color: 0xff0000ff,
                    ..
                }
            )
        }));
    }

    #[test]
    fn refuses_stylesheet_redirects_from_remote_origins_to_file_urls() {
        #[derive(Debug)]
        struct RedirectToFileTransport {
            requests: std::sync::Mutex<Vec<String>>,
        }

        impl Transport for RedirectToFileTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                self.requests.lock().unwrap().push(request.url.to_string());
                if request.policy.resource_kind == ResourceKind::Document {
                    return Ok(Response::new(200)
                        .with_header("content-type", "text/html")
                        .with_body(
                            br#"<link rel="stylesheet" href="https://cdn.example/site.css"><body>Safe</body>"#
                                .to_vec(),
                        ));
                }

                assert_eq!(request.url.scheme(), "https");
                Ok(Response::new(302).with_header("location", "file:///tmp/private.css"))
            }
        }

        let transport = RedirectToFileTransport {
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
        let request = Request::new(Url::parse("https://example.org/").unwrap());
        let page = loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();

        assert_eq!(page.document.text_content(), "Safe");
        let requests = loader.transport.requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            2,
            "the file URL must never reach the transport"
        );
        assert!(requests.iter().all(|url| !url.starts_with("file:")));
    }

    #[test]
    fn stylesheet_redirects_respect_the_loader_redirect_limit() {
        use std::sync::Mutex;

        #[derive(Debug)]
        struct RedirectChainTransport {
            requests: Mutex<Vec<String>>,
        }

        impl Transport for RedirectChainTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                self.requests.lock().unwrap().push(request.url.to_string());
                if request.policy.resource_kind == ResourceKind::Document {
                    return Ok(Response::new(200)
                        .with_header("content-type", "text/html")
                        .with_body(
                            br#"<link rel="stylesheet" href="/style.css"><body>Safe</body>"#
                                .to_vec(),
                        ));
                }
                assert_eq!(request.url.path(), "/style.css");
                Ok(Response::new(302).with_header("location", "/next.css"))
            }
        }

        let transport = RedirectChainTransport {
            requests: Mutex::new(Vec::new()),
        };
        let loader =
            DocumentLoader::new(NetworkPipeline::new(AllowAll), transport).with_max_redirects(0);
        let request = Request::new(Url::parse("https://example.org/page").unwrap());

        let page = loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();
        assert_eq!(page.document.text_content(), "Safe");
        let requests = loader.transport.requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            2,
            "zero redirects must still allow the initial request"
        );
        assert_eq!(requests[1], "https://example.org/style.css");
    }

    #[test]
    fn linked_stylesheet_redirect_loops_stop_before_repeating_the_request() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        struct StylesheetLoopTransport(Arc<AtomicUsize>);

        impl Transport for StylesheetLoopTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                if request.policy.resource_kind == ResourceKind::Document {
                    return Ok(Response::new(200)
                        .with_header("content-type", "text/html")
                        .with_body(
                            br#"<link rel="stylesheet" href="/loop.css"><body>Safe</body>"#
                                .to_vec(),
                        ));
                }
                Ok(Response::new(302).with_header("location", "/loop.css#alternate"))
            }
        }

        let sends = Arc::new(AtomicUsize::new(0));
        let loader = DocumentLoader::new(
            NetworkPipeline::new(AllowAll),
            StylesheetLoopTransport(Arc::clone(&sends)),
        );
        let request = Request::new(Url::parse("https://example.org/page").unwrap());

        assert!(loader.load(&request, LayoutViewport::new(320, 200)).is_ok());
        assert_eq!(
            sends.load(Ordering::SeqCst),
            2,
            "the stylesheet redirect loop should stop after one stylesheet request"
        );
    }

    #[test]
    fn refuses_local_stylesheet_redirects_to_network_urls() {
        #[derive(Debug)]
        struct LocalToNetworkRedirectTransport {
            requests: std::sync::Mutex<Vec<String>>,
        }

        impl Transport for LocalToNetworkRedirectTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                self.requests.lock().unwrap().push(request.url.to_string());
                if request.policy.resource_kind == ResourceKind::Document {
                    return Ok(Response::new(200)
                        .with_header("content-type", "text/html")
                        .with_body(
                        br#"<link rel="stylesheet" href="file:///tmp/site.css"><body>Safe</body>"#
                            .to_vec(),
                    ));
                }

                assert_eq!(request.url.scheme(), "file");
                Ok(Response::new(302).with_header("location", "https://cdn.example/private.css"))
            }
        }

        let transport = LocalToNetworkRedirectTransport {
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
        let request = Request::new(Url::parse("file:///tmp/index.html").unwrap());
        let page = loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();

        assert_eq!(page.document.text_content(), "Safe");
        let requests = loader.transport.requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            2,
            "network redirect target must not be requested"
        );
        assert!(requests.iter().all(|url| url.starts_with("file:")));
    }

    #[test]
    fn follows_linked_stylesheet_redirects() {
        #[derive(Debug)]
        struct SequenceTransport {
            responses: std::sync::Mutex<Vec<Response>>,
        }

        impl Transport for SequenceTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                if request.policy.resource_kind == ResourceKind::Stylesheet {
                    assert_eq!(request.header("accept"), Some("text/css"));
                    assert_eq!(
                        request.header("referer"),
                        Some("https://example.org/index.html")
                    );
                    assert_eq!(
                        request.url.to_string(),
                        if request.url.path() == "/css/site.css" {
                            "https://example.org/css/site.css"
                        } else {
                            "https://example.org/css/final.css"
                        }
                    );
                }
                Ok(self.responses.lock().unwrap().remove(0))
            }
        }

        let document = Response::new(200)
            .with_header("content-type", "text/html")
            .with_body(
                br#"<head><link rel="stylesheet" href="/css/site.css"></head><body><div class="hero">Hello</div></body>"#
                    .to_vec(),
            );
        let stylesheet = Response::new(200)
            .with_header("content-type", "text/css")
            .with_body(b".hero { color: blue; }".to_vec());
        let loader = DocumentLoader::new(
            NetworkPipeline::new(AllowAll),
            SequenceTransport {
                responses: std::sync::Mutex::new(vec![
                    document,
                    Response::new(302).with_header("location", "/css/final.css"),
                    stylesheet,
                ]),
            },
        );
        let request = Request::new(Url::parse("https://example.org/index.html").unwrap());

        let page = loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();
        assert!(page.display_list.commands().iter().any(|command| {
            matches!(
                command,
                crate::render::PaintCommand::DrawText {
                    color: 0x0000ffff,
                    ..
                }
            )
        }));
    }

    #[test]
    fn redirect_inherits_fragment_unless_location_overrides_it() {
        #[derive(Debug)]
        struct RecordingTransport {
            responses: std::sync::Mutex<Vec<Response>>,
            urls: std::sync::Mutex<Vec<String>>,
        }

        impl Transport for RecordingTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                self.urls.lock().unwrap().push(request.url.to_string());
                Ok(self.responses.lock().unwrap().remove(0))
            }
        }

        for (location, expected) in [
            ("/final", "https://example.org/final#section"),
            (
                "/final#destination",
                "https://example.org/final#destination",
            ),
        ] {
            let transport = RecordingTransport {
                responses: std::sync::Mutex::new(vec![
                    Response::new(302).with_header("location", location),
                    Response::new(200)
                        .with_header("content-type", "text/html")
                        .with_body(b"<body>safe</body>".to_vec()),
                ]),
                urls: std::sync::Mutex::new(Vec::new()),
            };
            let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
            let request = Request::new(Url::parse("https://example.org/start#section").unwrap());

            loader
                .load(&request, LayoutViewport::new(320, 200))
                .unwrap();

            let urls = loader.transport.urls.lock().unwrap();
            assert_eq!(urls.len(), 2);
            assert_eq!(urls[1], expected);
        }
    }

    #[test]
    fn rejects_unexpected_http_status() {
        let response = Response::new(500);
        let loader =
            DocumentLoader::new(NetworkPipeline::new(AllowAll), MockTransport { response });
        let request = Request::new(Url::parse("https://example.org/").unwrap());

        assert!(matches!(
            loader.load(&request, LayoutViewport::new(320, 200)),
            Err(DocumentLoadError::HttpStatus(500))
        ));
    }

    #[test]
    fn strips_origin_credentials_on_cross_origin_redirects() {
        #[derive(Debug)]
        struct RecordingTransport {
            responses: std::sync::Mutex<Vec<Response>>,
            requests: std::sync::Mutex<Vec<Request>>,
        }

        impl Transport for RecordingTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                self.requests.lock().unwrap().push(request.clone());
                Ok(self.responses.lock().unwrap().remove(0))
            }
        }

        let transport = RecordingTransport {
            responses: std::sync::Mutex::new(vec![
                Response::new(302).with_header("location", "https://other.example/final"),
                Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_body(b"<body>safe</body>".to_vec()),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
        let request = Request::new(Url::parse("https://example.org/start").unwrap())
            .with_header("authorization", "Bearer secret")
            .with_header("proxy-authorization", "Basic secret")
            .with_header("host", "example.org");
        loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();

        let requests = loader.transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].header("authorization"), Some("Bearer secret"));
        assert_eq!(requests[1].header("authorization"), None);
        assert_eq!(requests[1].header("proxy-authorization"), None);
        assert_eq!(requests[1].header("host"), None);
    }

    #[test]
    fn strips_mixed_case_sensitive_headers_on_cross_origin_redirects() {
        #[derive(Debug)]
        struct RecordingTransport {
            responses: std::sync::Mutex<Vec<Response>>,
            requests: std::sync::Mutex<Vec<Request>>,
        }

        impl Transport for RecordingTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                self.requests.lock().unwrap().push(request.clone());
                Ok(self.responses.lock().unwrap().remove(0))
            }
        }

        let transport = RecordingTransport {
            responses: std::sync::Mutex::new(vec![
                Response::new(302).with_header("location", "https://other.example/final"),
                Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_body(b"<body>safe</body>".to_vec()),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
        let mut request = Request::new(Url::parse("https://example.org/start").unwrap());
        request
            .headers
            .insert("Authorization".into(), "Bearer secret".into());
        request
            .headers
            .insert("Proxy-Authorization".into(), "Basic secret".into());
        request.headers.insert("Host".into(), "example.org".into());
        request
            .headers
            .insert("Referer".into(), "https://example.org/private".into());
        loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();

        let requests = loader.transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        for name in ["authorization", "proxy-authorization", "host"] {
            assert!(
                !has_header_case_insensitive(&requests[1].headers, name),
                "mixed-case {name} must not survive a cross-origin redirect"
            );
        }
        assert_eq!(
            requests[1].header("referer"),
            Some("https://example.org/"),
            "a mixed-case Referer must be reduced to the origin, not leaked verbatim"
        );
    }

    #[test]
    fn rejects_https_to_http_redirect_without_sending_referrer_or_second_request() {
        #[derive(Debug)]
        struct DowngradeTransport {
            calls: std::sync::Mutex<usize>,
        }

        impl Transport for DowngradeTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                let mut calls = self.calls.lock().unwrap();
                *calls += 1;
                assert_eq!(request.url.scheme(), "https");
                Ok(Response::new(302).with_header("location", "http://other.example/landing"))
            }
        }

        let transport = DowngradeTransport {
            calls: std::sync::Mutex::new(0),
        };
        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
        let mut request = Request::new(Url::parse("https://example.org/secret-path").unwrap());
        request
            .headers
            .insert("referer".into(), "https://example.org/private-page".into());
        request.policy.referrer = Some(Url::parse("https://example.org/private-page").unwrap());

        assert!(matches!(
            loader.load(&request, LayoutViewport::new(320, 200)),
            Err(DocumentLoadError::InvalidRedirect)
        ));
        assert_eq!(
            *loader.transport.calls.lock().unwrap(),
            1,
            "the HTTP destination must not receive a request"
        );
    }

    #[test]
    fn strips_arbitrary_custom_headers_on_cross_origin_redirects() {
        #[derive(Debug)]
        struct RecordingTransport {
            responses: std::sync::Mutex<Vec<Response>>,
            requests: std::sync::Mutex<Vec<Request>>,
        }

        impl Transport for RecordingTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                self.requests.lock().unwrap().push(request.clone());
                Ok(self.responses.lock().unwrap().remove(0))
            }
        }

        let transport = RecordingTransport {
            responses: std::sync::Mutex::new(vec![
                Response::new(302).with_header("location", "https://other.example/final"),
                Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_body(b"<body>safe</body>".to_vec()),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
        let mut request = Request::new(Url::parse("https://example.org/start").unwrap());
        request
            .headers
            .insert("X-API-Key".into(), "secret-key".into());
        request
            .headers
            .insert("X-Private-Token".into(), "secret-token".into());
        request.headers.insert("Accept".into(), "text/html".into());
        request
            .headers
            .insert("Origin".into(), "https://example.org".into());

        loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();

        let requests = loader.transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("x-api-key") && value == "secret-key"
        }));
        assert!(requests[0].headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("x-private-token") && value == "secret-token"
        }));
        for name in ["x-api-key", "x-private-token", "origin"] {
            assert!(
                !has_header_case_insensitive(&requests[1].headers, name),
                "{name} must not cross an origin boundary"
            );
        }
        assert!(requests[1]
            .headers
            .iter()
            .any(|(name, value)| { name.eq_ignore_ascii_case("accept") && value == "text/html" }));
    }

    #[test]
    fn follows_relative_redirects() {
        let first = Response::new(302).with_header("location", "/next");
        let final_response = Response::new(200)
            .with_header("content-type", "text/html")
            .with_body(b"<body>redirected</body>".to_vec());

        #[derive(Debug)]
        struct SequenceTransport {
            responses: std::sync::Mutex<Vec<Response>>,
        }

        impl Transport for SequenceTransport {
            fn send(&self, _: &Request) -> Result<Response, TransportError> {
                Ok(self.responses.lock().unwrap().remove(0))
            }
        }

        let loader = DocumentLoader::new(
            NetworkPipeline::new(AllowAll),
            SequenceTransport {
                responses: std::sync::Mutex::new(vec![first, final_response]),
            },
        );
        let request = Request::new(Url::parse("https://example.org/start").unwrap());
        let page = loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();
        assert_eq!(page.document.text_content(), "redirected");
        assert_eq!(
            page.url().map(ToString::to_string),
            Some("https://example.org/next".to_owned())
        );
    }

    #[test]
    fn refuses_remote_document_redirects_to_local_files() {
        let response = Response::new(302).with_header("location", "file:///etc/passwd");
        let loader =
            DocumentLoader::new(NetworkPipeline::new(AllowAll), MockTransport { response });
        let request = Request::new(Url::parse("https://example.org/").unwrap());

        assert!(matches!(
            loader.load(&request, LayoutViewport::new(320, 200)),
            Err(DocumentLoadError::InvalidRedirect)
        ));
    }

    #[test]
    fn refuses_local_document_redirects_to_network_urls() {
        let response = Response::new(302).with_header("location", "https://example.org/remote");
        let loader =
            DocumentLoader::new(NetworkPipeline::new(AllowAll), MockTransport { response });
        let request = Request::new(Url::parse("file:///tmp/start.html").unwrap());

        assert!(matches!(
            loader.load(&request, LayoutViewport::new(320, 200)),
            Err(DocumentLoadError::InvalidRedirect)
        ));
    }

    #[test]
    fn rejects_redirect_without_location() {
        let response = Response::new(302);
        let loader =
            DocumentLoader::new(NetworkPipeline::new(AllowAll), MockTransport { response });
        let request = Request::new(Url::parse("https://example.org/").unwrap());
        assert!(matches!(
            loader.load(&request, LayoutViewport::new(320, 200)),
            Err(DocumentLoadError::InvalidRedirect)
        ));
    }

    #[test]
    fn enforces_redirect_limit() {
        let response = Response::new(302).with_header("location", "/loop");
        let loader =
            DocumentLoader::new(NetworkPipeline::new(AllowAll), MockTransport { response })
                .with_max_redirects(1);
        let request = Request::new(Url::parse("https://example.org/").unwrap());
        assert!(matches!(
            loader.load(&request, LayoutViewport::new(320, 200)),
            Err(DocumentLoadError::RedirectLimitExceeded)
        ));
    }

    #[test]
    fn rejects_non_html_content() {
        let response = Response::new(200).with_header("content-type", "image/png");
        let loader =
            DocumentLoader::new(NetworkPipeline::new(AllowAll), MockTransport { response });
        let request = Request::new(Url::parse("https://example.org/image.png").unwrap());

        assert!(matches!(
            loader.load(&request, LayoutViewport::new(320, 200)),
            Err(DocumentLoadError::UnsupportedContentType)
        ));
    }

    #[test]
    fn cross_origin_307_preserves_entity_but_strips_credentials_and_custom_headers() {
        #[derive(Debug)]
        struct RecordingTransport {
            responses: std::sync::Mutex<Vec<Response>>,
            requests: std::sync::Mutex<Vec<Request>>,
        }

        impl Transport for RecordingTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                self.requests.lock().unwrap().push(request.clone());
                Ok(self.responses.lock().unwrap().remove(0))
            }
        }

        let transport = RecordingTransport {
            responses: std::sync::Mutex::new(vec![
                Response::new(307).with_header("location", "https://other.example/submit"),
                Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_body(b"<body>submitted</body>".to_vec()),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
        let mut request = Request::new(Url::parse("https://example.org/submit").unwrap())
            .with_method(HttpMethod::Post)
            .with_body(b"private payload".to_vec())
            .with_header("content-type", "application/octet-stream")
            .with_header("authorization", "Bearer secret")
            .with_header("cookie", "sid=secret")
            .with_header("x-custom-secret", "secret")
            .with_header("accept", "text/html");
        request.headers.insert("content-length".into(), "15".into());

        loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();

        let requests = loader.transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, HttpMethod::Post);
        assert_eq!(requests[1].method, HttpMethod::Post);
        assert_eq!(requests[1].body, b"private payload");
        assert!(requests[1]
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("content-type")
                && value == "application/octet-stream"));
        assert!(requests[1]
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("content-length") && value == "15"));
        for name in ["authorization", "cookie", "x-custom-secret"] {
            assert!(
                !has_header_case_insensitive(&requests[1].headers, name),
                "{name} must not cross an origin boundary"
            );
        }
        assert!(requests[1]
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("accept") && value == "text/html"));
    }

    #[test]
    fn redirect_method_rewriting_matches_fetch_semantics() {
        assert!(should_switch_to_get(HttpMethod::Post, 301));
        assert!(should_switch_to_get(HttpMethod::Post, 302));
        assert!(!should_switch_to_get(HttpMethod::Put, 302));
        assert!(!should_switch_to_get(HttpMethod::Patch, 302));
        assert!(should_switch_to_get(HttpMethod::Patch, 303));
        assert!(!should_switch_to_get(HttpMethod::Get, 302));
        assert!(!should_switch_to_get(HttpMethod::Head, 303));
    }

    #[test]
    fn cross_origin_308_preserves_post_body_but_strips_credentials() {
        #[derive(Debug)]
        struct RecordingTransport {
            responses: std::sync::Mutex<Vec<Response>>,
            requests: std::sync::Mutex<Vec<Request>>,
        }

        impl Transport for RecordingTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                self.requests.lock().unwrap().push(request.clone());
                Ok(self.responses.lock().unwrap().remove(0))
            }
        }

        let transport = RecordingTransport {
            responses: std::sync::Mutex::new(vec![
                Response::new(308).with_header("location", "https://other.example/submit"),
                Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_body(b"<body>submitted</body>".to_vec()),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
        let mut request = Request::new(Url::parse("https://example.org/submit").unwrap())
            .with_method(HttpMethod::Post)
            .with_body(b"private payload".to_vec())
            .with_header("content-type", "application/octet-stream")
            .with_header("authorization", "Bearer secret")
            .with_header("cookie", "sid=secret")
            .with_header("x-custom-secret", "secret")
            .with_header("accept", "text/html");
        request.headers.insert("content-length".into(), "15".into());

        loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();

        let requests = loader.transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, HttpMethod::Post);
        assert_eq!(requests[1].method, HttpMethod::Post);
        assert_eq!(requests[1].body, b"private payload");
        assert!(requests[1]
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("content-type")
                && value == "application/octet-stream"));
        assert!(requests[1]
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("content-length") && value == "15"));
        for name in ["authorization", "cookie", "x-custom-secret"] {
            assert!(
                !has_header_case_insensitive(&requests[1].headers, name),
                "{name} must not cross a 308 origin boundary"
            );
        }
    }

    #[test]
    fn cross_origin_301_switches_post_to_get_and_removes_entity_headers() {
        #[derive(Debug)]
        struct RecordingTransport {
            responses: std::sync::Mutex<Vec<Response>>,
            requests: std::sync::Mutex<Vec<Request>>,
        }

        impl Transport for RecordingTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                self.requests.lock().unwrap().push(request.clone());
                Ok(self.responses.lock().unwrap().remove(0))
            }
        }

        let transport = RecordingTransport {
            responses: std::sync::Mutex::new(vec![
                Response::new(301).with_header("location", "https://other.example/landing"),
                Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_body(b"<body>landed</body>".to_vec()),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
        let request = Request::new(Url::parse("https://example.org/submit").unwrap())
            .with_method(HttpMethod::Post)
            .with_body(b"private payload".to_vec())
            .with_header("content-type", "application/octet-stream")
            .with_header("authorization", "Bearer secret")
            .with_header("x-custom-secret", "secret")
            .with_header("accept", "text/html");

        loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();

        let requests = loader.transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, HttpMethod::Post);
        assert_eq!(requests[1].method, HttpMethod::Get);
        assert!(requests[1].body.is_empty());
        for name in [
            "content-type",
            "content-length",
            "authorization",
            "x-custom-secret",
        ] {
            assert!(
                !has_header_case_insensitive(&requests[1].headers, name),
                "{name} must not survive this redirect"
            );
        }
        assert!(requests[1]
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("accept") && value == "text/html"));
    }

    #[test]
    fn cross_origin_303_switches_post_to_get_and_strips_credentials() {
        #[derive(Debug)]
        struct RecordingTransport {
            responses: std::sync::Mutex<Vec<Response>>,
            requests: std::sync::Mutex<Vec<Request>>,
        }

        impl Transport for RecordingTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                self.requests.lock().unwrap().push(request.clone());
                Ok(self.responses.lock().unwrap().remove(0))
            }
        }

        let transport = RecordingTransport {
            responses: std::sync::Mutex::new(vec![
                Response::new(303).with_header("location", "https://other.example/result"),
                Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_body(b"<body>result</body>".to_vec()),
            ]),
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
        let request = Request::new(Url::parse("https://example.org/submit").unwrap())
            .with_method(HttpMethod::Post)
            .with_body(b"private payload".to_vec())
            .with_header("content-type", "application/octet-stream")
            .with_header("authorization", "Bearer secret")
            .with_header("cookie", "sid=secret")
            .with_header("x-custom-secret", "secret")
            .with_header("accept", "text/html");

        loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();

        let requests = loader.transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, HttpMethod::Post);
        assert_eq!(requests[1].method, HttpMethod::Get);
        assert!(requests[1].body.is_empty());
        for name in [
            "content-type",
            "content-length",
            "authorization",
            "cookie",
            "x-custom-secret",
        ] {
            assert!(
                !has_header_case_insensitive(&requests[1].headers, name),
                "{name} must not survive a cross-origin 303"
            );
        }
        assert!(requests[1]
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("accept") && value == "text/html"));
    }

    #[test]
    fn cross_origin_307_and_308_preserve_entity_but_strip_credentials() {
        #[derive(Debug)]
        struct RedirectTransport {
            redirect_status: u16,
            requests: std::sync::Mutex<Vec<Request>>,
        }

        impl Transport for RedirectTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                let mut requests = self.requests.lock().unwrap();
                requests.push(request.clone());
                if requests.len() == 1 {
                    Ok(Response::new(self.redirect_status)
                        .with_header("location", "https://other.example/landing"))
                } else {
                    Ok(Response::new(200)
                        .with_header("content-type", "text/html")
                        .with_body(b"<html><body>landed</body></html>".to_vec()))
                }
            }
        }

        for status in [307, 308] {
            let transport = RedirectTransport {
                redirect_status: status,
                requests: std::sync::Mutex::new(Vec::new()),
            };
            let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
            let request = Request::new(Url::parse("https://example.org/submit").unwrap())
                .with_method(HttpMethod::Post)
                .with_body(b"payload must survive".to_vec())
                .with_header("content-type", "application/octet-stream")
                .with_header("authorization", "Bearer secret")
                .with_header("x-api-key", "private-key")
                .with_header("accept", "text/html");

            loader
                .load(&request, LayoutViewport::new(320, 200))
                .unwrap();

            let requests = loader.transport.requests.lock().unwrap();
            assert_eq!(requests.len(), 2, "status {status}");
            let redirected = &requests[1];
            assert_eq!(redirected.method, HttpMethod::Post, "status {status}");
            assert_eq!(redirected.body, b"payload must survive", "status {status}");
            assert!(
                has_header_case_insensitive(&redirected.headers, "content-type"),
                "entity metadata must survive status {status}"
            );
            for name in ["authorization", "x-api-key", "cookie"] {
                assert!(
                    !has_header_case_insensitive(&redirected.headers, name),
                    "{name} must not cross origins on status {status}"
                );
            }
            assert!(redirected
                .headers
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case("accept") && value == "text/html"));
        }
    }

    #[test]
    fn non_get_methods_are_preserved_for_307_and_308() {
        assert!(!should_switch_to_get(HttpMethod::Post, 307));
        assert!(!should_switch_to_get(HttpMethod::Put, 308));
        assert!(!should_switch_to_get(HttpMethod::Patch, 307));
    }

    #[test]
    fn secure_document_rejects_https_to_http_redirect_before_second_request() {
        #[derive(Debug)]
        struct RedirectTransport {
            requests: std::sync::Mutex<Vec<Request>>,
        }

        impl Transport for RedirectTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                self.requests.lock().unwrap().push(request.clone());
                Ok(Response::new(302).with_header("location", "http://example.org/landing"))
            }
        }

        let transport = RedirectTransport {
            requests: std::sync::Mutex::new(Vec::new()),
        };
        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
        let request = Request::new(Url::parse("https://example.org/start").unwrap());

        assert!(matches!(
            loader.load(&request, LayoutViewport::new(320, 200)),
            Err(DocumentLoadError::InvalidRedirect)
        ));
        assert_eq!(
            loader.transport.requests.lock().unwrap().len(),
            1,
            "the cleartext redirect target must never be requested"
        );
    }

    #[test]
    fn secure_documents_block_http_stylesheets_and_https_downgrade_redirects() {
        #[derive(Debug)]
        struct MixedContentTransport {
            stylesheet_href: &'static str,
        }

        impl Transport for MixedContentTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                assert_eq!(
                    request.url.scheme(),
                    "https",
                    "insecure request escaped guard"
                );
                if request.url.path() == "/" {
                    let html = format!(
                        "<html><head><link rel=\"stylesheet\" href=\"{}\"></head><body>Safe</body></html>",
                        self.stylesheet_href
                    );
                    return Ok(Response::new(200)
                        .with_header("content-type", "text/html")
                        .with_body(html.into_bytes()));
                }

                assert_eq!(request.url.path(), "/style.css");
                Ok(Response::new(302).with_header("location", "http://example.org/style.css"))
            }
        }

        for stylesheet_href in [
            "http://example.org/style.css",
            "https://example.org/style.css",
        ] {
            let loader = DocumentLoader::new(
                NetworkPipeline::new(AllowAll),
                MixedContentTransport { stylesheet_href },
            );
            let request = Request::new(Url::parse("https://example.org/").unwrap());
            let page = loader
                .load(&request, LayoutViewport::new(320, 200))
                .unwrap();
            assert_eq!(page.document.text_content(), "Safe");
        }
    }
    #[test]
    fn oversized_html_is_capped_before_decoding_and_parsing() {
        #[derive(Debug)]
        struct OversizedHtmlTransport;

        impl Transport for OversizedHtmlTransport {
            fn send(&self, _: &Request) -> Result<Response, TransportError> {
                let mut body = b"<html><body>retained-prefix</body></html>".to_vec();
                body.resize(MAX_HTML_DOCUMENT_BYTES + 1024, b' ');
                body.extend_from_slice(b"untrusted-tail-marker");
                Ok(Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_body(body))
            }
        }

        let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), OversizedHtmlTransport);
        let request = Request::new(Url::parse("https://example.org/large").unwrap());
        assert!(matches!(
            loader.load(&request, LayoutViewport::new(320, 200)),
            Err(DocumentLoadError::DocumentTooLarge)
        ));
    }

    #[test]
    fn css_statement_scanner_does_not_split_semicolons_inside_comments_or_strings() {
        let css = r#"/* ; @import "commented-out.css"; still a comment */ body::before { content: "a;b"; }"#;
        let mut statements = Vec::new();
        let mut start = 0;
        while start < css.len() {
            let (statement, next) = next_css_statement(css, start).unwrap();
            assert!(next > start);
            statements.push(statement);
            start = next;
        }

        assert_eq!(statements.len(), 1);
        assert!(
            statements
                .iter()
                .all(|statement| parse_import_reference(statement).is_none()),
            "commented-out @import text must never become a resource request"
        );
    }

    #[test]
    fn css_import_parser_rejects_malformed_and_empty_references() {
        for statement in [
            "@imported \"theme.css\";",
            "@import url();",
            "@import url(   );",
            "@import \"\";",
            "@import url(\"theme.css');",
            "@import url(theme.css) trailing;",
            "@import url(theme.css",
        ] {
            assert_eq!(
                parse_import_reference(statement),
                None,
                "malformed import must be ignored: {statement}"
            );
        }
    }

    #[test]
    fn css_import_parser_ignores_comments_but_preserves_quoted_comment_markers() {
        assert_eq!(
            parse_import_reference("/* header */ @import /* url */ url('theme.css');"),
            Some("theme.css".to_owned())
        );
        assert_eq!(
            parse_import_reference(r#"@import "theme/*not-a-comment*/.css";"#),
            Some("theme/*not-a-comment*/.css".to_owned())
        );
        assert_eq!(
            parse_import_reference("/* only a comment; */ body { color: red; }"),
            None
        );
    }

    #[test]
    fn css_import_parser_accepts_case_insensitive_keyword_and_url_function() {
        assert_eq!(
            parse_import_reference("@IMPORT URL(\"theme.css\");"),
            Some("theme.css".to_owned())
        );
        assert_eq!(
            parse_import_reference("@import 'nested.css';"),
            Some("nested.css".to_owned())
        );
        assert_eq!(
            parse_import_reference("@import url(https://example.org/theme.css);"),
            Some("https://example.org/theme.css".to_owned())
        );
    }
}
