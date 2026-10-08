use crate::css::StyleSheet;
use crate::html::{parse, HtmlTokenizer, Node};
use crate::layout::{LayoutEngine, LayoutNode, LayoutViewport};
use crate::net::Url;
use crate::render::{DisplayList, SoftwareRenderer};
use crate::style_tree::{StyleEngine, StyledNode};

#[derive(Debug, Clone)]
pub struct Page {
    url: Option<Url>,
    pub document: Node,
    pub styled: StyledNode,
    pub layout: LayoutNode,
    pub display_list: DisplayList,
}

impl Page {
    pub fn from_html(html: &str, css: &str, viewport: LayoutViewport) -> Self {
        Self::from_html_at(None, html, css, viewport)
    }

    pub fn from_html_at(url: Option<Url>, html: &str, css: &str, viewport: LayoutViewport) -> Self {
        let tokens = HtmlTokenizer::tokenize(html);
        let document = parse(&tokens);
        let stylesheet = StyleSheet::parse(css);
        let styled = StyleEngine::style(&document, &stylesheet);
        let layout = LayoutEngine::layout_styled(&styled, viewport);
        let display_list = SoftwareRenderer::build_display_list_styled(&styled, &layout);

        Self {
            url,
            document,
            styled,
            layout,
            display_list,
        }
    }

    pub fn url(&self) -> Option<&Url> {
        self.url.as_ref()
    }

    pub fn base_url(&self) -> Option<Url> {
        let document_url = self.url.clone()?;
        let href = self
            .document
            .find_first_element("base")?
            .attribute("href")?
            .trim();
        if href.is_empty() {
            return None;
        }

        document_url.resolve(href).ok()
    }

    pub fn resolve_reference(&self, reference: &str) -> Result<Url, crate::net::UrlError> {
        let base = self
            .base_url()
            .or_else(|| self.url.clone())
            .ok_or(crate::net::UrlError::MissingAuthority)?;
        base.resolve(reference)
    }

    pub fn title(&self) -> Option<String> {
        let title = self.document.find_first_element("title")?.text_content();
        let title = title.trim();
        (!title.is_empty()).then(|| title.to_owned())
    }

    pub fn render_into(&self, surface: &mut crate::surface::SoftwareSurface) {
        SoftwareRenderer::rasterize(&self.display_list, surface);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_static_html_css_pipeline() {
        let page = Page::from_html(
            r#"<body><div style="background-color: #102030">Hello</div></body>"#,
            "",
            LayoutViewport::new(320, 200),
        );

        assert_eq!(page.document.text_content(), "Hello");
        assert!(!page.display_list.commands().is_empty());
        assert_eq!(page.layout.rect.width, 320);
    }

    #[test]
    fn css_styles_reach_render_tree() {
        let page = Page::from_html(
            r#"<body><div class="hero">Hello</div></body>"#,
            ".hero { background-color: red; }",
            LayoutViewport::new(100, 100),
        );

        assert!(page.display_list.commands().iter().any(|command| {
            matches!(
                command,
                crate::render::PaintCommand::FillRect {
                    color: 0xff0000ff,
                    ..
                }
            )
        }));
    }

    #[test]
    #[test]
    fn resolves_links_against_document_base_element() {
        let url = Url::parse("https://example.org/docs/index.html").unwrap();
        let page = Page::from_html_at(
            Some(url),
            r#"<head><base href="/guide/"></head><body><a href="chapter.html">Next</a></body>"#,
            "",
            LayoutViewport::new(320, 200),
        );

        assert_eq!(
            page.base_url().map(|value| value.to_string()),
            Some("https://example.org/guide/".to_owned())
        );
        assert_eq!(
            page.resolve_reference("chapter.html").unwrap().to_string(),
            "https://example.org/guide/chapter.html"
        );
    }

    #[test]
    fn invalid_base_href_falls_back_to_document_url() {
        let page = Page::from_html_at(
            Some(Url::parse("https://example.org/docs/index.html").unwrap()),
            r#"<head><base href="javascript:bad"></head><body>Hello</body>"#,
            "",
            LayoutViewport::new(320, 200),
        );

        assert_eq!(
            page.resolve_reference("next.html").unwrap().to_string(),
            "https://example.org/docs/next.html"
        );
    }

    #[test]
    fn retains_document_url_when_loaded_at_a_navigation_target() {
        let url = Url::parse("https://example.org/docs/index.html").unwrap();
        let page = Page::from_html_at(
            Some(url.clone()),
            "<title>Example</title><body>Hello</body>",
            "",
            LayoutViewport::new(320, 200),
        );

        assert_eq!(page.url(), Some(&url));
    }

    fn extracts_trimmed_document_title() {
        let page = Page::from_html(
            "<html><head><title>  Archia Browser  </title></head><body>Hello</body></html>",
            "",
            LayoutViewport::new(320, 200),
        );

        assert_eq!(page.title(), Some("Archia Browser".to_owned()));
    }

    #[test]
    fn empty_or_missing_title_is_none() {
        let missing = Page::from_html("<body>Hello</body>", "", LayoutViewport::new(320, 200));
        let empty = Page::from_html(
            "<head><title>   </title></head><body>Hello</body>",
            "",
            LayoutViewport::new(320, 200),
        );

        assert_eq!(missing.title(), None);
        assert_eq!(empty.title(), None);
    }
}
