use super::{dom::Node, tokenizer::HtmlToken};

const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
    "source", "track", "wbr",
];

const BLOCK_CLOSES_P: &[&str] = &[
    "address", "article", "aside", "blockquote", "div", "dl", "fieldset", "footer", "form",
    "h1", "h2", "h3", "h4", "h5", "h6", "header", "hr", "menu", "nav", "ol", "p", "pre",
    "section", "table", "ul",
];

pub fn parse(tokens: &[HtmlToken]) -> Node {
    let mut root = Node::document();
    let mut stack: Vec<Node> = Vec::new();

    for token in tokens {
        match token {
            HtmlToken::Doctype(_) => {}
            HtmlToken::Comment(comment) => append_node(&mut root, &mut stack, Node::comment(comment)),
            HtmlToken::Text(text) => {
                if text.is_empty() {
                    continue;
                }
                append_node(&mut root, &mut stack, Node::text(text));
            }
            HtmlToken::StartTag {
                name,
                attributes,
                self_closing,
            } => {
                if name == "p" && has_open(&stack, "p") {
                    close_element(&mut root, &mut stack, "p");
                } else if is_block_closing_p(name) && has_open(&stack, "p") {
                    close_element(&mut root, &mut stack, "p");
                }

                if name == "li" && has_open(&stack, "li") {
                    close_element(&mut root, &mut stack, "li");
                }

                let mut node = Node::element(name.clone());
                for (attribute, value) in attributes {
                    node.set_attribute(attribute, value);
                }

                if *self_closing || VOID_ELEMENTS.contains(&name.as_str()) {
                    append_node(&mut root, &mut stack, node);
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

fn is_block_closing_p(name: &str) -> bool {
    BLOCK_CLOSES_P.contains(&name)
}

fn close_element(root: &mut Node, stack: &mut Vec<Node>, name: &str) {
    let Some(position) = stack
        .iter()
        .rposition(|node| node.tag_name() == Some(name))
    else {
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
        return root;
    };

    if html_position != 0 {
        let html = root.children.remove(html_position);
        root.children.insert(0, html);
    }

    let html = &mut root.children[0];
    let Some(head_position) = html
        .children
        .iter()
        .position(|node| node.tag_name() == Some("head"))
    else {
        if let Some(body_position) = html
            .children
            .iter()
            .position(|node| node.tag_name() == Some("body"))
        {
            let body = html.children.remove(body_position);
            let mut head = Node::element("head");
            head.children = Vec::new();
            html.children.insert(0, head);
            html.children.push(body);
        }
        return root;
    };

    if head_position != 0 {
        let head = html.children.remove(head_position);
        html.children.insert(0, head);
    }

    if !html.children.iter().any(|node| node.tag_name() == Some("body")) {
        html.children.push(Node::element("body"));
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::tokenizer::HtmlTokenizer;

    #[test]
    fn builds_basic_tree() {
        let tokens = HtmlTokenizer::tokenize("<html><body>Hello</body></html>");
        let root = parse(&tokens);
        assert_eq!(root.children.len(), 1);
        assert_eq!(root.children[0].tag_name(), Some("html"));
        assert_eq!(root.children[0].children[0].tag_name(), Some("body"));
    }

    #[test]
    fn preserves_nested_children_when_closing_parent() {
        let tokens = HtmlTokenizer::tokenize("<div><span>hello</span><b>world</b></div>");
        let root = parse(&tokens);
        assert_eq!(root.children[0].children.len(), 2);
        assert_eq!(root.text_content(), "helloworld");
    }

    #[test]
    fn void_elements_do_not_capture_following_text() {
        let tokens = HtmlTokenizer::tokenize("<div><br>after</div>");
        let root = parse(&tokens);
        let div = &root.children[0];
        assert_eq!(div.children[0].tag_name(), Some("br"));
        assert_eq!(div.children[1].text_content(), "after");
    }

    #[test]
    fn p_auto_closes_before_block_content() {
        let tokens = HtmlTokenizer::tokenize("<p>one<div>two</div>");
        let root = parse(&tokens);
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[0].tag_name(), Some("p"));
        assert_eq!(root.children[1].tag_name(), Some("div"));
    }

    #[test]
    fn comments_are_kept_in_the_dom() {
        let tokens = HtmlTokenizer::tokenize("<div><!-- note --></div>");
        let root = parse(&tokens);
        assert!(matches!(
            root.children[0].children[0].kind,
            super::super::dom::NodeKind::Comment(_)
        ));
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
