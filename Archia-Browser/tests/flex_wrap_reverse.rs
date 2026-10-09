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

    assert_eq!(layout.children[0].rect.y, 90);
    assert_eq!(layout.children[1].rect.y, 90);
    assert_eq!(layout.children[2].rect.y, 80);
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

    assert_eq!(layout.children[0].rect.x, 90);
    assert_eq!(layout.children[1].rect.x, 90);
    assert_eq!(layout.children[2].rect.x, 80);
}

#[test]
fn flex_align_self_overrides_parent_flex_start() {
    let mut root = Node::element("div");
    root.set_attribute(
        "style",
        "display: flex; align-items: flex-start; width: 100px; height: 100px;",
    );
    let mut child = Node::element("div");
    child.set_attribute("style", "width: 20px; height: 20px; align-self: center;");
    root.append(child);

    let layout = styled_layout(&root, 100, 100);

    assert_eq!(layout.children[0].rect.y, 40);
}

#[test]
fn block_margins_do_not_collapse_across_inline_content() {
    let mut root = Node::element("div");
    let mut first = Node::element("div");
    first.set_attribute("style", "height: 10px; margin-bottom: 20px;");
    let mut inline = Node::element("span");
    inline.append(Node::text("x"));
    let mut second = Node::element("div");
    second.set_attribute("style", "height: 10px; margin-top: 30px;");
    root.append(first);
    root.append(inline);
    root.append(second);

    let layout = styled_layout(&root, 100, 100);

    assert_eq!(layout.children[2].rect.y, 76);
}

#[test]
fn flex_align_items_stretches_auto_height_row_children() {
    let mut root = Node::element("div");
    root.set_attribute(
        "style",
        "display: flex; align-items: stretch; width: 100px; height: 100px;",
    );
    let mut child = Node::element("div");
    child.set_attribute("style", "width: 20px;");
    root.append(child);

    let layout = styled_layout(&root, 100, 100);

    assert_eq!(layout.children[0].rect.height, 100);
}

#[test]
fn minimum_dimensions_win_when_they_exceed_maximum_dimensions() {
    let mut root = Node::element("div");
    let mut child = Node::element("div");
    child.set_attribute(
        "style",
        "width: 10px; min-width: 80px; max-width: 40px; height: 10px; min-height: 80px; max-height: 40px;",
    );
    root.append(child);

    let layout = styled_layout(&root, 100, 100);

    assert_eq!(layout.children[0].rect.width, 80);
    assert_eq!(layout.children[0].rect.height, 80);
}

#[test]
fn flex_stretch_preserves_min_height_when_it_exceeds_max_height() {
    let mut root = Node::element("div");
    root.set_attribute(
        "style",
        "display: flex; align-items: stretch; width: 100px; height: 100px;",
    );
    let mut child = Node::element("div");
    child.set_attribute("style", "min-height: 80px; max-height: 40px;");
    root.append(child);

    let layout = styled_layout(&root, 100, 100);

    assert_eq!(layout.children[0].rect.height, 80);
}
