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

    let html = HtmlTokenizer::tokenize(r#"<div id="app" class="card">Hello</div>"#);
    let document = parse(&html);
    let NodeKind::Element { attributes, .. } = &document.children[0].kind else {
        panic!("expected element");
    };
    assert_eq!(attributes.get("id"), Some(&"app".to_string()));

    let sheet = StyleSheet::parse("#app { color: red; } .card { padding: 8px; }");
    let styled = sheet.compute_style(&document.children[0]);
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
