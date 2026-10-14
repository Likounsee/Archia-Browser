use std::collections::BTreeMap;

fn is_html_whitespace(character: char) -> bool {
    matches!(character, '\t' | '\n' | '\u{000C}' | '\r' | ' ')
}

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
                name: name.into().to_ascii_lowercase(),
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

    /// Append a child without panicking if the child vector cannot grow.
    ///
    /// On failure, the caller retains ownership of the child.
    pub fn try_append(&mut self, child: Node) -> Result<(), Node> {
        if self.children.try_reserve(1).is_err() {
            return Err(child);
        }
        self.children.push(child);
        Ok(())
    }

    /// Clone this node's kind and attributes without recursively cloning its
    /// descendants. StyledNode stores descendants in its own parallel tree.
    pub(crate) fn shallow_clone(&self) -> Self {
        Self {
            kind: self.kind.clone(),
            children: Vec::new(),
        }
    }

    pub fn insert_child(&mut self, index: usize, child: Node) -> Result<(), Node> {
        if index > self.children.len() || self.children.try_reserve(1).is_err() {
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
            NodeKind::Element { attributes, .. } => attributes
                .get(&name.to_ascii_lowercase())
                .map(String::as_str),
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
            NodeKind::Element { attributes, .. } => attributes.remove(&name.to_ascii_lowercase()),
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

    /// Iterate over all descendants in document order (pre-order), without
    /// recursive calls. The node itself is intentionally excluded.
    pub fn descendants(&self) -> Descendants<'_> {
        Descendants {
            stack: self.children.iter().rev().collect(),
        }
    }

    /// Find the first descendant element with the requested tag name.
    /// The node itself is excluded. HTML element names are stored lowercase
    /// by the parser, so lookup is ASCII case-insensitive.
    pub fn find_first_element(&self, tag_name: &str) -> Option<&Node> {
        self.descendants().find(|node| {
            node.tag_name()
                .is_some_and(|name| name.eq_ignore_ascii_case(tag_name))
        })
    }

    /// Collect descendant elements matching a tag name in document order.
    /// The node itself is excluded, matching the other descendant lookup APIs.
    pub fn find_elements_by_tag(&self, tag_name: &str) -> Vec<&Node> {
        self.descendants()
            .filter(|node| {
                node.tag_name()
                    .is_some_and(|name| name.eq_ignore_ascii_case(tag_name))
            })
            .collect()
    }

    /// Find the first descendant element whose `id` exactly matches `id`.
    pub fn find_by_id(&self, id: &str) -> Option<&Node> {
        self.descendants()
            .find(|node| node.is_element() && node.attribute("id") == Some(id))
    }

    /// Collect descendant form-related elements whose `name` attribute
    /// exactly matches the requested value. The name value is case-sensitive.
    pub fn find_by_name(&self, name: &str) -> Vec<&Node> {
        self.find_by_attribute_value("name", name)
    }

    /// Find the first descendant element that has the named attribute.
    pub fn find_first_by_attribute(&self, name: &str) -> Option<&Node> {
        self.descendants()
            .find(|node| node.is_element() && node.has_attribute(name))
    }

    /// Find the first descendant element whose named attribute exactly matches
    /// the requested value.
    pub fn find_first_by_attribute_value(&self, name: &str, value: &str) -> Option<&Node> {
        self.descendants()
            .find(|node| node.is_element() && node.attribute(name) == Some(value))
    }

    /// Find the first descendant element containing the requested class token.
    pub fn find_first_by_class(&self, class_name: &str) -> Option<&Node> {
        if class_name.is_empty() || class_name.chars().any(is_html_whitespace) {
            return None;
        }
        self.descendants().find(|node| {
            node.is_element()
                && node.attribute("class").is_some_and(|classes| {
                    classes
                        .split(is_html_whitespace)
                        .any(|name| name == class_name)
                })
        })
    }

    /// Find the first descendant form-related element whose `name` value matches.
    pub fn find_first_by_name(&self, name: &str) -> Option<&Node> {
        self.find_first_by_attribute_value("name", name)
    }

    /// Collect descendant elements that have the named attribute, regardless
    /// of its value. Attribute names are ASCII case-insensitive.
    pub fn find_by_attribute(&self, name: &str) -> Vec<&Node> {
        self.descendants()
            .filter(|node| node.is_element() && node.has_attribute(name))
            .collect()
    }

    /// Collect descendant elements whose named attribute has the exact value.
    /// Attribute names are ASCII case-insensitive; values remain case-sensitive.
    pub fn find_by_attribute_value(&self, name: &str, value: &str) -> Vec<&Node> {
        self.descendants()
            .filter(|node| node.is_element() && node.attribute(name) == Some(value))
            .collect()
    }

    /// Collect descendant elements with the requested class token.
    /// Whitespace-separated class names are handled without substring matches.
    pub fn find_by_class(&self, class_name: &str) -> Vec<&Node> {
        if class_name.is_empty() || class_name.chars().any(is_html_whitespace) {
            return Vec::new();
        }
        self.descendants()
            .filter(|node| {
                node.is_element()
                    && node.attribute("class").is_some_and(|classes| {
                        classes
                            .split(is_html_whitespace)
                            .any(|name| name == class_name)
                    })
            })
            .collect()
    }

    /// Return direct child elements, excluding text and comment nodes.
    pub fn element_children(&self) -> impl Iterator<Item = &Node> {
        self.children.iter().filter(|child| child.is_element())
    }

    /// Collect descendant text in document order without recursive calls.
    ///
    /// Iterative traversal keeps this accessor safe for deeply nested trees
    /// created by DOM mutation APIs, not only trees produced by the bounded parser.
    pub fn text_content(&self) -> String {
        let mut output = String::new();
        let mut stack = vec![self];

        while let Some(node) = stack.pop() {
            if let NodeKind::Text(text) = &node.kind {
                output.push_str(text);
            }
            stack.extend(node.children.iter().rev());
        }

        output
    }
}

