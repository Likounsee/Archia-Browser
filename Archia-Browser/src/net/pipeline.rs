use super::{Request, Response, Transport, TransportError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestPolicy {
    pub priority: RequestPriority,
    pub resource_kind: ResourceKind,
    pub referrer: Option<super::Url>,
    pub first_party: Option<super::Url>,
}

impl Default for RequestPolicy {
    fn default() -> Self {
        Self {
            priority: RequestPriority::Normal,
            resource_kind: ResourceKind::Other,
            referrer: None,
            first_party: None,
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
        if self.policy.decide(request) == PolicyDecision::Block {
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
