use archia_browser::{
    css::{parse_declarations, CssTokenizer, StyleSheet},
    html::{parse, HtmlTokenizer, NodeKind},
    net::{
        filter::{FilterDecision, FilterRule, RequestFilter, ResourceType},
        Url,
    },
};

#[test]
fn browser_stack_foundations_work_together() {
    let url = Url::parse("https://example.org/index.html").unwrap();
    assert_eq!(url.scheme(), "https");

    let html = HtmlTokenizer::tokenize("<html><body>Hello</body></html>");
    let document = parse(&html);
    assert_eq!(document.children.len(), 1);
    assert_eq!(document.children[0].tag_name(), Some("html"));
    assert_eq!(document.children[0].children[0].tag_name(), Some("head"));
    assert_eq!(document.children[0].children[1].tag_name(), Some("body"));

    let html = HtmlTokenizer::tokenize(r#"<div id="app" class="card">Hello</div>"#);
    let document = parse(&html);
    let body = &document.children[0].children[1];
    let NodeKind::Element { attributes, .. } = &body.children[0].kind else {
        panic!("expected element");
    };
    assert_eq!(attributes.get("id"), Some(&"app".to_string()));

    let sheet = StyleSheet::parse("#app { color: red; } .card { padding: 8px; }");
    let styled = sheet.compute_style(&body.children[0]);
    assert_eq!(styled.get("color"), Some("red"));
    assert_eq!(styled.get("padding"), Some("8px"));

    let css = CssTokenizer::tokenize("color: red;");
    let declarations = parse_declarations(&css);
    assert_eq!(declarations[0].name, "color");

    let mut filter = RequestFilter::default();
    filter.add_rule(FilterRule::block("ads.example").for_resource(ResourceType::Script));
    assert_eq!(
        filter.decide("https://ads.example/ad.js", Some(ResourceType::Script)),
        FilterDecision::Block
    );
}

#[test]
fn visibility_is_inherited_but_visible_descendants_can_paint() {
    let mut root = archia_browser::html::Node::element("div");
    root.set_attribute("style", "visibility: hidden; background-color: red;");
    let mut hidden = archia_browser::html::Node::element("span");
    hidden.set_attribute("style", "display: block; background-color: blue;");
    hidden.append(archia_browser::html::Node::text("hidden"));
    let mut visible = archia_browser::html::Node::element("span");
    visible.set_attribute(
        "style",
        "display: block; visibility: visible; background-color: green;",
    );
    visible.append(archia_browser::html::Node::text("visible"));
    root.append(hidden);
    root.append(visible);

    let styled = archia_browser::style_tree::StyleEngine::style(&root, &StyleSheet::default());
    let layout = archia_browser::layout::LayoutEngine::layout_styled(
        &styled,
        archia_browser::layout::LayoutViewport::new(200, 100),
    );
    assert_eq!(layout.children.len(), 2);
    assert_eq!(layout.children[0].rect.y, 0);
    assert_eq!(layout.children[1].rect.y, 16);

    let list =
        archia_browser::render::SoftwareRenderer::build_display_list_styled(&styled, &layout);
    assert!(list.commands().iter().any(|command| matches!(
        command,
        archia_browser::render::PaintCommand::FillRect {
            color: 0x008000ff,
            ..
        }
    )));
    assert!(!list.commands().iter().any(|command| matches!(
        command,
        archia_browser::render::PaintCommand::FillRect {
            color: 0xff0000ff,
            ..
        }
    )));
}

#[test]
fn visibility_collapse_stays_in_layout_but_suppresses_paint() {
    let mut root = archia_browser::html::Node::element("div");
    root.set_attribute("style", "visibility: collapse;");
    root.append(archia_browser::html::Node::text("collapsed"));
    let styled = archia_browser::style_tree::StyleEngine::style(&root, &StyleSheet::default());
    let layout = archia_browser::layout::LayoutEngine::layout_styled(
        &styled,
        archia_browser::layout::LayoutViewport::new(200, 100),
    );
    assert!(layout.rect.height > 0);
    let list =
        archia_browser::render::SoftwareRenderer::build_display_list_styled(&styled, &layout);
    assert!(list.commands().is_empty());
}

#[test]
fn whitespace_pre_line_collapses_spaces_but_preserves_newlines() {
    let mut root = archia_browser::html::Node::element("div");
    root.set_attribute("style", "white-space: pre-line;");
    root.append(archia_browser::html::Node::text("  one\n  two  "));
    let styled = archia_browser::style_tree::StyleEngine::style(&root, &StyleSheet::default());
    let layout = archia_browser::layout::LayoutEngine::layout_styled(
        &styled,
        archia_browser::layout::LayoutViewport::new(200, 100),
    );
    let list =
        archia_browser::render::SoftwareRenderer::build_display_list_styled(&styled, &layout);
    assert!(list.commands().iter().any(|command| matches!(
        command,
        archia_browser::render::PaintCommand::DrawText { text, .. } if text == "one\ntwo"
    )));
}