/// Explicit-stack preorder traversal over a DOM subtree.
pub struct Descendants<'a> {
    stack: Vec<&'a Node>,
}

impl<'a> Iterator for Descendants<'a> {
    type Item = &'a Node;

    fn next(&mut self) -> Option<Self::Item> {
        let node = self.stack.pop()?;
        self.stack.extend(node.children.iter().rev());
        Some(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_content_preserves_document_order_through_nested_nodes() {
        let mut root = Node::element("div");
        let mut first = Node::element("span");
        first.append(Node::text("one"));
        let mut nested = Node::element("em");
        nested.append(Node::text("two"));
        first.append(nested);
        root.append(first);
        root.append(Node::text("three"));
        assert_eq!(root.text_content(), "onetwothree");
    }

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
    fn fallible_append_retains_child_when_append_succeeds() {
        let mut parent = Node::element("div");
        let child = Node::text("safe");
        assert!(parent.try_append(child).is_ok());
        assert_eq!(parent.child_count(), 1);
        assert_eq!(
            parent.child(0).map(Node::text_content).as_deref(),
            Some("safe")
        );
    }

    #[test]
    fn attributes_can_be_mutated_and_read() {
        let mut node = Node::element("div");
        node.set_attribute("CLASS", "card");
        assert_eq!(node.attribute("CLASS"), Some("card"));
        assert_eq!(node.attribute("class"), Some("card"));
        assert_eq!(node.remove_attribute("CLASS"), Some("card".into()));
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
    fn element_constructor_normalizes_html_tag_names() {
        let element = Node::element("DiV");
        assert_eq!(element.tag_name(), Some("div"));
        assert!(element.is_element());
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
    fn finds_all_matching_tags_in_document_order() {
        let mut root = Node::element("section");
        let mut first = Node::element("div");
        first.append(Node::element("span"));
        root.append(first);
        root.append(Node::element("DIV"));

        let matches = root.find_elements_by_tag("div");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].tag_name(), Some("div"));
        assert_eq!(matches[1].tag_name(), Some("div"));
    }

    #[test]
    fn find_first_element_excludes_the_receiver() {
        let mut root = Node::element("section");
        root.append(Node::element("section"));

        assert_eq!(
            root.find_first_element("section").and_then(Node::tag_name),
            Some("section")
        );
        assert!(root.find_first_element("missing").is_none());
    }

    #[test]
    fn descendant_traversal_is_preorder_and_excludes_self() {
        let mut root = Node::element("root");
        let mut first = Node::element("first");
        first.append(Node::element("nested"));
        root.append(first);
        root.append(Node::text("text"));
        root.append(Node::element("last"));

        let names: Vec<_> = root.descendants().filter_map(Node::tag_name).collect();
        assert_eq!(names, ["first", "nested", "last"]);
    }

    #[test]
    fn first_match_queries_return_the_earliest_descendant() {
        let mut root = Node::document();
        let mut first = Node::element("input");
        first.set_attribute("name", "email");
        first.set_attribute("class", "field required");
        first.set_attribute("data-state", "");
        let mut second = Node::element("input");
        second.set_attribute("name", "email");
        second.set_attribute("class", "field");
        root.append(first);
        root.append(second);

        assert_eq!(
            root.find_first_by_name("email").and_then(Node::tag_name),
            Some("input")
        );
        assert_eq!(
            root.find_first_by_attribute("data-state")
                .and_then(|node| node.attribute("data-state")),
            Some("")
        );
        assert_eq!(
            root.find_first_by_class("required").and_then(Node::tag_name),
            Some("input")
        );
        assert_eq!(root.find_first_by_attribute_value("name", "missing"), None);
        assert_eq!(root.find_first_by_class("field required extra"), None);
    }

    #[test]
    fn name_queries_match_exact_values_in_document_order() {
        let mut root = Node::document();
        let mut first = Node::element("input");
        first.set_attribute("name", "email");
        let mut second = Node::element("button");
        second.set_attribute("name", "email-address");
        let mut third = Node::element("textarea");
        third.set_attribute("NAME", "email");
        root.append(first);
        root.append(second);
        root.append(third);

        let matches = root.find_by_name("email");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].tag_name(), Some("input"));
        assert_eq!(matches[1].tag_name(), Some("textarea"));
        assert!(root.find_by_name("EMAIL").is_empty());
    }

