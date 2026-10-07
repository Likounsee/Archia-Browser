use std::collections::VecDeque;

use super::{Request, Response, Transport, TransportError};

#[derive(Debug, Default)]
pub struct MockTransport {
    responses: VecDeque<Result<Response, TransportError>>,
    requests: std::sync::Mutex<Vec<Request>>,
}

impl MockTransport {
    pub fn push_response(&mut self, response: Result<Response, TransportError>) {
        self.responses.push_back(response);
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().expect("request log poisoned").clone()
    }
}

impl Transport for MockTransport {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        self.requests
            .lock()
            .expect("request log poisoned")
            .push(request.clone());
        Err(TransportError::ConnectionFailed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Block,
}

pub trait RequestPolicyEngine {
    fn decide(&self, request: &Request) -> PolicyDecision;
}

#[derive(Debug, Default)]
pub struct NetworkPipeline<P> {
    policy: P,
}

impl<P> NetworkPipeline<P>
where
    P: RequestPolicyEngine,
{
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
        filter::{FilterDecision, RequestFilter},
        Url,
    };

    struct FilterPolicy(RequestFilter);

    impl RequestPolicyEngine for FilterPolicy {
        fn decide(&self, request: &Request) -> PolicyDecision {
            match self.0.decide(&request.url.to_string(), Some(request.policy.resource_kind.into())) {
                FilterDecision::Allow => PolicyDecision::Allow,
                FilterDecision::Block => PolicyDecision::Block,
            }
        }
    }

    #[test]
    fn pipeline_blocks_before_transport() {
        let request = Request::new(Url::parse("https://ads.example/script.js").unwrap());
        let mut transport = MockTransport::default();
        transport.push_response(Ok(Response::new(200)));

        let pipeline = NetworkPipeline::new(FilterPolicy(RequestFilter::default()));
        let result = pipeline.execute(&transport, &request);

        assert!(matches!(result, Err(TransportError::ConnectionFailed)));
    }
}
