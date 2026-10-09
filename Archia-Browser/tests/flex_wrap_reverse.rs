use archia_browser::css::StyleSheet;
use archia_browser::html::Node;
use archia_browser::layout::{LayoutEngine, LayoutViewport};
use archia_browser::style_tree::StyleEngine;

fn styled_layout(root: &Node, width: u32, height: u32) -> archia_browser::layout::LayoutNode {
    let styled = StyleEngine::style(root, &StyleSheet::default());
    LayoutEngine::layout_styled(&styled, LayoutViewport::new(width, height))
}

#[test]
fn flex_row_wrap_reverse_places_first_line_at_cross_axis_end() {
    let mut root = Node::element("div");
    root.set_attribute(
        "style",
        "display: flex; flex-wrap: wrap-reverse; align-content: flex-start; width: 40px; height: 100px;",
    );
    for _ in 0..3 {
        let mut child = Node::element("div");
        child.set_attribute("style", "width: 20px; height: 10px; flex: 0 0 20px;");
        root.append(child);
    }

    let layout = styled_layout(&root, 40, 100);

    assert_eq!(layout.children[0].rect.y, 80);
    assert_eq!(layout.children[1].rect.y, 80);
    assert_eq!(layout.children[2].rect.y, 70);
}

#[test]
fn flex_column_wrap_reverse_places_first_column_at_cross_axis_end() {
    let mut root = Node::element("div");
    root.set_attribute(
        "style",
        "display: flex; flex-flow: column wrap-reverse; align-content: flex-start; width: 100px; height: 40px;",
    );
    for _ in 0..3 {
        let mut child = Node::element("div");
        child.set_attribute("style", "width: 10px; height: 20px; flex: 0 0 20px;");
        root.append(child);
    }

    let layout = styled_layout(&root, 100, 40);

    assert_eq!(layout.children[0].rect.x, 80);
    assert_eq!(layout.children[1].rect.x, 80);
    assert_eq!(layout.children[2].rect.x, 70);
}
