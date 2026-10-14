use std::sync::{Arc, Mutex};

use archia_browser::{
    layout::LayoutViewport,
    net::{
        loader::{DocumentLoadError, DocumentLoader},
        pipeline::{NetworkPipeline, PolicyDecision, RequestPolicyEngine},
        HttpMethod, Request, Response, Transport, TransportError, Url,
    },
};

struct AllowAll;

impl RequestPolicyEngine for AllowAll {
    fn decide(&self, _: &Request) -> PolicyDecision {
        PolicyDecision::Allow
    }
}

struct BlockDeniedHost;

impl RequestPolicyEngine for BlockDeniedHost {
    fn decide(&self, request: &Request) -> PolicyDecision {
        if request.url.host() == "blocked.example" {
            PolicyDecision::Block
        } else {
            PolicyDecision::Allow
        }
    }
}

struct RedirectTransport {
    responses: Mutex<Vec<Response>>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Transport for RedirectTransport {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        self.requests
            .lock()
            .expect("request log mutex poisoned")
            .push(request.url.to_string());
        let mut responses = self
            .responses
            .lock()
            .expect("response queue mutex poisoned");
        if responses.is_empty() {
            return Err(TransportError::ConnectionFailed);
        }
        Ok(responses.remove(0))
    }
}

#[test]
fn fragment_only_redirects_cannot_evade_loop_detection() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let transport = RedirectTransport {
        responses: Mutex::new(vec![
            Response::new(302).with_header("location", "/next#first"),
            Response::new(302).with_header("location", "#second"),
        ]),
        requests: Arc::clone(&requests),
    };
    let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
    let request =
        Request::new(Url::parse("https://example.org/start").unwrap()).with_method(HttpMethod::Get);

    let result = loader.load(&request, LayoutViewport::new(320, 200));

    assert!(matches!(
        result,
        Err(DocumentLoadError::RedirectLoopDetected)
    ));
    let requests = requests.lock().expect("request log mutex poisoned");
    assert_eq!(
        requests.len(),
        2,
        "loop must be stopped before a third request"
    );
    assert_eq!(requests[1], "https://example.org/next#first");
}

#[test]
fn redirect_target_is_rechecked_by_policy_before_transport() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let transport = RedirectTransport {
        responses: Mutex::new(vec![
            Response::new(302).with_header("location", "https://blocked.example/private")
        ]),
        requests: Arc::clone(&requests),
    };
    let loader = DocumentLoader::new(NetworkPipeline::new(BlockDeniedHost), transport);
    let request = Request::new(Url::parse("https://example.org/start").unwrap());

    let result = loader.load(&request, LayoutViewport::new(320, 200));

    assert!(matches!(
        result,
        Err(DocumentLoadError::Network(TransportError::BlockedByPolicy))
    ));
    let requests = requests.lock().expect("request log mutex poisoned");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0], "https://example.org/start");
}

struct RequestRecordingTransport {
    responses: Mutex<Vec<Response>>,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl Transport for RequestRecordingTransport {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        self.requests
            .lock()
            .expect("request log mutex poisoned")
            .push(request.clone());
        let mut responses = self
            .responses
            .lock()
            .expect("response queue mutex poisoned");
        if responses.is_empty() {
            return Err(TransportError::ConnectionFailed);
        }
        Ok(responses.remove(0))
    }
}

fn html_response() -> Response {
    Response::new(200)
        .with_header("content-type", "text/html; charset=utf-8")
        .with_body(b"<!doctype html><html><body>ok</body></html>".to_vec())
}

#[test]
fn cross_origin_redirect_does_not_forward_credentials_or_custom_headers() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let transport = RequestRecordingTransport {
        responses: Mutex::new(vec![
            Response::new(302).with_header("location", "https://other.example/private"),
            html_response(),
        ]),
        requests: Arc::clone(&requests),
    };
    let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
    let mut request = Request::new(Url::parse("https://example.org/start").unwrap());
    request
        .headers
        .insert("authorization".into(), "Bearer secret".into());
    request
        .headers
        .insert("x-api-key".into(), "secret-key".into());
    request
        .headers
        .insert("accept-language".into(), "fr-FR".into());

    loader
        .load(&request, LayoutViewport::new(320, 200))
        .expect("redirected HTML document should load");

    let requests = requests.lock().expect("request log mutex poisoned");
    assert_eq!(requests.len(), 2);
    let redirected = &requests[1].headers;
    assert!(!redirected.contains_key("authorization"));
    assert!(!redirected.contains_key("x-api-key"));
    assert_eq!(
        redirected.get("accept-language").map(String::as_str),
        Some("fr-FR")
    );
}

#[test]
fn proxy_authorization_is_removed_even_on_same_origin_redirects() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let transport = RequestRecordingTransport {
        responses: Mutex::new(vec![
            Response::new(302).with_header("location", "/next"),
            html_response(),
        ]),
        requests: Arc::clone(&requests),
    };
    let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
    let mut request = Request::new(Url::parse("https://example.org/start").unwrap());
    request
        .headers
        .insert("proxy-authorization".into(), "Basic secret".into());

    loader
        .load(&request, LayoutViewport::new(320, 200))
        .expect("redirected HTML document should load");

    let requests = requests.lock().expect("request log mutex poisoned");
    assert_eq!(requests.len(), 2);
    assert!(!requests[1].headers.contains_key("proxy-authorization"));
}

#[test]
fn post_redirect_to_same_origin_302_becomes_bodyless_get() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let transport = RequestRecordingTransport {
        responses: Mutex::new(vec![
            Response::new(302).with_header("location", "/after-submit"),
            html_response(),
        ]),
        requests: Arc::clone(&requests),
    };
    let loader = DocumentLoader::new(NetworkPipeline::new(AllowAll), transport);
    let mut request = Request::new(Url::parse("https://example.org/submit").unwrap())
        .with_method(HttpMethod::Post);
    request.body = b"secret=payload".to_vec();
    request
        .headers
        .insert("content-type".into(), "application/x-www-form-urlencoded".into());
    request
        .headers
        .insert("content-length".into(), request.body.len().to_string());

    loader
        .load(&request, LayoutViewport::new(320, 200))
        .expect("redirected HTML document should load");

    let requests = requests.lock().expect("request log mutex poisoned");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, HttpMethod::Post);
    assert_eq!(requests[1].method, HttpMethod::Get);
    assert!(requests[1].body.is_empty());
    assert!(!requests[1].headers.contains_key("content-length"));
    assert!(!requests[1].headers.contains_key("content-type"));
    assert_eq!(requests[1].url.path(), "/after-submit");
}
