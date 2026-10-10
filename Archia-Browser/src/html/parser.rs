use super::{dom::Node, tokenizer::HtmlToken};

const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

const MAX_DOM_DEPTH: usize = 256;

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
    // Keep malformed hostile nesting from making open-element searches
    // quadratic or creating a tree too deep for recursive DOM consumers.
    // Once the limit is reached, discard that subtree until its matching
    // end tag; tokenization already bounds the total input and token count.
    let mut suppressed: Vec<String> = Vec::new();

    for token in tokens {
        if !suppressed.is_empty() {
            match token {
                HtmlToken::StartTag { name, self_closing: _, .. }
                    if !VOID_ELEMENTS.contains(&name.as_str()) =>
                {
                    suppressed.push(name.clone());
                }
                HtmlToken::EndTag(name) => {
                    if let Some(position) = suppressed.iter().rposition(|open| open == name) {
                        suppressed.truncate(position);
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
                if name == "p" && has_open(&stack, "p") {
                    close_element(&mut root, &mut stack, "p");
                } else if is_block_closing_p(name) && has_open(&stack, "p") {
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
                    suppressed.push(name.clone());
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

    let mut matching = completed.pop().expect("matching element exists");
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
    let mut html = root_children
        .next()
        .expect("the located html element exists");
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
        assert_eq!(root.children[0].children[1].children[0].tag_name(), Some("p"));
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
