use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Document,
    Element { name: String, attributes: BTreeMap<String, String> },
    Text(String),
    Comment(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub kind: NodeKind,
    pub children: Vec<Node>,
}

impl Node {
    pub fn document() -> Self {
        Self { kind: NodeKind::Document, children: Vec::new() }
    }

    pub fn element(name: impl Into<String>) -> Self {
        Self {
            kind: NodeKind::Element { name: name.into(), attributes: BTreeMap::new() },
            children: Vec::new(),
        }
    }

    pub fn text(value: impl Into<String>) -> Self {
        Self { kind: NodeKind::Text(value.into()), children: Vec::new() }
    }

    pub fn comment(value: impl Into<String>) -> Self {
        Self { kind: NodeKind::Comment(value.into()), children: Vec::new() }
    }

    pub fn append(&mut self, child: Node) {
        self.children.push(child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_can_own_children() {
        let mut document = Node::document();
        let mut body = Node::element("body");
        body.append(Node::text("Hello"));
        document.append(body);
        assert_eq!(document.children.len(), 1);
    }
}
