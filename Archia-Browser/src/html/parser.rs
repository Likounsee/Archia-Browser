use std::collections::HashMap;

use super::{dom::Node, tokenizer::HtmlToken};

const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

const MAX_DOM_DEPTH: usize = 256;
const MAX_DOM_TOKENS: usize = 100_000;

const BLOCK_CLOSES_P: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "details",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "pre",
    "search",
    "section",
    "table",
    "ul",
];

pub fn parse(tokens: &[HtmlToken]) -> Node {
    let mut root = Node::document();
    let mut stack: Vec<Node> = Vec::new();
    // Keep malformed hostile nesting from creating a tree too deep for
    // recursive DOM consumers. Once the limit is reached, discard that subtree;
    // an indexed tag stack lets matching end tags recover without repeated
    // linear scans through attacker-controlled nesting.
    let mut suppressed_stack: Vec<String> = Vec::new();
    // Index open suppressed tags by name so matching close tags do not scan the
    // entire attacker-controlled nesting stack on every token.
    let mut suppressed_positions: HashMap<String, Vec<usize>> = HashMap::new();

    for token in tokens.iter().take(MAX_DOM_TOKENS) {
        if !suppressed_stack.is_empty() {
            match token {
                HtmlToken::StartTag { name, .. } if !VOID_ELEMENTS.contains(&name.as_str()) => {
                    let position = suppressed_stack.len();
                    suppressed_stack.push(name.clone());
                    suppressed_positions
                        .entry(name.clone())
                        .or_default()
                        .push(position);
                }
                HtmlToken::EndTag(name) if !VOID_ELEMENTS.contains(&name.as_str()) => {
                    // Mirror the parser's matching-end-tag behavior inside the
                    // discarded subtree. Unmatched/void end tags must not reduce
                    // the suppression depth and expose hostile nested markup.
                    if let Some(position) = suppressed_positions
                        .get(name)
                        .and_then(|positions| positions.last())
                        .copied()
                    {
                        while suppressed_stack.len() > position {
                            if let Some(open) = suppressed_stack.pop() {
                                let remove_entry =
                                    if let Some(positions) = suppressed_positions.get_mut(&open) {
                                        positions.pop();
                                        positions.is_empty()
                                    } else {
                                        false
                                    };
                                if remove_entry {
                                    suppressed_positions.remove(&open);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            continue;
        }

        match token {
            HtmlToken::Doctype(_) => {}
            HtmlToken::Comment(comment) => {
                append_node(&mut root, &mut stack, Node::comment(comment));
            }
            HtmlToken::Text(text) => {
                if text.is_empty() {
                    continue;
                }
                append_node(&mut root, &mut stack, Node::text(text));
            }
            HtmlToken::StartTag {
                name,
                attributes,
                self_closing: _,
            } => {
                // Apply the most common HTML implied-end-tag rules before
                // inserting a new element. Without these, malformed-but-common
                // markup such as adjacent anchors or options produces a DOM
                // tree different from browser expectations.
                for implied_end in implied_end_tags_for_start(name) {
                    if has_open(&stack, implied_end) {
                        close_element(&mut root, &mut stack, implied_end);
                    }
                }

                if (name == "p" || is_block_closing_p(name)) && has_open(&stack, "p") {
                    close_element(&mut root, &mut stack, "p");
                }

                if name == "li" && has_open_list_item_in_current_list(&stack) {
                    close_element(&mut root, &mut stack, "li");
                }

                let mut node = Node::element(name.clone());
                for (attribute, value) in attributes {
                    node.set_attribute(attribute, value);
                }

                // In HTML syntax, the self-closing flag is ignored for
                // ordinary HTML elements. Only void elements close without
                // an end tag; treating <div/> as empty changes the DOM tree.
                if VOID_ELEMENTS.contains(&name.as_str()) {
                    append_node(&mut root, &mut stack, node);
                } else if stack.len() >= MAX_DOM_DEPTH {
                    suppressed_stack.push(name.clone());
                    suppressed_positions
                        .entry(name.clone())
                        .or_default()
                        .push(0);
                } else {
                    stack.push(node);
                }
            }
            HtmlToken::EndTag(name) => close_element(&mut root, &mut stack, name),
        }
    }

    while let Some(node) = stack.pop() {
        append_node(&mut root, &mut stack, node);
    }

    normalize_document_structure(root)
}

fn append_node(root: &mut Node, stack: &mut [Node], node: Node) {
    if let Some(parent) = stack.last_mut() {
        parent.append(node);
    } else {
        root.append(node);
    }
}

fn has_open(stack: &[Node], name: &str) -> bool {
    stack.iter().any(|node| node.tag_name() == Some(name))
}

fn has_open_list_item_in_current_list(stack: &[Node]) -> bool {
    for node in stack.iter().rev() {
        match node.tag_name() {
            Some("li") => return true,
            Some("ol" | "ul") => return false,
            _ => {}
        }
    }
    false
}

fn implied_end_tags_for_start(name: &str) -> &'static [&'static str] {
    match name {
        "a" => &["a"],
        "button" => &["button"],
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => &["h1", "h2", "h3", "h4", "h5", "h6"],
        "dt" | "dd" => &["dt", "dd"],
        "option" => &["option"],
        "optgroup" => &["option", "optgroup"],
        _ => &[],
    }
}

fn is_block_closing_p(name: &str) -> bool {
    BLOCK_CLOSES_P.contains(&name)
}

fn close_element(root: &mut Node, stack: &mut Vec<Node>, name: &str) {
    let Some(position) = stack.iter().rposition(|node| node.tag_name() == Some(name)) else {
        return;
    };

    let mut completed = Vec::new();
    while stack.len() > position {
        if let Some(node) = stack.pop() {
            completed.push(node);
        }
    }

    let Some(mut matching) = completed.pop() else {
        // A matching stack position should always yield a node, but malformed
        // input must never turn an internal invariant violation into a panic.
        return;
    };
    while let Some(node) = completed.pop() {
        matching.append(node);
    }
    append_node(root, stack, matching);
}

fn normalize_document_structure(mut root: Node) -> Node {
    let Some(html_position) = root
        .children
        .iter()
        .position(|node| node.tag_name() == Some("html"))
    else {
        let mut html = Node::element("html");
        let mut head = Node::element("head");
        let mut body = Node::element("body");

        let children = std::mem::take(&mut root.children);
        for child in children {
            if child.tag_name() == Some("head") {
                head.append(child);
            } else {
                body.append(child);
            }
        }

        html.append(head);
        html.append(body);
        root.append(html);
        return root;
    };

    // Content outside an explicit <html> element is still part of the
    // document. Preserve it by placing preceding siblings at the start of
    // <body> and following siblings at the end, rather than leaving nodes
    // as invalid document-level siblings.
    let mut root_children = std::mem::take(&mut root.children).into_iter();
    let mut preceding = root_children
        .by_ref()
        .take(html_position)
        .collect::<Vec<_>>();
    let Some(mut html) = root_children.next() else {
        // The position was found before taking the children; keep parsing
        // resilient if that invariant is ever changed by future refactors.
        return root;
    };
    let following = root_children.collect::<Vec<_>>();

    let mut head = None;
    let mut body = None;
    let mut body_extras = Vec::new();
    let children = std::mem::take(&mut html.children);

    for child in children {
        match child.tag_name() {
            Some("head") if head.is_none() => head = Some(child),
            Some("body") if body.is_none() => body = Some(child),
            _ => body_extras.push(child),
        }
    }

    let mut body = body.unwrap_or_else(|| Node::element("body"));
    for child in body_extras {
        body.append(child);
    }

    preceding.append(&mut body.children);
    body.children = preceding;
    body.children.extend(following);

    html.append(head.unwrap_or_else(|| Node::element("head")));
    html.append(body);
    root.append(html);
    root
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::tokenizer::HtmlTokenizer;

    #[test]
    fn excessive_nesting_is_discarded_without_corrupting_following_siblings() {
        let mut html = "<div>".repeat(MAX_DOM_DEPTH + 32);
        html.push_str("discarded");
        html.push_str(&"</div>".repeat(MAX_DOM_DEPTH + 32));
        html.push_str("<p>visible</p>");

        let root = parse(&HtmlTokenizer::tokenize(&html));

        assert_eq!(root.text_content(), "visible");
        assert!(root.find_first_element("p").is_some());
    }

    #[test]
    fn unmatched_and_void_end_tags_cannot_escape_suppressed_subtree() {
        let mut html = "<div>".repeat(MAX_DOM_DEPTH + 1);
        html.push_str("hidden</br></unknown></img>");
        html.push_str(&"<span>".repeat(8));
        html.push_str("still hidden</div>");
        html.push_str(&"</span>".repeat(8));
        html.push_str(&"</div>".repeat(MAX_DOM_DEPTH));
        html.push_str("<p>visible</p>");

        let root = parse(&HtmlTokenizer::tokenize(&html));

        assert_eq!(root.text_content(), "visible");
        assert!(root.find_first_element("p").is_some());
        assert!(root.find_first_element("span").is_none());
    }

    #[test]
    fn matching_ancestor_end_tag_recovers_after_suppressed_subtree() {
        let mut html = "<div>".repeat(MAX_DOM_DEPTH + 1);
        html.push_str("hidden</div>");
        html.push_str(&"</div>".repeat(MAX_DOM_DEPTH));
        html.push_str("<p>visible</p>");

        let root = parse(&HtmlTokenizer::tokenize(&html));

        assert_eq!(root.text_content(), "visible");
        assert!(root.find_first_element("p").is_some());
    }

    #[test]
    fn parser_caps_direct_token_input_even_without_the_tokenizer() {
        let mut tokens = vec![HtmlToken::Doctype(String::new()); MAX_DOM_TOKENS];
        tokens.push(HtmlToken::Text("beyond-limit".to_owned()));

        let root = parse(&tokens);

        assert!(root.text_content().is_empty());
    }

    #[test]
    fn builds_basic_tree() {
        let tokens = HtmlTokenizer::tokenize("<html><body>Hello</body></html>");
        let root = parse(&tokens);
        assert_eq!(root.children.len(), 1);
        assert_eq!(root.children[0].tag_name(), Some("html"));
        assert_eq!(root.children[0].children[0].tag_name(), Some("head"));
        assert_eq!(root.children[0].children[1].tag_name(), Some("body"));
    }

    #[test]
    fn preserves_nested_children_when_closing_parent() {
        let tokens = HtmlTokenizer::tokenize("<div><span>hello</span><b>world</b></div>");
        let root = parse(&tokens);
        let body = &root.children[0].children[1];
        let div = &body.children[0];
        assert_eq!(div.children.len(), 2);
        assert_eq!(root.text_content(), "helloworld");
    }

    #[test]
    fn self_closing_syntax_does_not_close_non_void_html_elements() {
        let tokens = HtmlTokenizer::tokenize("<div/>inside</div><p/>text</p>");
        let root = parse(&tokens);
        let body = &root.children[0].children[1];

        assert_eq!(body.children[0].tag_name(), Some("div"));
        assert_eq!(body.children[0].text_content(), "inside");
        assert_eq!(body.children[1].tag_name(), Some("p"));
        assert_eq!(body.children[1].text_content(), "text");
    }

    #[test]
    fn void_elements_do_not_capture_following_text() {
        let tokens = HtmlTokenizer::tokenize("<div><br>after</div>");
        let root = parse(&tokens);
        let body = &root.children[0].children[1];
        let div = &body.children[0];
        assert_eq!(div.children[0].tag_name(), Some("br"));
        assert_eq!(div.children[1].text_content(), "after");
    }

    #[test]
    fn p_auto_closes_before_additional_html_block_elements() {
        for tag in [
            "details",
            "figcaption",
            "figure",
            "hgroup",
            "main",
            "search",
        ] {
            let markup = format!("<p>before<{tag}>inside</{tag}>");
            let tokens = HtmlTokenizer::tokenize(&markup);
            let root = parse(&tokens);
            let body = &root.children[0].children[1];

            assert_eq!(body.children[0].tag_name(), Some("p"), "tag: {tag}");
            assert_eq!(body.children[0].text_content(), "before", "tag: {tag}");
            assert_eq!(body.children[1].tag_name(), Some(tag), "tag: {tag}");
        }
    }

    #[test]
    fn p_auto_closes_before_block_content() {
        let tokens = HtmlTokenizer::tokenize("<p>one<div>two</div>");
        let root = parse(&tokens);
        let body = &root.children[0].children[1];
        assert_eq!(body.children.len(), 2);
        assert_eq!(body.children[0].tag_name(), Some("p"));
        assert_eq!(body.children[1].tag_name(), Some("div"));
    }

    #[test]
    fn nested_list_items_do_not_close_the_outer_list_item() {
        let tokens = HtmlTokenizer::tokenize(
            "<ul><li>outer<ul><li>inner</li></ul></li><li>second</li></ul>",
        );
        let root = parse(&tokens);
        let body = &root.children[0].children[1];
        let outer_list = &body.children[0];
        let outer_item = &outer_list.children[0];
        let nested_list = outer_item.find_first_element("ul").unwrap();

        assert_eq!(outer_item.text_content(), "outerinner");
        assert_eq!(nested_list.children.len(), 1);
        assert_eq!(nested_list.children[0].text_content(), "inner");
        assert_eq!(outer_list.children[1].text_content(), "second");
    }

    #[test]
    fn adjacent_anchors_and_buttons_do_not_nest() {
        let root = parse(&HtmlTokenizer::tokenize(
            "<a href='/first'>one<a href='/second'>two</a><button>first<button>second</button>",
        ));
        let body = &root.children[0].children[1];

        assert_eq!(body.children[0].tag_name(), Some("a"));
        assert_eq!(body.children[0].text_content(), "one");
        assert_eq!(body.children[1].tag_name(), Some("a"));
        assert_eq!(body.children[1].text_content(), "two");
        assert_eq!(body.children[2].tag_name(), Some("button"));
        assert_eq!(body.children[2].text_content(), "first");
        assert_eq!(body.children[3].tag_name(), Some("button"));
        assert_eq!(body.children[3].text_content(), "second");
    }

    #[test]
    fn headings_and_definition_terms_close_previous_peers() {
        let root = parse(&HtmlTokenizer::tokenize(
            "<h1>one<h2>two</h2><dl><dt>term<dd>definition</dd></dl>",
        ));
        let body = &root.children[0].children[1];

        assert_eq!(body.children[0].tag_name(), Some("h1"));
        assert_eq!(body.children[0].text_content(), "one");
        assert_eq!(body.children[1].tag_name(), Some("h2"));
        let list = &body.children[2];
        assert_eq!(list.children[0].tag_name(), Some("dt"));
        assert_eq!(list.children[0].text_content(), "term");
        assert_eq!(list.children[1].tag_name(), Some("dd"));
        assert_eq!(list.children[1].text_content(), "definition");
    }

    #[test]
    fn options_close_before_the_next_option_or_group() {
        let root = parse(&HtmlTokenizer::tokenize(
            "<select><option>one<option>two<optgroup><option>three</optgroup></select>",
        ));
        let select = root.find_first_element("select").unwrap();

        assert_eq!(select.children[0].tag_name(), Some("option"));
        assert_eq!(select.children[0].text_content(), "one");
        assert_eq!(select.children[1].tag_name(), Some("option"));
        assert_eq!(select.children[1].text_content(), "two");
        assert_eq!(select.children[2].tag_name(), Some("optgroup"));
        assert_eq!(select.children[2].children[0].tag_name(), Some("option"));
        assert_eq!(select.children[2].children[0].text_content(), "three");
    }

    #[test]
    fn comments_are_kept_in_the_dom() {
        let tokens = HtmlTokenizer::tokenize("<div><!-- note --></div>");
        let root = parse(&tokens);
        let body = &root.children[0].children[1];
        let div = &body.children[0];
        assert!(matches!(
            div.children[0].kind,
            super::super::dom::NodeKind::Comment(_)
        ));
    }

    #[test]
    fn plain_content_gets_implicit_html_head_and_body() {
        let tokens = HtmlTokenizer::tokenize("Hello <p>world</p>");
        let root = parse(&tokens);
        let html = &root.children[0];
        assert_eq!(html.tag_name(), Some("html"));
        assert_eq!(html.children[0].tag_name(), Some("head"));
        assert_eq!(html.children[1].tag_name(), Some("body"));
        assert_eq!(html.children[1].text_content(), "Hello world");
    }

    #[test]
    fn content_outside_explicit_html_is_moved_into_body_in_order() {
        let tokens = HtmlTokenizer::tokenize(
            "before<html><head><title>title</title></head><body>inside</body></html>after",
        );
        let root = parse(&tokens);

        assert_eq!(root.children.len(), 1);
        let html = &root.children[0];
        let body = &html.children[1];
        assert_eq!(body.text_content(), "beforeinsideafter");
    }

    #[test]
    fn explicit_html_gets_an_implicit_head() {
        let tokens = HtmlTokenizer::tokenize("<html><body>Hello</body></html>");
        let root = parse(&tokens);
        let html = &root.children[0];
        assert_eq!(html.children[0].tag_name(), Some("head"));
        assert_eq!(html.children[1].tag_name(), Some("body"));
    }
}
