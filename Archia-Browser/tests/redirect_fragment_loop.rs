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
    let request = Request::new(Url::parse("https://example.org/start").unwrap())
        .with_method(HttpMethod::Get);

    let result = loader.load(&request, LayoutViewport::new(320, 200));

    assert!(matches!(result, Err(DocumentLoadError::RedirectLoopDetected)));
    let requests = requests.lock().expect("request log mutex poisoned");
    assert_eq!(requests.len(), 2, "loop must be stopped before a third request");
    assert_eq!(requests[1], "https://example.org/next#first");
}

#[test]
fn redirect_target_is_rechecked_by_policy_before_transport() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let transport = RedirectTransport {
        responses: Mutex::new(vec![
            Response::new(302).with_header("location", "https://blocked.example/private"),
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
    assert_eq!(
        requests,
        &["https://example.org/start"],
        "the denied redirect destination must never reach transport"
    );
}
