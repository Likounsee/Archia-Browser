use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Document,
    Element {
        name: String,
        attributes: BTreeMap<String, String>,
    },
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
        Self {
            kind: NodeKind::Document,
            children: Vec::new(),
        }
    }

    pub fn element(name: impl Into<String>) -> Self {
        Self {
            kind: NodeKind::Element {
                name: name.into(),
                attributes: BTreeMap::new(),
            },
            children: Vec::new(),
        }
    }

    pub fn text(value: impl Into<String>) -> Self {
        Self {
            kind: NodeKind::Text(value.into()),
            children: Vec::new(),
        }
    }

    pub fn comment(value: impl Into<String>) -> Self {
        Self {
            kind: NodeKind::Comment(value.into()),
            children: Vec::new(),
        }
    }

    pub fn append(&mut self, child: Node) {
        self.children.push(child);
    }

    pub fn children(&self) -> &[Node] {
        &self.children
    }

    pub fn children_mut(&mut self) -> &mut [Node] {
        &mut self.children
    }

    pub fn is_element(&self) -> bool {
        matches!(self.kind, NodeKind::Element { .. })
    }

    pub fn tag_name(&self) -> Option<&str> {
        match &self.kind {
            NodeKind::Element { name, .. } => Some(name),
            _ => None,
        }
    }

    pub fn attribute(&self, name: &str) -> Option<&str> {
        match &self.kind {
            NodeKind::Element { attributes, .. } => attributes.get(name).map(String::as_str),
            _ => None,
        }
    }

    pub fn set_attribute(&mut self, name: impl Into<String>, value: impl Into<String>) {
        if let NodeKind::Element { attributes, .. } = &mut self.kind {
            attributes.insert(name.into().to_ascii_lowercase(), value.into());
        }
    }

    pub fn remove_attribute(&mut self, name: &str) -> Option<String> {
        match &mut self.kind {
            NodeKind::Element { attributes, .. } => attributes.remove(name),
            _ => None,
        }
    }

    pub fn text_content(&self) -> String {
        let mut output = String::new();
        self.append_text_content(&mut output);
        output
    }

    fn append_text_content(&self, output: &mut String) {
        if let NodeKind::Text(text) = &self.kind {
            output.push_str(text);
        }
        for child in &self.children {
            child.append_text_content(output);
        }
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
        assert_eq!(document.children[0].children()[0].text_content(), "Hello");
        assert_eq!(body.tag_name(), Some("body"));
    }

    #[test]
    fn attributes_can_be_mutated_and_read() {
        let mut node = Node::element("div");
        node.set_attribute("CLASS", "card");
        assert_eq!(node.attribute("CLASS"), None);
        assert_eq!(node.attribute("class"), Some("card"));
        assert_eq!(node.remove_attribute("class"), Some("card".into()));
        assert_eq!(node.attribute("class"), None);
    }
}
