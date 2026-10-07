use archia_browser::{css::{parse_declarations, CssTokenizer}, html::{parse, HtmlTokenizer}, net::{filter::{FilterDecision, FilterRule, RequestFilter, ResourceType}, Url}};

#[test]
fn browser_stack_foundations_work_together() {
    let url = Url::parse("https://example.org/index.html").unwrap();
    assert_eq!(url.scheme(), "https");

    let html = HtmlTokenizer::tokenize("<html><body>Hello</body></html>");
    let document = parse(&html);
    assert_eq!(document.children.len(), 1);

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
