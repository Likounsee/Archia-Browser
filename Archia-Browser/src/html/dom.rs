use std::collections::BTreeMap;

/// Convert a dataset property name to its corresponding data-* attribute.
fn dataset_key_to_attribute(key: &str) -> Option<String> {
    if key.is_empty() || key.chars().any(is_html_whitespace) {
        return None;
    }
    let mut attribute = String::from("data-");
    for character in key.chars() {
        if character.is_ascii_uppercase() {
            attribute.push('-');
            attribute.push(character.to_ascii_lowercase());
        } else {
            attribute.push(character);
        }
    }
    Some(attribute)
}

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

    /// Insert a new child immediately before the child at `index`.
    /// Returns the child unchanged when the reference index does not exist.
    pub fn insert_before(&mut self, index: usize, child: Node) -> Result<(), Node> {
        if index >= self.children.len() || self.children.try_reserve(1).is_err() {
            return Err(child);
        }
        self.children.insert(index, child);
        Ok(())
    }

    /// Insert a new child immediately after the child at `index`.
    pub fn insert_after(&mut self, index: usize, child: Node) -> Result<(), Node> {
        if index >= self.children.len() || self.children.try_reserve(1).is_err() {
            return Err(child);
        }
        self.children.insert(index + 1, child);
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

    /// Whether this node currently has at least one child node, including text
    /// and comment nodes.
    pub fn has_child_nodes(&self) -> bool {
        !self.children.is_empty()
    }

    /// Return the first child node, including text and comments.
    pub fn first_child(&self) -> Option<&Node> {
        self.children.first()
    }

    /// Return the last child node, including text and comments.
    pub fn last_child(&self) -> Option<&Node> {
        self.children.last()
    }

    /// Number of element children, excluding text and comment nodes.
    pub fn child_element_count(&self) -> usize {
        self.children
            .iter()
            .filter(|child| child.is_element())
            .count()
    }

    /// First element child, skipping text and comment nodes.
    pub fn first_element_child(&self) -> Option<&Node> {
        self.children.iter().find(|child| child.is_element())
    }

    /// Last element child, skipping text and comment nodes.
    pub fn last_element_child(&self) -> Option<&Node> {
        self.children.iter().rev().find(|child| child.is_element())
    }

    /// Element children in document order.
    pub fn element_children(&self) -> Vec<&Node> {
        self.children
            .iter()
            .filter(|child| child.is_element())
            .collect()
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

    pub fn is_document(&self) -> bool {
        matches!(self.kind, NodeKind::Document)
    }

    pub fn is_text(&self) -> bool {
        matches!(self.kind, NodeKind::Text(_))
    }

    pub fn is_comment(&self) -> bool {
        matches!(self.kind, NodeKind::Comment(_))
    }

    /// DOM-compatible numeric node type: Document=9, Element=1, Text=3, Comment=8.
    pub fn node_type(&self) -> u16 {
        match self.kind {
            NodeKind::Element { .. } => 1,
            NodeKind::Text(_) => 3,
            NodeKind::Comment(_) => 8,
            NodeKind::Document => 9,
        }
    }

    /// DOM-compatible node name. HTML element names are exposed uppercase.
    pub fn node_name(&self) -> String {
        match &self.kind {
            NodeKind::Document => "#document".to_owned(),
            NodeKind::Element { name, .. } => name.to_ascii_uppercase(),
            NodeKind::Text(_) => "#text".to_owned(),
            NodeKind::Comment(_) => "#comment".to_owned(),
        }
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

    /// DOM-style alias for retrieving an element attribute.
    pub fn get_attribute(&self, name: &str) -> Option<&str> {
        self.attribute(name)
    }

    /// Number of attributes on this element; non-elements report zero.
    pub fn attribute_count(&self) -> usize {
        self.attributes().map_or(0, BTreeMap::len)
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

    /// DOM id property. Non-elements return an empty string.
    pub fn id(&self) -> &str {
        self.attribute("id").unwrap_or("")
    }

    /// Set the DOM id property on an element.
    pub fn set_id(&mut self, value: impl Into<String>) {
        self.set_attribute("id", value);
    }

    /// DOM className property. Non-elements return an empty string.
    pub fn class_name(&self) -> &str {
        self.attribute("class").unwrap_or("")
    }

    /// Replace the complete className value on an element.
    pub fn set_class_name(&mut self, value: impl Into<String>) {
        self.set_attribute("class", value);
    }

    /// Read a data-* attribute using its dataset camelCase key.
    pub fn dataset_value(&self, key: &str) -> Option<&str> {
        let attribute = dataset_key_to_attribute(key)?;
        self.attribute(&attribute)
    }

    /// Set a data-* attribute using its dataset camelCase key.
    pub fn set_dataset_value(&mut self, key: &str, value: impl Into<String>) -> bool {
        let Some(attribute) = dataset_key_to_attribute(key) else {
            return false;
        };
        self.set_attribute(attribute, value);
        true
    }

    /// Remove a data-* attribute using its dataset camelCase key.
    pub fn remove_dataset_value(&mut self, key: &str) -> Option<String> {
        let attribute = dataset_key_to_attribute(key)?;
        self.remove_attribute(&attribute)
    }

    /// Set or remove a boolean attribute. Returns the resulting presence state.
    pub fn toggle_attribute(&mut self, name: &str, force: Option<bool>) -> bool {
        let present = self.has_attribute(name);
        let should_be_present = force.unwrap_or(!present);
        if should_be_present {
            if !present {
                self.set_attribute(name, "");
            }
        } else if present {
            self.remove_attribute(name);
        }
        should_be_present
    }

    /// Return whitespace-separated class tokens in source order.
    pub fn class_list(&self) -> Vec<&str> {
        self.attribute("class")
            .map(|classes| {
                classes
                    .split(is_html_whitespace)
                    .filter(|token| !token.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn has_class(&self, class_name: &str) -> bool {
        !class_name.is_empty()
            && !class_name.chars().any(is_html_whitespace)
            && self.class_list().contains(&class_name)
    }

    /// Add a class token without duplicating an existing token.
    /// Invalid empty or whitespace-containing tokens are ignored.
    pub fn add_class(&mut self, class_name: &str) {
        if class_name.is_empty() || class_name.chars().any(is_html_whitespace) {
            return;
        }
        if self.has_class(class_name) {
            return;
        }
        let mut classes = self.class_list().join(" ");
        if !classes.is_empty() {
            classes.push(' ');
        }
        classes.push_str(class_name);
        self.set_attribute("class", classes);
    }

    pub fn remove_class(&mut self, class_name: &str) {
        if class_name.is_empty() || class_name.chars().any(is_html_whitespace) {
            return;
        }
        let remaining = self
            .class_list()
            .into_iter()
            .filter(|token| *token != class_name)
            .collect::<Vec<_>>();
        if remaining.is_empty() {
            self.remove_attribute("class");
        } else {
            self.set_attribute("class", remaining.join(" "));
        }
    }

    /// Toggle a class token with an optional forced presence state.
    pub fn toggle_class(&mut self, class_name: &str, force: Option<bool>) -> bool {
        if class_name.is_empty() || class_name.chars().any(is_html_whitespace) {
            return false;
        }
        let present = self.has_class(class_name);
        let should_be_present = force.unwrap_or(!present);
        if should_be_present {
            self.add_class(class_name);
        } else {
            self.remove_class(class_name);
        }
        should_be_present
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

    /// Whether this element carries the HTML disabled boolean attribute.
    pub fn is_disabled(&self) -> bool {
        self.has_attribute("disabled")
    }

    /// Whether this element carries the HTML required boolean attribute.
    pub fn is_required(&self) -> bool {
        self.has_attribute("required")
    }

    /// Default checked state for checkbox and radio inputs.
    /// Runtime checkedness can diverge after user interaction and belongs in form state.
    pub fn is_checked(&self) -> bool {
        self.tag_name() == Some("input")
            && matches!(
                self.attribute("type")
                    .unwrap_or("text")
                    .to_ascii_lowercase()
                    .as_str(),
                "checkbox" | "radio"
            )
            && self.has_attribute("checked")
    }

    /// Default selected state for an option element.
    pub fn is_selected(&self) -> bool {
        self.tag_name() == Some("option") && self.has_attribute("selected")
    }

    /// Read the authored/default value for common form controls.
    ///
    /// This does not model live user-edited value state, select-option
    /// selection algorithms, or browser-specific default values.
    pub fn form_value(&self) -> Option<String> {
        match self.tag_name()? {
            "input" | "button" => Some(self.attribute("value").unwrap_or("").to_owned()),
            "textarea" => Some(self.text_content()),
            "option" => Some(
                self.attribute("value")
                    .map(str::to_owned)
                    .unwrap_or_else(|| self.text_content()),
            ),
            _ => None,
        }
    }

    pub fn attributes(&self) -> Option<&BTreeMap<String, String>> {
        match &self.kind {
            NodeKind::Element { attributes, .. } => Some(attributes),
            _ => None,
        }
    }

    /// Whether this element currently has one or more attributes.
    pub fn has_attributes(&self) -> bool {
        self.attributes()
            .is_some_and(|attributes| !attributes.is_empty())
    }

    /// Return attribute names in the DOM's deterministic map order.
    pub fn attribute_names(&self) -> Vec<&str> {
        self.attributes()
            .map(|attributes| attributes.keys().map(String::as_str).collect())
            .unwrap_or_default()
    }

    /// Replace all children at once, returning the previous child nodes.
    pub fn replace_children(&mut self, children: Vec<Node>) -> Vec<Node> {
        std::mem::replace(&mut self.children, children)
    }

    /// Normalize this subtree by removing empty text nodes and merging
    /// adjacent text nodes, while preserving comments and element boundaries.
    pub fn normalize(&mut self) {
        let mut normalized: Vec<Node> = Vec::with_capacity(self.children.len());
        for mut child in std::mem::take(&mut self.children) {
            child.normalize();
            if let NodeKind::Text(value) = &child.kind {
                if value.is_empty() {
                    continue;
                }
            }
            if let NodeKind::Text(value) = child.kind {
                if let Some(Node {
                    kind: NodeKind::Text(previous),
                    ..
                }) = normalized.last_mut()
                {
                    previous.push_str(&value);
                    continue;
                }
                normalized.push(Node::text(value));
            } else {
                normalized.push(child);
            }
        }
        self.children = normalized;
    }

    /// Iterate over all descendants in document order (pre-order), without
    /// recursive calls. The node itself is intentionally excluded.
    pub fn descendants(&self) -> Descendants<'_> {
        Descendants {
            stack: self.children.iter().rev().collect(),
        }
    }

    /// Test node identity, including the receiver itself, as required by
    /// the DOM Node.contains() contract.
    pub fn contains(&self, other: &Node) -> bool {
        std::ptr::eq(self, other) || self.descendants().any(|node| std::ptr::eq(node, other))
    }

    /// DOM-style getElementById alias. Searches descendants, not the receiver.
    pub fn get_element_by_id(&self, id: &str) -> Option<&Node> {
        self.find_by_id(id)
    }

    /// DOM-style getElementsByTagName. The wildcard matches every descendant
    /// element; the receiver itself is excluded.
    pub fn get_elements_by_tag_name(&self, tag_name: &str) -> Vec<&Node> {
        if tag_name == "*" {
            return self
                .descendants()
                .filter(|node| node.is_element())
                .collect();
        }
        self.find_elements_by_tag(tag_name)
    }

    /// DOM-style getElementsByClassName. Every whitespace-separated class
    /// token in the query must be present on a matching descendant element.
    pub fn get_elements_by_class_name(&self, class_names: &str) -> Vec<&Node> {
        let required: Vec<&str> = class_names
            .split(is_html_whitespace)
            .filter(|token| !token.is_empty())
            .collect();
        if required.is_empty() {
            return Vec::new();
        }
        self.descendants()
            .filter(|node| node.is_element() && required.iter().all(|class| node.has_class(class)))
            .collect()
    }

    /// Test whether this element matches a compound CSS selector or selector
    /// list. Combinators are rejected because this node alone has no ancestor
    /// or sibling context; use querySelector on a tree for those selectors.
    pub fn matches_selector(&self, selector: &str) -> bool {
        if !self.is_element() {
            return false;
        }
        parse_dom_selector_list(selector)
            .is_some_and(|selectors| selectors.iter().any(|selector| selector.matches(self)))
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

    /// Collect descendant elements matching a tag and an exact attribute value.
    /// Tag and attribute names are ASCII case-insensitive; values are case-sensitive.
    pub fn find_by_tag_and_attribute_value(
        &self,
        tag_name: &str,
        attribute_name: &str,
        value: &str,
    ) -> Vec<&Node> {
        self.descendants()
            .filter(|node| {
                node.tag_name()
                    .is_some_and(|tag| tag.eq_ignore_ascii_case(tag_name))
                    && node.attribute(attribute_name) == Some(value)
            })
            .collect()
    }

    /// Collect descendant elements matching a tag and a class token.
    pub fn find_by_tag_and_class(&self, tag_name: &str, class_name: &str) -> Vec<&Node> {
        if class_name.is_empty() || class_name.chars().any(is_html_whitespace) {
            return Vec::new();
        }
        self.descendants()
            .filter(|node| {
                node.tag_name()
                    .is_some_and(|tag| tag.eq_ignore_ascii_case(tag_name))
                    && node.attribute("class").is_some_and(|classes| {
                        classes
                            .split(is_html_whitespace)
                            .any(|name| name == class_name)
                    })
            })
            .collect()
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

    /// Return the first descendant element matching a CSS selector.
    ///
    /// This follows querySelector semantics: the receiver itself is not included.
    pub fn query_selector(&self, selector: &str) -> Option<&Node> {
        self.query_selector_all(selector).into_iter().next()
    }

    /// Return all descendant elements matching a CSS selector in document order.
    ///
    /// The selector engine evaluates the complete ancestor and sibling path, so
    /// descendant, child and sibling combinators work across the DOM tree.
    pub fn query_selector_all(&self, selector: &str) -> Vec<&Node> {
        let Some(selectors) = parse_dom_selector_list(selector) else {
            return Vec::new();
        };

        let mut matches = Vec::new();
        let mut stack: Vec<(&Node, Vec<&Node>, Vec<&[Node]>, Vec<usize>)> = Vec::new();

        let root_path = vec![self];
        let root_siblings: Vec<&[Node]> = vec![&[]];
        let root_positions = vec![0];

        for (index, child) in self.children.iter().enumerate().rev() {
            stack.push((child, root_path.clone(), root_siblings.clone(), {
                let mut positions = root_positions.clone();
                positions.push(index);
                positions
            }));
        }

        while let Some((node, mut path, mut sibling_lists, mut positions)) = stack.pop() {
            let siblings = path
                .last()
                .map(|parent| parent.children.as_slice())
                .unwrap_or(&[]);
            path.push(node);
            sibling_lists.push(siblings);

            if node.is_element()
                && selectors.iter().any(|selector| {
                    selector.matches_path_with_siblings(&path, &sibling_lists, &positions)
                })
            {
                matches.push(node);
            }

            let children = node.children.as_slice();
            for (index, child) in children.iter().enumerate().rev() {
                let mut child_positions = positions.clone();
                child_positions.push(index);
                stack.push((child, path.clone(), sibling_lists.clone(), child_positions));
            }
        }

        matches
    }

    /// Serialize this node's children as HTML, equivalent to the DOM innerHTML getter.
    pub fn inner_html(&self) -> String {
        let mut output = String::new();
        for child in &self.children {
            serialize_html(child, &mut output);
        }
        output
    }

    /// Serialize this node as HTML, equivalent to the DOM outerHTML getter.
    /// Documents have no element wrapper and serialize their children only.
    pub fn outer_html(&self) -> String {
        let mut output = String::new();
        serialize_html(self, &mut output);
        output
    }

    /// Replace this element/document's children by parsing an HTML fragment.
    ///
    /// The current parser uses its document-mode recovery rules for fragments;
    /// context-sensitive fragment parsing (table/select/template) is still a
    /// separate HTML5 tree-builder task. Returns false for text/comment nodes
    /// and HTML void elements, which cannot contain children.
    pub fn set_inner_html(&mut self, html: &str) -> bool {
        if !(self.is_document() || self.is_element()) {
            return false;
        }
        if self.tag_name().is_some_and(is_void_html_element) {
            return false;
        }

        let tokens = super::HtmlTokenizer::tokenize(html);
        let fragment = super::parser::parse_fragment(&tokens);
        self.replace_children(fragment.children);
        true
    }

    /// Append parsed HTML nodes to this element/document.
    /// Returns false when the receiver cannot contain child nodes.
    pub fn append_html(&mut self, html: &str) -> bool {
        if !(self.is_document() || self.is_element()) {
            return false;
        }
        if self.tag_name().is_some_and(is_void_html_element) {
            return false;
        }

        let tokens = super::HtmlTokenizer::tokenize(html);
        let fragment = super::parser::parse_fragment(&tokens);
        if self.children.try_reserve(fragment.children.len()).is_err() {
            return false;
        }
        self.children.extend(fragment.children);
        true
    }

    /// Insert parsed HTML nodes before the receiver's existing first child.
    pub fn prepend_html(&mut self, html: &str) -> bool {
        if !(self.is_document() || self.is_element()) {
            return false;
        }
        if self.tag_name().is_some_and(is_void_html_element) {
            return false;
        }

        let tokens = super::HtmlTokenizer::tokenize(html);
        let mut fragment = super::parser::parse_fragment(&tokens);
        if fragment.children.try_reserve(self.children.len()).is_err() {
            return false;
        }
        fragment.children.append(&mut self.children);
        self.children = fragment.children;
        true
    }

    /// Set the node's text content without interpreting the value as markup.
    /// For elements/documents this replaces children; for text/comment nodes
    /// this replaces the node data.
    pub fn set_text_content(&mut self, value: impl Into<String>) {
        let value = value.into();
        match &mut self.kind {
            NodeKind::Document => self.children.clear(),
            NodeKind::Element { .. } => {
                self.children.clear();
                if !value.is_empty() {
                    self.children.push(Node::text(value));
                }
            }
            NodeKind::Text(data) | NodeKind::Comment(data) => *data = value,
        }
    }

    /// Collect text intended for a basic visual page view.
    ///
    /// Unlike DOM textContent, this omits metadata and non-rendered source
    /// containers. It deliberately does not attempt CSS layout or visibility
    /// computation; the renderer can replace this approximation later.
    pub fn visible_text_content(&self) -> String {
        const NON_VISIBLE: &[&str] = &["head", "script", "style", "template", "noscript"];

        let mut output = String::new();
        let mut stack = vec![self];

        while let Some(node) = stack.pop() {
            if let Some(tag) = node.tag_name() {
                if NON_VISIBLE.contains(&tag)
                    || node.attribute("hidden").is_some()
                    || node.attribute("aria-hidden") == Some("true")
                {
                    continue;
                }
            }

            if let NodeKind::Text(text) = &node.kind {
                output.push_str(text);
            }
            stack.extend(node.children.iter().rev());
        }

        output
    }
}

/// Parse a CSS selector list while keeping commas inside strings, attributes,
/// escaped sequences and functional pseudo-classes intact.
fn parse_dom_selector_list(input: &str) -> Option<Vec<crate::css::Selector>> {
    if input.len() > 4_096 {
        return None;
    }

    let mut selectors = Vec::new();
    let mut start = 0;
    let mut brackets = 0usize;
    let mut parentheses = 0usize;
    let mut quote = None;
    let mut escaped = false;

    for (index, character) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' if brackets > 0 || parentheses > 0 => quote = Some(character),
            '[' => brackets += 1,
            ']' => {
                brackets = brackets.checked_sub(1)?;
            }
            '(' => parentheses += 1,
            ')' => {
                parentheses = parentheses.checked_sub(1)?;
            }
            ',' if brackets == 0 && parentheses == 0 => {
                selectors.push(crate::css::Selector::parse(&input[start..index])?);
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }

    if escaped || quote.is_some() || brackets != 0 || parentheses != 0 {
        return None;
    }
    selectors.push(crate::css::Selector::parse(&input[start..])?);
    Some(selectors)
}

/// HTML void elements cannot have serialized end tags or children.
fn is_void_html_element(name: &str) -> bool {
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

fn append_escaped_html(output: &mut String, value: &str, attribute: bool) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' if !attribute => output.push_str("&gt;"),
            '"' if attribute => output.push_str("&quot;"),
            _ => output.push(character),
        }
    }
}

/// Iterative serialization avoids consuming the Rust call stack for deeply
/// nested trees created through public DOM mutation methods.
fn serialize_html(root: &Node, output: &mut String) {
    enum Entry<'a> {
        Node(&'a Node, bool),
        Close(&'a str),
    }

    let mut stack = vec![Entry::Node(root, false)];
    while let Some(entry) = stack.pop() {
        match entry {
            Entry::Close(name) => {
                output.push_str("</");
                output.push_str(name);
                output.push('>');
            }
            Entry::Node(node, raw_text) => match &node.kind {
                NodeKind::Document => {
                    stack.extend(
                        node.children
                            .iter()
                            .rev()
                            .map(|child| Entry::Node(child, false)),
                    );
                }
                NodeKind::Text(value) => {
                    if raw_text {
                        output.push_str(value);
                    } else {
                        append_escaped_html(output, value, false);
                    }
                }
                NodeKind::Comment(value) => {
                    output.push_str("<!--");
                    output.push_str(value);
                    output.push_str("-->");
                }
                NodeKind::Element { name, attributes } => {
                    output.push('<');
                    output.push_str(name);
                    for (attribute, value) in attributes {
                        output.push(' ');
                        output.push_str(attribute);
                        output.push_str("=\"");
                        append_escaped_html(output, value, true);
                        output.push('"');
                    }
                    output.push('>');
                    if is_void_html_element(name) {
                        continue;
                    }
                    stack.push(Entry::Close(name));
                    let child_raw_text = matches!(name.as_str(), "script" | "style");
                    stack.extend(
                        node.children
                            .iter()
                            .rev()
                            .map(|child| Entry::Node(child, child_raw_text)),
                    );
                }
            },
        }
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

mod tests {
    use super::*;

    #[test]
    fn inner_html_setter_replaces_children_and_parses_markup() {
        let mut root = Node::element("div");
        root.append(Node::element("old"));
        assert!(root.set_inner_html("<p class='x'>hello &amp; <b>world</b></p><!--done-->"));
        assert_eq!(root.children.len(), 2);
        assert_eq!(
            root.query_selector("p.x").unwrap().text_content(),
            "hello & world"
        );
        assert_eq!(
            root.inner_html(),
            "<p class=\"x\">hello &amp; <b>world</b></p><!--done-->"
        );
    }

    #[test]
    fn append_html_preserves_existing_children_and_rejects_void_elements() {
        let mut root = Node::element("div");
        root.append(Node::text("before"));
        assert!(root.append_html("<span>after</span>"));
        assert_eq!(root.text_content(), "beforeafter");

        let mut image = Node::element("img");
        assert!(!image.append_html("<span>invalid</span>"));
        assert!(image.children.is_empty());
    }

    #[test]
    fn dom_collection_and_matches_apis_cover_wildcards_and_multiple_classes() {
        let mut root = Node::element("main");
        let mut first = Node::element("div");
        first.set_attribute("id", "first");
        first.set_attribute("class", "card featured");
        let mut second = Node::element("span");
        second.set_attribute("class", "card");
        root.append(first);
        root.append(second);

        assert_eq!(
            root.get_element_by_id("first").unwrap().tag_name(),
            Some("div")
        );
        assert_eq!(root.get_elements_by_tag_name("*").len(), 2);
        assert_eq!(root.get_elements_by_tag_name("DIV").len(), 1);
        assert_eq!(root.get_elements_by_class_name("card featured").len(), 1);
        assert_eq!(root.get_elements_by_class_name("featured card").len(), 1);
        assert!(root.get_elements_by_class_name("missing").is_empty());
        assert!(root
            .child(0)
            .unwrap()
            .matches_selector("div.card, span.active"));
        assert!(!root.child(0).unwrap().matches_selector("main > div"));
    }

    #[test]
    fn insert_before_and_after_preserve_order_and_reject_invalid_indices() {
        let mut root = Node::element("div");
        root.append(Node::element("a"));
        root.append(Node::element("c"));
        assert!(root.insert_before(1, Node::element("b")).is_ok());
        assert!(root.insert_after(1, Node::element("between")).is_ok());
        assert!(root.insert_before(99, Node::element("invalid")).is_err());
        assert_eq!(
            root.children
                .iter()
                .filter_map(Node::tag_name)
                .collect::<Vec<_>>(),
            ["a", "b", "between", "c"]
        );
    }

    #[test]
    fn prepend_html_inserts_before_existing_children() {
        let mut root = Node::element("div");
        root.append(Node::element("existing"));
        assert!(root.prepend_html("<span>first</span><span>second</span>"));
        assert_eq!(
            root.children
                .iter()
                .filter_map(Node::tag_name)
                .collect::<Vec<_>>(),
            ["span", "span", "existing"]
        );
        assert_eq!(root.text_content(), "firstsecond");
    }

    #[test]
    fn set_text_content_never_parses_markup() {
        let mut element = Node::element("div");
        element.append(Node::element("b"));
        element.set_text_content("<b>literal</b>");
        assert_eq!(element.children.len(), 1);
        assert!(element.children[0].is_text());
        assert_eq!(element.inner_html(), "&lt;b&gt;literal&lt;/b&gt;");

        let mut text = Node::text("old");
        text.set_text_content("new");
        assert_eq!(text.text_content(), "new");
    }

    #[test]
    fn contains_uses_node_identity_and_includes_the_receiver() {
        let mut root = Node::element("main");
        let mut section = Node::element("section");
        section.append(Node::text("inside"));
        root.append(section);
        let child = root.child(0).expect("section");
        let text = child.child(0).expect("text");
        let unrelated = Node::element("section");

        assert!(root.contains(&root));
        assert!(root.contains(child));
        assert!(root.contains(text));
        assert!(!child.contains(&root));
        assert!(!root.contains(&unrelated));
    }

    #[test]
    fn attribute_alias_and_count_follow_element_attributes() {
        let mut element = Node::element("div");
        assert_eq!(element.attribute_count(), 0);
        assert_eq!(element.get_attribute("id"), None);
        element.set_attribute("ID", "root");
        element.set_attribute("class", "main");
        assert_eq!(element.get_attribute("id"), Some("root"));
        assert_eq!(element.attribute_count(), 2);
        element.remove_attribute("class");
        assert_eq!(element.attribute_count(), 1);
        assert_eq!(Node::text("text").attribute_count(), 0);
    }

    #[test]
    fn node_metadata_matches_dom_node_types_and_names() {
        let document = Node::document();
        let element = Node::element("div");
        let text = Node::text("hello");
        let comment = Node::comment("note");

        assert_eq!(document.node_type(), 9);
        assert_eq!(document.node_name(), "#document");
        assert!(document.is_document());

        assert_eq!(element.node_type(), 1);
        assert_eq!(element.node_name(), "DIV");
        assert!(element.is_element());

        assert_eq!(text.node_type(), 3);
        assert_eq!(text.node_name(), "#text");
        assert!(text.is_text());

        assert_eq!(comment.node_type(), 8);
        assert_eq!(comment.node_name(), "#comment");
        assert!(comment.is_comment());
    }

    #[test]
    fn child_node_accessors_include_text_and_comments() {
        let mut parent = Node::element("div");
        assert!(!parent.has_child_nodes());
        assert!(parent.first_child().is_none());
        assert!(parent.last_child().is_none());

        parent.append(Node::text("before"));
        parent.append(Node::comment("marker"));
        parent.append(Node::element("span"));

        assert!(parent.has_child_nodes());
        assert_eq!(
            parent.first_child().and_then(|node| match &node.kind {
                NodeKind::Text(value) => Some(value.as_str()),
                _ => None,
            }),
            Some("before")
        );
        assert!(matches!(
            parent.last_child().map(|node| &node.kind),
            Some(NodeKind::Element { name, .. }) if name == "span"
        ));
    }

    #[test]
    fn attribute_introspection_reports_element_attributes_only() {
        let mut element = Node::element("div");
        assert!(!element.has_attributes());
        assert!(element.attribute_names().is_empty());

        element.set_attribute("id", "root");
        element.set_attribute("class", "main");
        assert!(element.has_attributes());
        assert_eq!(element.attribute_names(), vec!["class", "id"]);

        assert!(!Node::text("text").has_attributes());
        assert!(Node::text("text").attribute_names().is_empty());
    }

    #[test]
    fn replace_children_returns_previous_nodes() {
        let mut parent = Node::element("div");
        parent.append(Node::text("old"));
        parent.append(Node::element("span"));

        let previous = parent.replace_children(vec![Node::text("new")]);
        assert_eq!(previous.len(), 2);
        assert_eq!(parent.child_count(), 1);
        assert_eq!(parent.text_content(), "new");
    }

    #[test]
    fn normalize_merges_adjacent_text_and_preserves_comments() {
        let mut parent = Node::element("div");
        parent.append(Node::text("one"));
        parent.append(Node::text(""));
        parent.append(Node::text(" two"));
        parent.append(Node::comment("keep"));
        parent.append(Node::text("three"));

        parent.normalize();

        assert_eq!(parent.children.len(), 3);
        assert_eq!(parent.children[0].text_content(), "one two");
        assert!(matches!(parent.children[1].kind, NodeKind::Comment(_)));
        assert_eq!(parent.children[2].text_content(), "three");
    }

    #[test]
    fn normalize_merges_text_recursively_inside_elements() {
        let mut parent = Node::element("div");
        let mut child = Node::element("span");
        child.append(Node::text("a"));
        child.append(Node::text("b"));
        parent.append(child);

        parent.normalize();

        let span = parent.first_element_child().expect("span");
        assert_eq!(span.children.len(), 1);
        assert_eq!(span.text_content(), "ab");
    }

    #[test]
    fn element_child_accessors_ignore_text_and_comments() {
        let mut parent = Node::element("div");
        parent.append(Node::text("before"));
        parent.append(Node::element("span"));
        parent.append(Node::comment("between"));
        parent.append(Node::element("strong"));
        parent.append(Node::text("after"));

        assert_eq!(parent.child_element_count(), 2);
        assert_eq!(
            parent.first_element_child().and_then(Node::tag_name),
            Some("span")
        );
        assert_eq!(
            parent.last_element_child().and_then(Node::tag_name),
            Some("strong")
        );
    }

    #[test]
    fn class_list_operations_preserve_tokens_and_avoid_duplicates() {
        let mut node = Node::element("div");
        node.set_attribute("class", "alpha  beta\tgamma");
        assert_eq!(node.class_list(), vec!["alpha", "beta", "gamma"]);
        assert!(node.has_class("beta"));
        node.add_class("beta");
        node.add_class("delta");
        assert_eq!(node.attribute("class"), Some("alpha beta gamma delta"));
        node.remove_class("beta");
        assert_eq!(node.attribute("class"), Some("alpha gamma delta"));
        assert!(!node.toggle_class("gamma", None));
        assert!(!node.has_class("gamma"));
        assert!(node.toggle_class("omega", Some(true)));
        assert!(node.has_class("omega"));
        node.toggle_class("omega", Some(false));
        assert!(!node.has_class("omega"));
    }

    #[test]
    fn class_list_rejects_invalid_tokens_and_removes_empty_class_attribute() {
        let mut node = Node::element("div");
        node.add_class("");
        node.add_class("two words");
        assert!(!node.has_attribute("class"));
        node.set_attribute("class", "one");
        node.remove_class("one");
        assert!(!node.has_attribute("class"));
    }

    #[test]
    fn boolean_attribute_toggle_supports_force_and_default_modes() {
        let mut node = Node::element("input");
        assert!(node.toggle_attribute("required", None));
        assert!(node.has_attribute("required"));
        assert!(node.toggle_attribute("required", Some(true)));
        assert!(!node.toggle_attribute("required", Some(false)));
        assert!(!node.has_attribute("required"));
    }

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
    fn visible_text_omits_metadata_scripts_styles_and_hidden_subtrees() {
        let root = crate::html::parser::parse(&crate::html::tokenizer::HtmlTokenizer::tokenize(
            "<head><title>Page title</title><style>body { color: red }</style></head><body>before<script>alert(1)</script><span hidden>secret</span><span aria-hidden='true'>also secret</span><p>visible</p><template>template text</template><noscript>fallback</noscript>after</body>",
        ));

        assert_eq!(root.text_content(), "Page titlebody { color: red }beforealert(1)secretalso secretvisibletemplate textfallbackafter");
        assert_eq!(root.visible_text_content(), "beforevisibleafter");
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
    fn combined_tag_attribute_and_class_queries_filter_precisely() {
        let mut root = Node::document();
        let mut first = Node::element("input");
        first.set_attribute("type", "email");
        first.set_attribute("class", "field compact");
        let mut second = Node::element("input");
        second.set_attribute("type", "text");
        second.set_attribute("class", "field");
        let mut third = Node::element("textarea");
        third.set_attribute("type", "email");
        third.set_attribute("class", "field compact");
        root.append(first);
        root.append(second);
        root.append(third);

        let email_inputs = root.find_by_tag_and_attribute_value("INPUT", "TYPE", "email");
        assert_eq!(email_inputs.len(), 1);
        assert_eq!(email_inputs[0].attribute("type"), Some("email"));

        let compact_inputs = root.find_by_tag_and_class("input", "compact");
        assert_eq!(compact_inputs.len(), 1);
        assert_eq!(compact_inputs[0].attribute("type"), Some("email"));

        assert!(root
            .find_by_tag_and_class("input", "field compact")
            .is_empty());
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
            root.find_first_by_class("required")
                .and_then(Node::tag_name),
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
    fn query_selector_supports_descendant_child_and_sibling_combinators() {
        let mut root = Node::element("main");
        let mut section = Node::element("section");
        section.set_attribute("class", "cards");
        let mut first = Node::element("article");
        first.set_attribute("class", "card");
        let mut second = Node::element("article");
        second.set_attribute("class", "card featured");
        let mut nested = Node::element("span");
        nested.set_attribute("class", "label");
        second.append(nested);
        section.append(first);
        section.append(second);
        root.append(section);

        assert_eq!(
            root.query_selector("main > .cards > .card + .featured")
                .and_then(Node::tag_name),
            Some("article")
        );
        assert_eq!(root.query_selector_all(".cards .label").len(), 1);
        assert_eq!(root.query_selector_all(".card").len(), 2);
        assert!(root.query_selector("article + article .missing").is_none());
    }

    #[test]
    fn query_selector_supports_selector_lists_and_nested_commas() {
        let mut root = Node::element("main");
        let mut first = Node::element("p");
        first.set_attribute("class", "first");
        let mut second = Node::element("p");
        second.set_attribute("data-label", "a,b");
        second.set_attribute("class", "second");
        let mut third = Node::element("p");
        third.set_attribute("class", "third");
        root.append(first);
        root.append(second);
        root.append(third);

        assert_eq!(root.query_selector_all(".first, .third").len(), 2);
        assert_eq!(
            root.query_selector_all("[data-label='a,b'], .third").len(),
            2
        );
        assert_eq!(
            root.query_selector(".missing, .second")
                .and_then(Node::tag_name),
            Some("p")
        );
        assert!(root.query_selector_all(".first, > .broken").is_empty());
    }

    #[test]
    fn query_selector_excludes_receiver_and_rejects_invalid_selectors() {
        let mut root = Node::element("div");
        root.set_attribute("id", "root");
        root.append(Node::element("span"));

        assert!(root.query_selector("#root").is_none());
        assert_eq!(
            root.query_selector("span").and_then(Node::tag_name),
            Some("span")
        );
        assert!(root.query_selector("div > > span").is_none());
    }

    #[test]
    fn inner_html_escapes_text_and_attributes_and_serializes_comments() {
        let mut root = Node::element("div");
        root.set_attribute("title", "a & \"b\"");
        root.append(Node::text("<hello & goodbye>"));
        root.append(Node::comment("note"));
        root.append(Node::element("br"));

        assert_eq!(
            root.inner_html(),
            "&lt;hello &amp; goodbye&gt;<!--note--><br>"
        );
        assert_eq!(
            root.outer_html(),
            "<div title=\"a &amp; &quot;b&quot;\">&lt;hello &amp; goodbye&gt;<!--note--><br></div>"
        );
    }

    #[test]
    fn html_serialization_handles_deep_dom_trees_iteratively() {
        let mut root = Node::element("div");
        let mut current = &mut root;
        for _ in 0..2_000 {
            current.children.push(Node::element("span"));
            current = current.children.last_mut().unwrap();
        }
        current.append(Node::text("end"));

        let html = root.outer_html();
        assert!(html.starts_with("<div><span><span>"));
        assert!(html.contains("end</span>"));
        assert!(
            html.ends_with(&format!("{} </div>", "</span>".repeat(2_000)).replace(" </div>", "</div>")),
            "serialized suffix: {:?}",
            &html[html.len().saturating_sub(96)..]
        );
        assert_eq!(html.matches("<span>").count(), 2_000);
    }

    #[test]
    fn html_serialization_preserves_raw_text_and_void_elements() {
        let mut root = Node::document();
        let mut script = Node::element("script");
        script.append(Node::text("if (a < b) x = '&';"));
        root.append(script);
        let mut image = Node::element("img");
        image.set_attribute("alt", "sample");
        root.append(image);

        assert_eq!(
            root.outer_html(),
            "<script>if (a < b) x = '&';</script><img alt=\"sample\">"
        );
    }

    #[test]
    fn id_and_class_name_properties_round_trip() {
        let mut element = Node::element("div");
        assert_eq!(element.id(), "");
        assert_eq!(element.class_name(), "");
        element.set_id("main");
        element.set_class_name("card active");
        assert_eq!(element.id(), "main");
        assert_eq!(element.class_name(), "card active");
    }

    #[test]
    fn dataset_properties_map_camel_case_to_data_attributes() {
        let mut element = Node::element("div");
        assert!(element.set_dataset_value("userId", "42"));
        assert!(element.set_dataset_value("longName", "Ada"));
        assert_eq!(element.attribute("data-user-id"), Some("42"));
        assert_eq!(element.dataset_value("userId"), Some("42"));
        assert_eq!(element.dataset_value("longName"), Some("Ada"));
        assert_eq!(
            element.remove_dataset_value("userId").as_deref(),
            Some("42")
        );
        assert_eq!(element.dataset_value("userId"), None);
    }

    #[test]
    fn dataset_rejects_empty_or_whitespace_keys() {
        let mut element = Node::element("div");
        assert!(!element.set_dataset_value("", "x"));
        assert!(!element.set_dataset_value("bad key", "x"));
        assert_eq!(element.remove_dataset_value("bad key"), None);
    }

    #[test]
    fn form_control_helpers_expose_authored_default_state() {
        let mut checkbox = Node::element("input");
        checkbox.set_attribute("type", "checkbox");
        checkbox.set_attribute("checked", "");
        checkbox.set_attribute("required", "");
        assert!(checkbox.is_checked());
        assert!(checkbox.is_required());
        assert!(!checkbox.is_disabled());

        checkbox.set_attribute("disabled", "");
        assert!(checkbox.is_disabled());

        let mut text_input = Node::element("input");
        text_input.set_attribute("type", "text");
        text_input.set_attribute("checked", "");
        assert!(!text_input.is_checked());
        text_input.set_attribute("value", "hello");
        assert_eq!(text_input.form_value().as_deref(), Some("hello"));
    }

    #[test]
    fn form_value_handles_textarea_and_option_fallbacks() {
        let mut textarea = Node::element("textarea");
        textarea.append(Node::text("message"));
        assert_eq!(textarea.form_value().as_deref(), Some("message"));

        let mut option = Node::element("option");
        option.append(Node::text("Choice"));
        assert_eq!(option.form_value().as_deref(), Some("Choice"));
        option.set_attribute("value", "choice-id");
        assert_eq!(option.form_value().as_deref(), Some("choice-id"));
        option.set_attribute("selected", "");
        assert!(option.is_selected());
    }

    #[test]
    fn element_child_accessors_skip_text_and_comments() {
        let mut root = Node::element("div");
        root.append(Node::text("before"));
        root.append(Node::comment("note"));
        root.append(Node::element("span"));
        root.append(Node::text("between"));
        root.append(Node::element("strong"));
        root.append(Node::comment("after"));

        assert_eq!(root.child_count(), 6);
        assert_eq!(root.child_element_count(), 2);
        assert_eq!(
            root.first_element_child().and_then(Node::tag_name),
            Some("span")
        );
        assert_eq!(
            root.last_element_child().and_then(Node::tag_name),
            Some("strong")
        );
        assert_eq!(root.element_children().len(), 2);
        assert_eq!(root.element_children()[0].tag_name(), Some("span"));
        assert_eq!(root.element_children()[1].tag_name(), Some("strong"));
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
