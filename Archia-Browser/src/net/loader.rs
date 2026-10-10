use super::{
    pipeline::{NetworkPipeline, RequestPolicy, ResourceKind},
    HttpMethod, Request, Response, Transport, TransportError,
};
use crate::{
    document::Page,
    html::{parse, HtmlTokenizer, Node},
    layout::LayoutViewport,
};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentLoadError {
    Network(TransportError),
    HttpStatus(u16),
    UnsupportedContentType,
    RedirectLimitExceeded,
    InvalidRedirect,
    NoCurrentDocument,
}

const MAX_REDIRECTS: usize = 10;
const MAX_STYLESHEET_BYTES_PER_DOCUMENT: usize = 2 * 1024 * 1024;

pub struct DocumentLoader<P, T> {
    pipeline: NetworkPipeline<P>,
    transport: T,
    max_redirects: usize,
    cookies: Mutex<super::CookieJar>,
    cache: Mutex<super::HttpCache>,
}

impl<P, T> DocumentLoader<P, T> {
    pub fn new(pipeline: NetworkPipeline<P>, transport: T) -> Self {
        Self {
            pipeline,
            transport,
            max_redirects: MAX_REDIRECTS,
            cookies: Mutex::new(super::CookieJar::new()),
            cache: Mutex::new(super::HttpCache::default()),
        }
    }

    pub const fn with_max_redirects(mut self, max_redirects: usize) -> Self {
        self.max_redirects = max_redirects;
        self
    }

