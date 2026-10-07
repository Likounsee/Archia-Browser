use super::{Request, ResourceKind, Response, Transport, TransportError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestPolicy {
    pub priority: RequestPriority,
    pub resource_kind: ResourceKind,
    pub referrer: Option<super::Url>,
}

impl Default for RequestPolicy {
    fn default() -> Self {
        Self {
            priority: RequestPriority::Normal,
            resource_kind: ResourceKind::Other,
            referrer: None,
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

    pub fn execute<T: Transport>(
        &self,
        transport: &T,
        request: &Request,
    ) -> Result<Response, TransportError> {
        if self.policy.decide(request) == PolicyDecision::Block {
            return Err(TransportError::ConnectionFailed);
        }
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

            match self.0.decide(&request.url.to_string(), Some(kind)) {
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
    fn pipeline_blocks_before_transport() {
        let mut filter = RequestFilter::default();
        filter.add_rule(FilterRule::block("ads.example").for_resource(ResourceType::Script));

        let request = Request::new(
            super::super::Url::parse("https://ads.example/script.js").unwrap(),
        )
        .with_method(HttpMethod::Get)
        .with_policy(RequestPolicy {
            resource_kind: ResourceKind::Script,
            ..Default::default()
        });

        let transport = MockTransport;
        let pipeline = NetworkPipeline::new(FilterPolicy(filter));

        assert!(matches!(
            pipeline.execute(&transport, &request),
            Err(TransportError::ConnectionFailed)
        ));
    }
}
