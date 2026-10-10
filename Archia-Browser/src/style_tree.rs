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
        Self {
            node,
            style,
            children: Vec::new(),
        }
    }

    pub fn text_content(&self) -> String {
        self.node.text_content()
    }
}

pub struct StyleEngine;

impl StyleEngine {
    pub fn style(document: &Node, sheet: &StyleSheet) -> StyledNode {
        let mut path = Vec::new();
        let mut sibling_lists = vec![&[] as &[Node]];
        let mut sibling_positions = Vec::new();
        style_node(
            document,
            sheet,
            &mut path,
            &mut sibling_lists,
            &mut sibling_positions,
            0,
            &ComputedStyle::default(),
        )
    }
}

fn style_node<'a>(
    node: &'a Node,
    sheet: &StyleSheet,
    path: &mut Vec<&'a Node>,
    sibling_lists: &mut Vec<&'a [Node]>,
    sibling_positions: &mut Vec<usize>,
    position: usize,
    inherited: &ComputedStyle,
) -> StyledNode {
    path.push(node);
    sibling_positions.push(position);
    let style =
        sheet.compute_style_node_with_siblings(path, sibling_lists, sibling_positions, inherited);
    let children_slice = node.children.as_slice();
    let mut children = Vec::with_capacity(node.children.len());
    for (child_position, child) in node.children.iter().enumerate() {
        sibling_lists.push(children_slice);
        children.push(style_node(
            child,
            sheet,
            path,
            sibling_lists,
            sibling_positions,
            child_position,
            &style,
        ));
        sibling_lists.pop();
    }
    sibling_positions.pop();
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
    fn sibling_selectors_style_the_correct_dom_nodes() {
        let mut body = Node::element("body");
        let first = Node::element("span");
        let mut second = Node::element("p");
        second.set_attribute("class", "target");
        let third = Node::element("p");
        body.append(first);
        body.append(second);
        body.append(third);

        let sheet = StyleSheet::parse("span + p { color: red; } span ~ p.target { padding: 4px; }");
        let styled = StyleEngine::style(&body, &sheet);

        assert_eq!(styled.children[1].style.get("color"), Some("red"));
        assert_eq!(styled.children[1].style.get("padding"), Some("4px"));
        assert_eq!(styled.children[2].style.get("color"), None);
        assert_eq!(styled.children[2].style.get("padding"), None);
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