    pub fn with_cache_capacity(mut self, capacity: usize) -> Self {
        self.cache = Mutex::new(super::HttpCache::new(capacity));
        self
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
            .expect("cookie jar poisoned")
            .header_for_context(
                &current.url,
                cookie_first_party.as_ref(),
                true,
                current.method,
            )
        {
            current.headers.insert("cookie".into(), cookie);
        }

        for redirect_count in 0..=self.max_redirects {
            // A cached response is still subject to the current request policy.
            // Never let cache hits bypass filtering decisions.
            self.pipeline
                .check_policy(&current)
                .map_err(DocumentLoadError::Network)?;

            let cached_response = {
                let mut cache = self.cache.lock().expect("HTTP cache poisoned");
                cache.get(&current)
            };
            let response = if let Some(response) = cached_response {
                response
            } else {
                let response = self
                    .pipeline
                    .execute(&self.transport, &current)
                    .map_err(DocumentLoadError::Network)?;
                self.cache
                    .lock()
                    .expect("HTTP cache poisoned")
                    .store(&current, &response);
                response
            };

            for set_cookie in response.set_cookie_headers() {
                self.cookies
                    .lock()
                    .expect("cookie jar poisoned")
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

                // Remote content must never navigate the browser into the local
                // filesystem through an HTTP redirect.
                if current.url.scheme() != "file" && url.scheme() == "file" {
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
                    .expect("cookie jar poisoned")
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

            let html = String::from_utf8_lossy(&response.body);
            let stylesheet = self.load_linked_stylesheets(&current.url, &html);
            return Ok(Page::from_html_at(
                Some(current.url.clone()),
                &html,
                &stylesheet,
                viewport,
            ));
        }

        Err(DocumentLoadError::RedirectLimitExceeded)
    }

    fn load_linked_stylesheets(&self, document_url: &super::Url, html: &str) -> String {
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
    ) -> Option<String> {
        self.load_stylesheet_resource_at(document_url, url, 0, request_budget, remaining_css_bytes)
    }

    fn load_stylesheet_resource_at(
        &self,
        document_url: &super::Url,
        url: super::Url,
        import_depth: usize,
        request_budget: &mut usize,
        remaining_css_bytes: &mut usize,
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

        for _ in 0..=MAX_REDIRECTS {
            if *request_budget == 0 || !matches!(current.scheme(), "http" | "https" | "file") {
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
            };
            request.headers.insert("accept".into(), "text/css".into());
            if let Some(referrer) = referrer {
                request
                    .headers
                    .insert("referer".into(), referrer.to_string());
            }
            if let Some(cookie) = self
                .cookies
                .lock()
                .expect("cookie jar poisoned")
                .header_for_context(&current, Some(document_url), false, HttpMethod::Get)
            {
                request.headers.insert("cookie".into(), cookie);
            }

            // Filtering must run even when this stylesheet is cached.
            self.pipeline.check_policy(&request).ok()?;

            *request_budget -= 1;
            let cached_response = {
                let mut cache = self.cache.lock().expect("HTTP cache poisoned");
                cache.get(&request)
            };
            let response = if let Some(response) = cached_response {
                response
            } else {
                let response = match self.pipeline.execute(&self.transport, &request) {
                    Ok(response) => response,
                    Err(_) => return None,
                };
                self.cache
                    .lock()
                    .expect("HTTP cache poisoned")
                    .store(&request, &response);
                response
            };

            for set_cookie in response.set_cookie_headers() {
                self.cookies
                    .lock()
                    .expect("cookie jar poisoned")
                    .store(&current, set_cookie);
            }

            if is_redirect(response.status) {
                let location = response
                    .header("location")
                    .filter(|value| !value.trim().is_empty())?;
                current = current.resolve(location).ok()?;
                continue;
            }

            if !(200..300).contains(&response.status) || !is_css_response(&response) {
                return None;
            }

            let css = String::from_utf8_lossy(&response.body).into_owned();
            return Some(self.expand_stylesheet_imports(
                document_url,
                &current,
                &css,
                import_depth,
                request_budget,
                remaining_css_bytes,
            ));
        }

        None
    }

    fn expand_stylesheet_imports(
        &self,
        document_url: &super::Url,
        stylesheet_url: &super::Url,
        css: &str,
        import_depth: usize,
        request_budget: &mut usize,
        remaining_css_bytes: &mut usize,
    ) -> String {
        let mut output = String::with_capacity(css.len().min(*remaining_css_bytes));
        for statement in css.split_inclusive(';') {
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
    let statement = statement.trim();
    let statement = statement.strip_prefix("@import")?.trim_start();
    if !statement.ends_with(';') {
        return None;
    }
    let statement = statement[..statement.len() - 1].trim();
    if statement.contains('{') || statement.contains('}') {
        return None;
    }

    if let Some(value) = statement.strip_prefix("url(") {
        let end = value.find(')')?;
        if !value[end + 1..].trim().is_empty() {
            return None;
        }
        return Some(value[..end].trim().trim_matches(['"', '\'']).to_owned());
    }

    let quote = statement.chars().next()?;
    if !matches!(quote, '"' | '\'') || !statement.ends_with(quote) {
        return None;
    }
    Some(statement[1..statement.len() - 1].to_owned())
}

fn is_css_response(response: &Response) -> bool {
    let Some(content_type) = response.content_type.as_deref() else {
        return true;
    };
    let media_type = content_type
        .split(';')
        .next()
        .map(str::trim)
        .unwrap_or_default();
    media_type.eq_ignore_ascii_case("text/css")
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
    matches!(status, 301 | 302 | 303) && !matches!(method, HttpMethod::Get | HttpMethod::Head)
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

    use super::*;
    use crate::net::{
        pipeline::{PolicyDecision, RequestPolicyEngine},
        Response, Url,
    };

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
    fn linked_stylesheets_are_capped_at_the_document_byte_budget() {
        #[derive(Debug)]
        struct OversizedStylesheetTransport;

        impl Transport for OversizedStylesheetTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                assert_eq!(request.url.path(), "/style.css");
                Ok(Response::new(200)
                    .with_header("content-type", "text/css")
                    .with_body(vec![b'a'; MAX_STYLESHEET_BYTES_PER_DOCUMENT + 128]))
            }
        }

        let loader =
            DocumentLoader::new(NetworkPipeline::new(AllowAll), OversizedStylesheetTransport);
        let document_url = Url::parse("https://example.org/index.html").unwrap();
        let css = loader.load_linked_stylesheets(
            &document_url,
            r#"<link rel="stylesheet" href="/style.css">"#,
        );

        assert_eq!(css.len(), MAX_STYLESHEET_BYTES_PER_DOCUMENT);
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
            super::super::LocalFileTransport::new(),
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
        let loader = DocumentLoader::new(
            NetworkPipeline::new(AllowAll),
            super::super::LocalFileTransport::new(),
        );
        let request = Request::new(Url::parse("file:///local/index.html").unwrap());

        struct RootTransport {
            document: Response,
        }
        impl Transport for RootTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                if request.policy.resource_kind == ResourceKind::Stylesheet {
                    return super::super::LocalFileTransport::new().send(request);
                }
                Ok(self.document.clone())
            }
        }

        let loader =
            DocumentLoader::new(NetworkPipeline::new(AllowAll), RootTransport { document });
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
    fn strips_referrer_on_https_to_http_redirects() {
        #[derive(Debug)]
        struct DowngradeTransport {
            calls: std::sync::Mutex<usize>,
        }

        impl Transport for DowngradeTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                let mut calls = self.calls.lock().unwrap();
                *calls += 1;
                if *calls == 1 {
                    assert_eq!(request.url.scheme(), "https");
                    return Ok(
                        Response::new(302).with_header("location", "http://other.example/landing")
                    );
                }

                assert_eq!(request.url.scheme(), "http");
                assert!(request.header("referer").is_none());
                assert!(request.policy.referrer.is_none());
                Ok(Response::new(200)
                    .with_header("content-type", "text/html")
                    .with_body(b"<body>Safe</body>".to_vec()))
            }
        }

        let loader = DocumentLoader::new(
            NetworkPipeline::new(AllowAll),
            DowngradeTransport {
                calls: std::sync::Mutex::new(0),
            },
        );
        let mut request = Request::new(Url::parse("https://example.org/secret-path").unwrap());
        request
            .headers
            .insert("referer".into(), "https://example.org/private-page".into());
        request.policy.referrer = Some(Url::parse("https://example.org/private-page").unwrap());

        let page = loader
            .load(&request, LayoutViewport::new(320, 200))
            .unwrap();
        assert_eq!(page.document.text_content(), "Safe");
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
    fn post_switches_to_get_for_301_302_and_303() {
        assert!(should_switch_to_get(HttpMethod::Post, 301));
        assert!(should_switch_to_get(HttpMethod::Put, 302));
        assert!(should_switch_to_get(HttpMethod::Patch, 303));
        assert!(!should_switch_to_get(HttpMethod::Get, 302));
        assert!(!should_switch_to_get(HttpMethod::Head, 303));
    }

    #[test]
    fn non_get_methods_are_preserved_for_307_and_308() {
        assert!(!should_switch_to_get(HttpMethod::Post, 307));
        assert!(!should_switch_to_get(HttpMethod::Put, 308));
        assert!(!should_switch_to_get(HttpMethod::Patch, 307));
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
}
