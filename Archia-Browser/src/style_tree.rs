use crate::css::{ComputedStyle, StyleSheet};
use crate::html::Node;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledNode {
    pub node: Node,
    pub style: ComputedStyle,
    pub children: Vec<StyledNode>,
}

impl StyledNode {
    pub fn new(node: Node, style: ComputedStyle) -> Self {
        Self { node, style, children: Vec::new() }
    }

    pub fn text_content(&self) -> String {
        self.node.text_content()
    }
}

pub struct StyleEngine;

impl StyleEngine {
    pub fn style(document: &Node, sheet: &StyleSheet) -> StyledNode {
        let mut path = Vec::new();
        style_node(document, sheet, &mut path)
    }
}

fn style_node<'a>(
    node: &'a Node,
    sheet: &StyleSheet,
    path: &mut Vec<&'a Node>,
) -> StyledNode {
    path.push(node);
    let style = sheet.compute_style_path(path);
    let children = node
        .children
        .iter()
        .map(|child| style_node(child, sheet, path))
        .collect();
    path.pop();
    StyledNode {
        node: node.clone(),
        style,
        children,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::Node;

    #[test]
    fn computes_styles_per_dom_node() {
        let mut body = Node::element("body");
        let mut first = Node::element("div");
        first.set_attribute("class", "hot");
        first.append(Node::text("one"));
        let mut second = Node::element("div");
        second.append(Node::text("two"));
        body.append(first);
        body.append(second);

        let sheet = StyleSheet::parse(".hot { color: red; } div { padding: 4px; }");
        let styled = StyleEngine::style(&body, &sheet);

        assert_eq!(styled.children[0].style.get("color"), Some("red"));
        assert_eq!(styled.children[1].style.get("color"), None);
        assert_eq!(styled.children[0].style.get("padding"), Some("4px"));
        assert_eq!(styled.children[1].style.get("padding"), Some("4px"));
    }

    #[test]
    fn inline_style_is_kept_on_the_matching_node() {
        let mut root = Node::element("div");
        root.set_attribute("style", "color: blue;");
        root.append(Node::text("hello"));

        let styled = StyleEngine::style(&root, &StyleSheet::default());
        assert_eq!(styled.style.get("color"), Some("blue"));
        assert_eq!(styled.children[0].style.get("color"), Some("blue"));
    }
}
