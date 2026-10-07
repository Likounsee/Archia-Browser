use super::{dom::Node, tokenizer::HtmlToken};

pub fn parse(tokens: &[HtmlToken]) -> Node {
    let mut root = Node::document();
    let mut stack: Vec<Node> = Vec::new();

    for token in tokens {
        match token {
            HtmlToken::Doctype(_) | HtmlToken::Comment(_) => {}
            HtmlToken::Text(text) => {
                let node = Node::text(text);
                if let Some(parent) = stack.last_mut() { parent.append(node); }
                else { root.append(node); }
            }
            HtmlToken::StartTag { name, self_closing } => {
                let node = Node::element(name.clone());
                if *self_closing {
                    if let Some(parent) = stack.last_mut() { parent.append(node); }
                    else { root.append(node); }
                } else {
                    stack.push(node);
                }
            }
            HtmlToken::EndTag(name) => {
                if let Some(position) = stack.iter().rposition(|node| {
                    matches!(&node.kind, super::dom::NodeKind::Element { name: node_name, .. } if node_name == name)
                }) {
                    let mut completed = stack.split_off(position);
                    if let Some(node) = completed.pop() {
                        if let Some(parent) = stack.last_mut() { parent.append(node); }
                        else { root.append(node); }
                    }
                }
            }
        }
    }

    while let Some(node) = stack.pop() {
        if let Some(parent) = stack.last_mut() { parent.append(node); }
        else { root.append(node); }
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
    }
}