    #[test]
    fn attribute_presence_queries_include_empty_values_and_ignore_case() {
        let mut root = Node::document();
        let mut first = Node::element("input");
        first.set_attribute("DISABLED", "");
        let mut second = Node::element("button");
        second.set_attribute("disabled", "disabled");
        root.append(first);
        root.append(Node::element("button"));
        root.append(second);

        let matches = root.find_by_attribute("Disabled");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].tag_name(), Some("input"));
        assert_eq!(matches[1].tag_name(), Some("button"));
    }

    #[test]
    fn attribute_value_queries_match_exact_values_in_document_order() {
        let mut root = Node::document();
        let mut first = Node::element("a");
        first.set_attribute("DATA-kind", "guide");
        let mut nested = Node::element("span");
        nested.set_attribute("data-kind", "guide");
        first.append(nested);
        let mut partial = Node::element("a");
        partial.set_attribute("data-kind", "guide-extra");
        root.append(first);
        root.append(partial);

        let matches = root.find_by_attribute_value("DATA-KIND", "guide");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].tag_name(), Some("a"));
        assert_eq!(matches[1].tag_name(), Some("span"));
        assert!(root
            .find_by_attribute_value("data-kind", "GUIDE")
            .is_empty());
    }

    #[test]
    fn class_queries_use_html_whitespace_not_unicode_whitespace() {
        let mut root = Node::document();
        let mut element = Node::element("div");
        element.set_attribute("class", "wide\u{2003}screen");
        root.append(element);

        assert_eq!(root.find_by_class("wide\u{2003}screen").len(), 1);
        assert!(root.find_by_class("wide").is_empty());
        assert!(root.find_by_class("screen").is_empty());
    }

    #[test]
    fn id_and_class_queries_match_tokens_not_substrings() {
        let mut root = Node::document();
        let mut first = Node::element("div");
        first.set_attribute("id", "target");
        first.set_attribute("class", "card selected");
        let mut second = Node::element("span");
        second.set_attribute("id", "target-extra");
        second.set_attribute("class", "cardinal");
        root.append(first);
        root.append(second);

        assert_eq!(
            root.find_by_id("target").and_then(Node::tag_name),
            Some("div")
        );
        assert_eq!(root.find_by_id("missing"), None);
        assert_eq!(root.find_by_class("card").len(), 1);
        assert!(root.find_by_class("ar").is_empty());
        assert!(root.find_by_class("two words").is_empty());
    }

    #[test]
    fn tag_lookup_is_ascii_case_insensitive() {
        let mut root = Node::document();
        root.append(Node::element("section"));
        assert_eq!(
            root.find_first_element("SECTION").and_then(Node::tag_name),
            Some("section")
        );
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
