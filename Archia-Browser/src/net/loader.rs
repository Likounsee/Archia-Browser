use super::{pipeline::NetworkPipeline, Request, Response, Transport, TransportError};
use crate::{document::Page, layout::LayoutViewport};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentLoadError {
    Network(TransportError),
    UnsupportedContentType,
}

pub struct DocumentLoader<P, T> {
    pipeline: NetworkPipeline<P>,
    transport: T,
}

impl<P, T> DocumentLoader<P, T> {
    pub fn new(pipeline: NetworkPipeline<P>, transport: T) -> Self {
        Self {
            pipeline,
            transport,
        }
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
        let response = self
            .pipeline
            .execute(&self.transport, request)
            .map_err(DocumentLoadError::Network)?;

        if !is_html_response(&response) {
            return Err(DocumentLoadError::UnsupportedContentType);
        }

        let html = String::from_utf8_lossy(&response.body);
        Ok(Page::from_html(&html, "", viewport))
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
    fn rejects_non_html_content() {
        let response = Response::new(200).with_header("content-type", "image/png");
        let loader =
            DocumentLoader::new(NetworkPipeline::new(AllowAll), MockTransport { response });
        let request = Request::new(Url::parse("https://example.org/image.png").unwrap());

        assert_eq!(
            loader.load(&request, LayoutViewport::new(320, 200)),
            Err(DocumentLoadError::UnsupportedContentType)
        );
    }
}
