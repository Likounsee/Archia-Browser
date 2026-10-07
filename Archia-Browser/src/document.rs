use crate::css::StyleSheet;
use crate::html::{parse, HtmlTokenizer, Node};
use crate::layout::{LayoutEngine, LayoutNode, LayoutViewport};
use crate::render::{DisplayList, SoftwareRenderer};
use crate::style_tree::{StyleEngine, StyledNode};

#[derive(Debug, Clone)]
pub struct Page {
    pub document: Node,
    pub styled: StyledNode,
    pub layout: LayoutNode,
    pub display_list: DisplayList,
}

impl Page {
    pub fn from_html(html: &str, css: &str, viewport: LayoutViewport) -> Self {
        let tokens = HtmlTokenizer::tokenize(html);
        let document = parse(&tokens);
        let stylesheet = StyleSheet::parse(css);
        let styled = StyleEngine::style(&document, &stylesheet);
        let layout = LayoutEngine::layout_styled(&styled, viewport);
        let display_list = SoftwareRenderer::build_display_list_styled(&styled, &layout);

        Self {
            document,
            styled,
            layout,
            display_list,
        }
    }

    pub fn render_into(&self, surface: &mut crate::surface::SoftwareSurface) {
        surface.clear(crate::surface::Color::WHITE);
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
    fn render_into_produces_pixels() {
        let page = Page::from_html(
            r#"<body><div style="background-color: red">Hello</div></body>"#,
            "",
            LayoutViewport::new(32, 32),
        );
        let mut surface = crate::surface::SoftwareSurface::new(32, 32);
        page.render_into(&mut surface);
        assert!(surface
            .pixels()
            .chunks_exact(4)
            .any(|pixel| pixel != [255, 255, 255, 255]));
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
}
