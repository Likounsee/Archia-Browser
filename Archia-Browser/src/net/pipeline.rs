use super::{Request, Response, Transport, TransportError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestPolicy {
    pub priority: RequestPriority,
    pub resource_kind: ResourceKind,
    pub referrer: Option<super::Url>,
    pub first_party: Option<super::Url>,
    /// True only for a browser-initiated top-level document navigation.
    pub top_level_navigation: bool,
}

impl Default for RequestPolicy {
    fn default() -> Self {
        Self {
            priority: RequestPriority::Normal,
            resource_kind: ResourceKind::Other,
            referrer: None,
            first_party: None,
            top_level_navigation: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestPriority {
    High,
    Normal,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Document,
    Stylesheet,
    Script,
    Image,
    Font,
    Media,
    Fetch,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Block,
}

pub trait RequestPolicyEngine {
    fn decide(&self, request: &Request) -> PolicyDecision;
}

#[derive(Debug)]
pub struct NetworkPipeline<P> {
    policy: P,
}

impl<P: RequestPolicyEngine> NetworkPipeline<P> {
    pub fn new(policy: P) -> Self {
        Self { policy }
    }

    /// Enforce the current request policy independently of network I/O.
    ///
    /// Callers that can serve a response from a cache must still check policy
    /// first; a cache hit must never bypass filtering or resource restrictions.
    pub fn check_policy(&self, request: &Request) -> Result<(), TransportError> {
        // A secure document must not load cleartext HTTP subresources. Keep
        // this invariant in the shared pipeline so future resource loaders
        // cannot accidentally bypass a loader-specific mixed-content check.
        // Top-level document navigation is intentionally exempt: following an
        // HTTP link is navigation, not mixed content within the current page.
        let is_mixed_content = !(request.policy.resource_kind == ResourceKind::Document
            && request.policy.top_level_navigation)
            && request
                .policy
                .first_party
                .as_ref()
                .is_some_and(|first_party| first_party.is_secure())
            && request.url.scheme() == "http";
        if is_mixed_content || self.policy.decide(request) == PolicyDecision::Block {
            return Err(TransportError::BlockedByPolicy);
        }
        Ok(())
    }

    pub fn execute<T: Transport>(
        &self,
        transport: &T,
        request: &Request,
    ) -> Result<Response, TransportError> {
        self.check_policy(request)?;
        transport.send(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{
        filter::{FilterDecision, FilterRule, RequestFilter, ResourceType},
        HttpMethod,
    };

    struct FilterPolicy(RequestFilter);

    impl RequestPolicyEngine for FilterPolicy {
        fn decide(&self, request: &Request) -> PolicyDecision {
            let kind = match request.policy.resource_kind {
                ResourceKind::Document => ResourceType::Document,
                ResourceKind::Stylesheet => ResourceType::Style,
                ResourceKind::Script => ResourceType::Script,
                ResourceKind::Image => ResourceType::Image,
                ResourceKind::Font => ResourceType::Font,
                ResourceKind::Media => ResourceType::Media,
                ResourceKind::Fetch => ResourceType::Xhr,
                ResourceKind::Other => ResourceType::Other,
            };

            match self.0.decide_with_party(
                &request.url.to_string(),
                Some(kind),
                request.policy.first_party.as_ref(),
            ) {
                FilterDecision::Allow => PolicyDecision::Allow,
                FilterDecision::Block => PolicyDecision::Block,
            }
        }
    }

    #[derive(Debug, Default)]
    struct MockTransport;

    impl Transport for MockTransport {
        fn send(&self, _: &Request) -> Result<Response, TransportError> {
            Ok(Response::new(200))
        }
    }

    #[test]
    fn policy_blocks_every_resource_kind_before_transport() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        struct CountingTransport(Arc<AtomicUsize>);

        impl Transport for CountingTransport {
            fn send(&self, _: &Request) -> Result<Response, TransportError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(Response::new(200))
            }
        }

        let mut filter = RequestFilter::default();
        filter.add_rule(FilterRule::block("blocked.example"));
        let pipeline = NetworkPipeline::new(FilterPolicy(filter));
        let sends = Arc::new(AtomicUsize::new(0));
        let transport = CountingTransport(Arc::clone(&sends));
        let resource_kinds = [
            ResourceKind::Document,
            ResourceKind::Stylesheet,
            ResourceKind::Script,
            ResourceKind::Image,
            ResourceKind::Font,
            ResourceKind::Media,
            ResourceKind::Fetch,
            ResourceKind::Other,
        ];

        for resource_kind in resource_kinds {
            let request =
                Request::new(super::super::Url::parse("https://blocked.example/resource").unwrap())
                    .with_method(HttpMethod::Get)
                    .with_policy(RequestPolicy {
                        resource_kind,
                        ..Default::default()
                    });

            assert!(
                matches!(
                    pipeline.execute(&transport, &request),
                    Err(TransportError::BlockedByPolicy)
                ),
                "policy should block resource kind {resource_kind:?}"
            );
        }

        assert_eq!(
            sends.load(Ordering::SeqCst),
            0,
            "a blocked request must never reach the transport"
        );
    }

    #[test]
    fn secure_documents_cannot_load_http_subresources_through_shared_pipeline() {
        #[derive(Debug, Default)]
        struct AllowAll;

        impl RequestPolicyEngine for AllowAll {
            fn decide(&self, _: &Request) -> PolicyDecision {
                PolicyDecision::Allow
            }
        }

        let pipeline = NetworkPipeline::new(AllowAll);
        let secure_page = super::super::Url::parse("https://example.org/page").unwrap();
        for resource_kind in [
            ResourceKind::Stylesheet,
            ResourceKind::Script,
            ResourceKind::Image,
            ResourceKind::Font,
            ResourceKind::Media,
            ResourceKind::Fetch,
            ResourceKind::Other,
        ] {
            let mut request =
                Request::new(super::super::Url::parse("http://cdn.example/resource").unwrap());
            request.policy = RequestPolicy {
                resource_kind,
                first_party: Some(secure_page.clone()),
                ..RequestPolicy::default()
            };
            assert_eq!(
                pipeline.check_policy(&request),
                Err(TransportError::BlockedByPolicy),
                "cleartext subresource should be blocked: {resource_kind:?}"
            );
        }
    }

    #[test]
    fn mixed_content_guard_does_not_block_top_level_http_navigation() {
        #[derive(Debug, Default)]
        struct AllowAll;

        impl RequestPolicyEngine for AllowAll {
            fn decide(&self, _: &Request) -> PolicyDecision {
                PolicyDecision::Allow
            }
        }

        let pipeline = NetworkPipeline::new(AllowAll);
        let mut request =
            Request::new(super::super::Url::parse("http://example.org/next").unwrap());
        request.policy = RequestPolicy {
            resource_kind: ResourceKind::Document,
            first_party: Some(super::super::Url::parse("https://example.org/page").unwrap()),
            top_level_navigation: true,
            ..RequestPolicy::default()
        };

        assert_eq!(pipeline.check_policy(&request), Ok(()));
    }

    #[test]
    fn mixed_content_guard_blocks_http_embedded_documents() {
        #[derive(Debug, Default)]
        struct AllowAll;

        impl RequestPolicyEngine for AllowAll {
            fn decide(&self, _: &Request) -> PolicyDecision {
                PolicyDecision::Allow
            }
        }

        let pipeline = NetworkPipeline::new(AllowAll);
        let mut request = Request::new(super::super::Url::parse("http://frame.example/").unwrap());
        request.policy = RequestPolicy {
            resource_kind: ResourceKind::Document,
            first_party: Some(super::super::Url::parse("https://example.org/page").unwrap()),
            top_level_navigation: false,
            ..RequestPolicy::default()
        };

        assert_eq!(
            pipeline.check_policy(&request),
            Err(TransportError::BlockedByPolicy)
        );
    }

    #[test]
    fn canonicalized_host_is_used_by_request_policy_before_transport() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        struct CountingTransport(Arc<AtomicUsize>);

        impl Transport for CountingTransport {
            fn send(&self, _: &Request) -> Result<Response, TransportError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(Response::new(200))
            }
        }

        let mut filter = RequestFilter::default();
        filter.add_rule(FilterRule::block("example.org"));
        let pipeline = NetworkPipeline::new(FilterPolicy(filter));
        let sends = Arc::new(AtomicUsize::new(0));
        let transport = CountingTransport(Arc::clone(&sends));
        let request =
            Request::new(super::super::Url::parse("https://EXAMPLE.ORG/resource").unwrap())
                .with_method(HttpMethod::Get);

        assert_eq!(
            request.url.authority(),
            "example.org",
            "URL parsing must canonicalize the host before policy evaluation"
        );
        assert_eq!(
            pipeline.execute(&transport, &request),
            Err(TransportError::BlockedByPolicy)
        );
        assert_eq!(
            sends.load(Ordering::SeqCst),
            0,
            "a policy match on a canonicalized host must not reach transport"
        );
    }

    #[test]
    fn pipeline_blocks_before_transport() {
        let mut filter = RequestFilter::default();
        filter.add_rule(FilterRule::block("ads.example").for_resource(ResourceType::Script));

        let request =
            Request::new(super::super::Url::parse("https://ads.example/script.js").unwrap())
                .with_method(HttpMethod::Get)
                .with_policy(RequestPolicy {
                    resource_kind: ResourceKind::Script,
                    ..Default::default()
                });

        let transport = MockTransport;
        let pipeline = NetworkPipeline::new(FilterPolicy(filter));

        assert!(matches!(
            pipeline.execute(&transport, &request),
            Err(TransportError::BlockedByPolicy)
        ));
    }
}
