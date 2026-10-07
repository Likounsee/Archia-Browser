use super::{dom::Node, tokenizer::HtmlToken};

pub fn parse(tokens: &[HtmlToken]) -> Node {
    let mut root = Node::document();
    let mut stack: Vec<Node> = Vec::new();

    for token in tokens {
        match token {
            HtmlToken::Doctype(_) => {}
            HtmlToken::Comment(text) => append_node(&mut root, &mut stack, Node::comment(text)),
            HtmlToken::Text(text) => append_node(&mut root, &mut stack, Node::text(text)),
            HtmlToken::StartTag {
                name,
                attributes,
                self_closing,
            } => {
                let mut node = Node::element(name.clone());
                if let super::dom::NodeKind::Element {
                    attributes: target,
                    ..
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
            HtmlToken::EndTag(name) => close_element(&mut root, &mut stack, name),
        }
    }

    while let Some(node) = stack.pop() {
        append_node(&mut root, &mut stack, node);
    }

    root
}

fn append_node(root: &mut Node, stack: &mut [Node], node: Node) {
    if let Some(parent) = stack.last_mut() {
        parent.append(node);
    } else {
        root.append(node);
    }
}

fn close_element(root: &mut Node, stack: &mut Vec<Node>, name: &str) {
    let Some(position) = stack.iter().rposition(|node| {
        matches!(
            &node.kind,
            super::dom::NodeKind::Element {
                name: node_name,
                ..
            } if node_name == name
        )
    }) else {
        return;
    };

    while stack.len() > position + 1 {
        let child = stack.pop().expect("stack length checked");
        if let Some(parent) = stack.last_mut() {
            parent.append(child);
        }
    }

    let completed = stack.pop().expect("matching element exists");
    append_node(root, stack, completed);
}

fn is_void_element(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
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

    #[test]
    fn builds_basic_tree() {
        let tokens = HtmlTokenizer::tokenize("<html><body>Hello</body></html>");
        let root = parse(&tokens);
        assert_eq!(root.children.len(), 1);
        assert_eq!(root.children[0].children[0].text_content(), "Hello");
    }

    #[test]
    fn void_elements_do_not_capture_following_content() {
        let tokens = HtmlTokenizer::tokenize("<div><br><span>text</span></div>");
        let root = parse(&tokens);
        let div = &root.children[0];
        assert_eq!(div.children.len(), 2);
        assert_eq!(div.children[0].tag_name(), Some("br"));
        assert_eq!(div.children[1].text_content(), "text");
    }

    #[test]
    fn mismatched_end_tags_close_open_elements_in_order() {
        let tokens = HtmlTokenizer::tokenize("<div><span>text</div>");
        let root = parse(&tokens);
        assert_eq!(root.children[0].text_content(), "text");
        assert_eq!(root.children[0].children.len(), 1);
        assert_eq!(root.children[0].children[0].tag_name(), Some("span"));
    }

    #[test]
    fn preserves_comments() {
        let tokens = HtmlTokenizer::tokenize("<div><!-- hello --><span>text</span></div>");
        let root = parse(&tokens);
        assert!(matches!(
            root.children[0].children[0].kind,
            super::super::dom::NodeKind::Comment(_)
        ));
    }
}
