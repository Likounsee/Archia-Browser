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

pub struct DocumentLoader<P, T> {
    pipeline: NetworkPipeline<P>,
    transport: T,
    max_redirects: usize,
    cookies: Mutex<super::CookieJar>,
}

impl<P, T> DocumentLoader<P, T> {
    pub fn new(pipeline: NetworkPipeline<P>, transport: T) -> Self {
        Self {
            pipeline,
            transport,
            max_redirects: MAX_REDIRECTS,
            cookies: Mutex::new(super::CookieJar::new()),
        }
    }

    pub const fn with_max_redirects(mut self, max_redirects: usize) -> Self {
        self.max_redirects = max_redirects;
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
        let mut current = request
            .clone()
            .with_cookies(&self.cookies.lock().expect("cookie jar poisoned"));
        if current.policy.first_party.is_none() {
            current.policy.first_party = Some(current.url.clone());
        }

        for redirect_count in 0..=self.max_redirects {
            let response = self
                .pipeline
                .execute(&self.transport, &current)
                .map_err(DocumentLoadError::Network)?;

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
                let url = current
                    .url
                    .resolve(location)
                    .map_err(|_| DocumentLoadError::InvalidRedirect)?;

                let mut next = current.clone();
                next.url = url;
                next.headers.remove("cookie");
                if let Some(cookie) = self
                    .cookies
                    .lock()
                    .expect("cookie jar poisoned")
                    .header_for(&next.url)
                {
                    next.headers.insert("cookie".into(), cookie);
                }

                if should_switch_to_get(current.method, response.status) {
                    next.method = HttpMethod::Get;
                    next.body.clear();
                    next.headers.remove("content-length");
                    next.headers.remove("content-type");
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

        let mut stylesheet = String::new();
        for href in links {
            let Ok(url) = document_url.resolve(&href) else {
                continue;
            };
            let Some(css) = self.load_stylesheet_resource(document_url, url) else {
                continue;
            };
            if !stylesheet.is_empty() {
                stylesheet.push('\n');
            }
            stylesheet.push_str(&css);
        }

        stylesheet
    }

    fn load_stylesheet_resource(
        &self,
        document_url: &super::Url,
        url: super::Url,
    ) -> Option<String> {
        let mut current = url;

        for _ in 0..=MAX_REDIRECTS {
            if !matches!(current.scheme(), "http" | "https" | "file") {
                return None;
            }

            let mut request = Request::new(current.clone());
            request.policy = RequestPolicy {
                priority: super::pipeline::RequestPriority::Normal,
                resource_kind: ResourceKind::Stylesheet,
                referrer: Some(document_url.clone()),
                first_party: Some(document_url.clone()),
            };
            if let Some(cookie) = self
                .cookies
                .lock()
                .expect("cookie jar poisoned")
                .header_for(&current)
            {
                request.headers.insert("cookie".into(), cookie);
            }

            let Ok(response) = self.pipeline.execute(&self.transport, &request) else {
                return None;
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

            return Some(String::from_utf8_lossy(&response.body).into_owned());
        }

        None
    }
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

    matches!(media_type, "text/html" | "application/xhtml+xml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{
        pipeline::{PolicyDecision, RequestPolicyEngine},
        Response, Url,
    };

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

    #[test]
    fn loads_a_local_file_through_the_document_pipeline() {
        let path =
            std::env::temp_dir().join(format!("archia-browser-loader-{}.html", std::process::id()));
        std::fs::write(&path, b"<title>Local</title><body><h1>Hello</h1></body>").unwrap();

        let url = if cfg!(windows) {
            format!("file:///{}", path.display())
        } else {
            format!("file://{}", path.display())
        };
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
    fn follows_linked_stylesheet_redirects() {
        #[derive(Debug)]
        struct SequenceTransport {
            responses: std::sync::Mutex<Vec<Response>>,
        }

        impl Transport for SequenceTransport {
            fn send(&self, request: &Request) -> Result<Response, TransportError> {
                if request.policy.resource_kind == ResourceKind::Stylesheet {
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
}
