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

    pub fn insert_child(&mut self, index: usize, child: Node) -> Result<(), Node> {
        if index > self.children.len() {
            return Err(child);
        }
        self.children.insert(index, child);
        Ok(())
    }

    pub fn remove_child(&mut self, index: usize) -> Option<Node> {
        (index < self.children.len()).then(|| self.children.remove(index))
    }

    pub fn replace_child(&mut self, index: usize, child: Node) -> Result<Node, Node> {
        let Some(existing) = self.children.get_mut(index) else {
            return Err(child);
        };
        Ok(std::mem::replace(existing, child))
    }

    pub fn child(&self, index: usize) -> Option<&Node> {
        self.children.get(index)
    }

    pub fn child_mut(&mut self, index: usize) -> Option<&mut Node> {
        self.children.get_mut(index)
    }

    pub fn child_count(&self) -> usize {
        self.children.len()
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

    pub fn has_attribute(&self, name: &str) -> bool {
        match &self.kind {
            NodeKind::Element { attributes, .. } => {
                attributes.contains_key(&name.to_ascii_lowercase())
            }
            _ => false,
        }
    }

    /// Return the destination of a normal HTML anchor element.
    ///
    /// Navigation policy stays in the browser layer; the DOM only exposes the
    /// authored href value and does not interpret schemes or resolve it.
    pub fn link_href(&self) -> Option<&str> {
        (self.tag_name() == Some("a"))
            .then(|| self.attribute("href"))
            .flatten()
    }

    pub fn attributes(&self) -> Option<&BTreeMap<String, String>> {
        match &self.kind {
            NodeKind::Element { attributes, .. } => Some(attributes),
            _ => None,
        }
    }

    pub fn find_first_element(&self, tag_name: &str) -> Option<&Node> {
        if self.tag_name() == Some(tag_name) {
            return Some(self);
        }

        self.children
            .iter()
            .find_map(|child| child.find_first_element(tag_name))
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
        assert_eq!(body.tag_name(), Some("body"));
        document.append(body);
        assert_eq!(document.children.len(), 1);
        assert_eq!(document.children[0].children()[0].text_content(), "Hello");
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

    #[test]
    fn supports_dom_child_mutation_operations() {
        let mut parent = Node::element("div");
        parent.append(Node::element("first"));
        parent.append(Node::element("third"));

        assert!(parent.insert_child(1, Node::element("second")).is_ok());
        assert_eq!(parent.child_count(), 3);
        assert_eq!(parent.child(1).and_then(Node::tag_name), Some("second"));

        let replaced = parent
            .replace_child(1, Node::element("replacement"))
            .unwrap();
        assert_eq!(replaced.tag_name(), Some("second"));
        assert_eq!(
            parent.child(1).and_then(Node::tag_name),
            Some("replacement")
        );

        let removed = parent.remove_child(0).unwrap();
        assert_eq!(removed.tag_name(), Some("first"));
        assert_eq!(parent.child_count(), 2);
        assert_eq!(
            parent.child_mut(0).and_then(|node| node.tag_name()),
            Some("replacement")
        );

        assert!(parent.insert_child(99, Node::element("invalid")).is_err());
        assert!(parent.replace_child(99, Node::element("invalid")).is_err());
        assert!(parent.remove_child(99).is_none());
    }

    #[test]
    fn finds_first_matching_descendant() {
        let mut document = Node::document();
        let mut body = Node::element("body");
        let mut section = Node::element("section");
        section.append(Node::element("title"));
        body.append(section);
        document.append(body);

        assert_eq!(
            document
                .find_first_element("title")
                .and_then(Node::tag_name),
            Some("title")
        );
        assert!(document.find_first_element("missing").is_none());
    }

    #[test]
    fn exposes_only_anchor_href_values_as_link_targets() {
        let mut anchor = Node::element("a");
        anchor.set_attribute("href", " /docs/guide.html ");
        assert_eq!(anchor.link_href(), Some(" /docs/guide.html "));

        let mut div = Node::element("div");
        div.set_attribute("href", "/not-a-link");
        assert_eq!(div.link_href(), None);
    }
}
