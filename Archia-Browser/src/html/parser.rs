use super::{dom::Node, tokenizer::HtmlToken};

pub fn parse(tokens: &[HtmlToken]) -> Node {
    let mut root = Node::document();
    let mut stack: Vec<Node> = Vec::new();

    for token in tokens {
        match token {
            HtmlToken::Doctype(_) => {}
            HtmlToken::Comment(comment) => {
                append_node(&mut root, &mut stack, Node::comment(comment.clone()));
            }
            HtmlToken::Text(text) => {
                append_node(&mut root, &mut stack, Node::text(text));
            }
            HtmlToken::StartTag {
                name,
                attributes,
                self_closing,
            } => {
                close_optional_elements(&mut root, &mut stack, name);

                let mut node = Node::element(name.clone());
                if let super::dom::NodeKind::Element {
                    attributes: target, ..
                } = &mut node.kind
                {
                    *target = attributes.clone();
                }

                if *self_closing || is_void_element(name) {
                    append_node(&mut root, &mut stack, node);
                } else {
                    stack.push(node);
                }
            }
            HtmlToken::EndTag(name) => {
                if let Some(position) = stack.iter().rposition(|node| {
                    matches!(
                        &node.kind,
                        super::dom::NodeKind::Element {
                            name: node_name, ..
                        } if node_name == name
                    )
                }) {
                    let mut completed = stack.split_off(position);
                    if let Some(node) = completed.pop() {
                        append_node(&mut root, &mut stack, node);
                    }
                }
            }
        }
    }

    while let Some(node) = stack.pop() {
        append_node(&mut root, &mut stack, node);
    }

    root
}

fn append_node(root: &mut Node, stack: &mut Vec<Node>, node: Node) {
    if let Some(parent) = stack.last_mut() {
        parent.append(node);
    } else {
        root.append(node);
    }
}

fn close_optional_elements(root: &mut Node, stack: &mut Vec<Node>, incoming: &str) {
    let should_close = stack.last().and_then(|node| node.tag_name());
    if should_close.is_some_and(|name| optional_end_tag_conflicts(name, incoming)) {
        if let Some(node) = stack.pop() {
            append_node(root, stack, node);
        }
    }
}

fn optional_end_tag_conflicts(open: &str, incoming: &str) -> bool {
    match incoming {
        "p" => open == "p",

        "li" => open == "li",
        "dt" | "dd" => matches!(open, "dt" | "dd"),
        "tr" => open == "tr",
        "th" | "td" => matches!(open, "th" | "td"),
        "option" => open == "option",
        "thead" | "tbody" | "tfoot" => matches!(open, "thead" | "tbody" | "tfoot"),
        _ if is_p_closing_block(incoming) => open == "p",
        _ => false,
    }
}

fn is_p_closing_block(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "div"
            | "dl"
            | "fieldset"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hgroup"
            | "hr"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "ul"
    )
}

fn is_optional_end_tag(name: &str) -> bool {
    matches!(
        name,
        "p" | "li" | "dt" | "dd" | "tr" | "th" | "td" | "option" | "thead" | "tbody" | "tfoot"
    )
}

fn is_void_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::tokenizer::HtmlTokenizer;
    use crate::html::NodeKind;

    #[test]
    fn builds_basic_tree() {
        let tokens = HtmlTokenizer::tokenize("<html><body>Hello</body></html>");
        let root = parse(&tokens);
        assert_eq!(root.children.len(), 1);
    }

    #[test]
    fn preserves_comments_in_the_dom() {
        let tokens = HtmlTokenizer::tokenize("<div>a<!-- note -->b</div>");
        let root = parse(&tokens);
        assert!(matches!(
            &root.children[0].children[1].kind,
            NodeKind::Comment(value) if value == " note "
        ));
    }

    #[test]
    fn closes_paragraph_before_a_block_element() {
        let tokens = HtmlTokenizer::tokenize("<p>one<div>two</div>");
        let root = parse(&tokens);
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[0].text_content(), "one");
        assert_eq!(root.children[1].tag_name(), Some("div"));
    }

    #[test]
    fn closes_paragraph_when_a_new_paragraph_starts() {
        let tokens = HtmlTokenizer::tokenize("<p>one<p>two");
        let root = parse(&tokens);
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[0].text_content(), "one");
        assert_eq!(root.children[1].text_content(), "two");
    }

    #[test]
    fn void_elements_do_not_capture_following_text() {
        let tokens = HtmlTokenizer::tokenize("<div><br>after</div>");
        let root = parse(&tokens);
        let div = &root.children[0];
        assert_eq!(div.children.len(), 2);
        assert!(matches!(div.children[0].kind, NodeKind::Element { ref name, .. } if name == "br"));
        assert_eq!(div.children[1].text_content(), "after");
    }
}
