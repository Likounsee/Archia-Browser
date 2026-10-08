use crate::css::ComputedStyle;
use crate::html::{Node, NodeKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Block,
    Inline,
    None,
}

impl Display {
    pub fn from_style(style: &ComputedStyle) -> Self {
        match style.get("display").map(str::trim) {
            Some("none") => Self::None,
            Some("inline") | Some("inline-block") => Self::Inline,
            _ => Self::Block,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BoxModel {
    pub margin_top: u32,
    pub margin_right: u32,
    pub margin_bottom: u32,
    pub margin_left: u32,
    pub padding_top: u32,
    pub padding_right: u32,
    pub padding_bottom: u32,
    pub padding_left: u32,
    pub border_top: u32,
    pub border_right: u32,
    pub border_bottom: u32,
    pub border_left: u32,
}

impl BoxModel {
    pub fn horizontal_outer(&self) -> u32 {
        self.margin_left
            .saturating_add(self.margin_right)
            .saturating_add(self.padding_left)
            .saturating_add(self.padding_right)
            .saturating_add(self.border_left)
            .saturating_add(self.border_right)
    }

    pub fn vertical_outer(&self) -> u32 {
        self.margin_top
            .saturating_add(self.margin_bottom)
            .saturating_add(self.padding_top)
            .saturating_add(self.padding_bottom)
            .saturating_add(self.border_top)
            .saturating_add(self.border_bottom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutNode {
    pub display: Display,
    pub rect: Rect,
    pub box_model: BoxModel,
    pub children: Vec<LayoutNode>,
}

impl LayoutNode {
    pub fn new(display: Display) -> Self {
        Self {
            display,
            rect: Rect::default(),
            box_model: BoxModel::default(),
            children: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutViewport {
    pub width: u32,
    pub height: u32,
}

impl LayoutViewport {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

pub struct LayoutEngine;

impl LayoutEngine {
    pub fn layout_styled(
        root: &crate::style_tree::StyledNode,
        viewport: LayoutViewport,
    ) -> LayoutNode {
        layout_styled_node(root, 0, 0, viewport.width, viewport.height)
    }

    pub fn layout(root: &Node, viewport: LayoutViewport, style: &ComputedStyle) -> LayoutNode {
        let display = Display::from_style(style);
        let mut output = LayoutNode::new(display);
        if display == Display::None {
            return output;
        }

        output.rect.width = viewport.width;
        output.rect.height = intrinsic_height(root);
        layout_children(root, &mut output, viewport.width);
        output
    }
}

fn layout_styled_node(
    node: &crate::style_tree::StyledNode,
    x: i32,
    y: i32,
    containing_width: u32,
    viewport_height: u32,
) -> LayoutNode {
    let display = display_for_styled_node(node);
    let mut output = LayoutNode::new(display);
    if display == Display::None {
        return output;
    }

    let mut box_model = box_model_from_style(&node.style);
    let margin_x = box_model.margin_left.saturating_add(box_model.margin_right);
    let padding_border_x = box_model
        .padding_left
        .saturating_add(box_model.padding_right)
        .saturating_add(box_model.border_left)
        .saturating_add(box_model.border_right);
    let specified_width = parse_length(node.style.get("width"), containing_width);
    let border_box = node
        .style
        .get("box-sizing")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("border-box"));
    let mut content_width = specified_width.map_or_else(
        || {
            if display == Display::Inline {
                intrinsic_inline_content_width(node)
            } else {
                containing_width
                    .saturating_sub(margin_x)
                    .saturating_sub(padding_border_x)
            }
        },
        |width| {
            if border_box {
                width.saturating_sub(padding_border_x)
            } else {
                width
            }
        },
    );

    if let Some(min_width) = parse_length(node.style.get("min-width"), containing_width) {
        content_width = content_width.max(min_width);
    }
    if let Some(max_width) = parse_length(node.style.get("max-width"), containing_width) {
        content_width = content_width.min(max_width);
    }

    if display == Display::Block && specified_width.is_some() {
        let left_auto = is_auto_dimension(node.style.get("margin-left"));
        let right_auto = is_auto_dimension(node.style.get("margin-right"));
        if left_auto || right_auto {
            let fixed_outer = content_width
                .saturating_add(padding_border_x)
                .saturating_add(if left_auto { 0 } else { box_model.margin_left })
                .saturating_add(if right_auto {
                    0
                } else {
                    box_model.margin_right
                });
            let free_space = containing_width.saturating_sub(fixed_outer);
            match (left_auto, right_auto) {
                (true, true) => {
                    box_model.margin_left = free_space / 2;
                    box_model.margin_right = free_space.saturating_sub(box_model.margin_left);
                }
                (true, false) => box_model.margin_left = free_space,
                (false, true) => box_model.margin_right = free_space,
                (false, false) => {}
            }
        }
    }

    output.box_model = box_model;
    output.rect.x = x.saturating_add(box_model.margin_left as i32);
    output.rect.y = y.saturating_add(box_model.margin_top as i32);
    output.rect.width = content_width;

    let explicit_height = parse_px(node.style.get("height"));
    let content_origin_x = output
        .rect
        .x
        .saturating_add(box_model.border_left as i32)
        .saturating_add(box_model.padding_left as i32);
    let content_origin_y = output
        .rect
        .y
        .saturating_add(box_model.border_top as i32)
        .saturating_add(box_model.padding_top as i32);
    let mut cursor_y = 0_i32;
    let mut inline_x = 0_u32;
    let mut inline_line_height = 0_u32;
    let mut previous_block_margin_bottom = 0_u32;

    for child in &node.children {
        let child_display = display_for_styled_node(child);
        if child_display == Display::None {
            output.children.push(LayoutNode::new(Display::None));
            continue;
        }

        if child_display == Display::Inline {
            let width = intrinsic_inline_width(child);
            let line_height = used_inline_line_height(child);
            if inline_x > 0 && inline_x.saturating_add(width) > content_width {
                cursor_y = cursor_y.saturating_add(inline_line_height as i32);
                inline_x = 0;
                inline_line_height = 0;
            }
            let child_layout = layout_styled_node(
                child,
                content_origin_x.saturating_add(inline_x as i32),
                content_origin_y.saturating_add(cursor_y),
                content_width.saturating_sub(inline_x),
                viewport_height,
            );
            inline_x = inline_x.saturating_add(
                child_layout
                    .rect
                    .width
                    .saturating_add(child_layout.box_model.horizontal_outer()),
            );
            inline_line_height = inline_line_height.max(line_height);
            output.children.push(child_layout);
        } else {
            if inline_x > 0 {
                cursor_y = cursor_y.saturating_add(inline_line_height as i32);
                inline_x = 0;
                inline_line_height = 0;
            }
            let child_margin_top = box_model_from_style(&child.style).margin_top;
            let collapsed_margin = previous_block_margin_bottom.max(child_margin_top);
            cursor_y = cursor_y.saturating_add(collapsed_margin as i32);

            let child_layout = layout_styled_node(
                child,
                content_origin_x,
                content_origin_y.saturating_add(cursor_y),
                content_width,
                viewport_height,
            );
            let child_box = child_layout.box_model;
            cursor_y = cursor_y.saturating_add(child_layout.rect.height as i32);
            previous_block_margin_bottom = child_box.margin_bottom;
            output.children.push(child_layout);
        }
    }

    if inline_x > 0 {
        cursor_y = cursor_y.saturating_add(inline_line_height as i32);
    }

    let mut content_height = explicit_height.unwrap_or(cursor_y.max(0) as u32);
    if let Some(min_height) = parse_length(node.style.get("min-height"), viewport_height) {
        content_height = content_height.max(min_height);
    }
    if let Some(max_height) = parse_length(node.style.get("max-height"), viewport_height) {
        content_height = content_height.min(max_height);
    }

    output.rect.height = if border_box && explicit_height.is_some() {
        content_height
    } else {
        content_height
            .saturating_add(box_model.padding_top)
            .saturating_add(box_model.padding_bottom)
            .saturating_add(box_model.border_top)
            .saturating_add(box_model.border_bottom)
    };
    output
}

fn display_for_styled_node(node: &crate::style_tree::StyledNode) -> Display {
    if node.style.get("display").is_some() {
        return Display::from_style(&node.style);
    }
    match &node.node.kind {
        NodeKind::Text(_) => Display::Inline,
        NodeKind::Element { name, .. } => match name.as_str() {
            "head" | "title" | "meta" | "link" | "base" | "script" | "style" | "template" => {
                Display::None
            }
            "a" | "abbr" | "b" | "bdi" | "bdo" | "br" | "button" | "code" | "em" | "i" | "img"
            | "input" | "label" | "small" | "span" | "strong" | "sub" | "sup" | "textarea"
            | "time" | "u" => Display::Inline,
            _ => Display::Block,
        },
        NodeKind::Document => Display::Block,
        NodeKind::Comment(_) => Display::None,
    }
}

fn intrinsic_inline_content_width(node: &crate::style_tree::StyledNode) -> u32 {
    match &node.node.kind {
        NodeKind::Text(text) => text.chars().count().min(u32::MAX as usize) as u32 * 6,
        _ => node
            .children
            .iter()
            .map(intrinsic_inline_content_width)
            .fold(0, u32::saturating_add)
            .max(parse_px(node.style.get("width")).unwrap_or(0)),
    }
}

fn intrinsic_inline_width(node: &crate::style_tree::StyledNode) -> u32 {
    match &node.node.kind {
        NodeKind::Text(text) => text.chars().count().min(u32::MAX as usize) as u32 * 6,
        _ => {
            let children_width = node
                .children
                .iter()
                .map(intrinsic_inline_width)
                .fold(0, u32::saturating_add);
            let content_width = children_width.max(parse_px(node.style.get("width")).unwrap_or(0));
            content_width.saturating_add(box_model_from_style(&node.style).horizontal_outer())
        }
    }
}

fn used_inline_line_height(node: &crate::style_tree::StyledNode) -> u32 {
    node.style
        .get("line-height")
        .and_then(|value| parse_px(Some(value)))
        .or_else(|| Some(intrinsic_inline_height(node)))
        .unwrap_or(16)
        .max(1)
}

fn intrinsic_inline_height(node: &crate::style_tree::StyledNode) -> u32 {
    match &node.node.kind {
        NodeKind::Text(text) => text.split('\n').count().max(1) as u32 * 16,
        _ => node
            .children
            .iter()
            .map(intrinsic_inline_height)
            .max()
            .unwrap_or(16),
    }
}

fn box_model_from_style(style: &ComputedStyle) -> BoxModel {
    BoxModel {
        margin_top: parse_px(style.get("margin-top"))
            .unwrap_or_else(|| parse_px(style.get("margin")).unwrap_or(0)),
        margin_right: parse_px(style.get("margin-right"))
            .unwrap_or_else(|| parse_px(style.get("margin")).unwrap_or(0)),
        margin_bottom: parse_px(style.get("margin-bottom"))
            .unwrap_or_else(|| parse_px(style.get("margin")).unwrap_or(0)),
        margin_left: parse_px(style.get("margin-left"))
            .unwrap_or_else(|| parse_px(style.get("margin")).unwrap_or(0)),
        padding_top: parse_px(style.get("padding-top"))
            .unwrap_or_else(|| parse_px(style.get("padding")).unwrap_or(0)),
        padding_right: parse_px(style.get("padding-right"))
            .unwrap_or_else(|| parse_px(style.get("padding")).unwrap_or(0)),
        padding_bottom: parse_px(style.get("padding-bottom"))
            .unwrap_or_else(|| parse_px(style.get("padding")).unwrap_or(0)),
        padding_left: parse_px(style.get("padding-left"))
            .unwrap_or_else(|| parse_px(style.get("padding")).unwrap_or(0)),
        border_top: parse_border_width(style.get("border-top-width"))
            .unwrap_or_else(|| parse_border_width(style.get("border-width")).unwrap_or(0)),
        border_right: parse_border_width(style.get("border-right-width"))
            .unwrap_or_else(|| parse_border_width(style.get("border-width")).unwrap_or(0)),
        border_bottom: parse_border_width(style.get("border-bottom-width"))
            .unwrap_or_else(|| parse_border_width(style.get("border-width")).unwrap_or(0)),
        border_left: parse_border_width(style.get("border-left-width"))
            .unwrap_or_else(|| parse_border_width(style.get("border-width")).unwrap_or(0)),
    }
}

fn is_auto_dimension(value: Option<&str>) -> bool {
    value.is_some_and(|value| value.trim().eq_ignore_ascii_case("auto"))
}

fn parse_border_width(value: Option<&str>) -> Option<u32> {
    let value = value?.trim();
    match value.to_ascii_lowercase().as_str() {
        "thin" => Some(1),
        "medium" => Some(3),
        "thick" => Some(5),
        _ => parse_px(Some(value)),
    }
}

fn parse_length(value: Option<&str>, containing_width: u32) -> Option<u32> {
    let value = value?.trim();
    if let Some(percent) = value.strip_suffix('%') {
        let percent = percent.trim().parse::<u32>().ok()?;
        return Some(
            containing_width
                .saturating_mul(percent)
                .checked_div(100)
                .unwrap_or(0),
        );
    }
    parse_px(Some(value))
}

fn parse_px(value: Option<&str>) -> Option<u32> {
    let value = value?.trim();
    value.strip_suffix("px")?.trim().parse().ok()
}

fn layout_children(node: &Node, output: &mut LayoutNode, containing_width: u32) {
    let mut cursor_y = 0_i32;

    for child in &node.children {
        let child_display = if matches!(child.kind, NodeKind::Text(_)) {
            Display::Inline
        } else {
            Display::Block
        };
        let height = intrinsic_height(child);
        let mut child_layout = LayoutNode::new(child_display);
        child_layout.rect = Rect::new(0, cursor_y, containing_width, height);
        layout_children(child, &mut child_layout, containing_width);
        if child_display != Display::None {
            cursor_y = cursor_y.saturating_add(height as i32);
        }
        output.children.push(child_layout);
    }

    if !output.children.is_empty() {
        output.rect.height = cursor_y.max(output.rect.height as i32) as u32;
    }
}

fn intrinsic_height(node: &Node) -> u32 {
    match &node.kind {
        NodeKind::Text(text) => {
            let lines = text.split('\n').count().max(1);
            (lines as u32).saturating_mul(16)
        }
        _ => {
            let child_height = node
                .children
                .iter()
                .map(intrinsic_height)
                .fold(0_u32, u32::saturating_add);
            child_height.saturating_add(16)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_defaults_to_block() {
        let style = ComputedStyle::default();
        assert_eq!(Display::from_style(&style), Display::Block);
    }

    #[test]
    fn display_none_is_removed_from_layout_flow() {
        let mut style = ComputedStyle::default();
        style.set("display", "none");

        let node = Node::element("div");
        let layout = LayoutEngine::layout(&node, LayoutViewport::new(800, 600), &style);
        assert_eq!(layout.display, Display::None);
        assert!(layout.children.is_empty());
    }

    #[test]
    fn block_children_stack_vertically() {
        let mut root = Node::element("body");
        let mut first = Node::element("div");
        first.append(Node::text("first"));
        let mut second = Node::element("div");
        second.append(Node::text("second"));
        root.append(first);
        root.append(second);

        let layout = LayoutEngine::layout(
            &root,
            LayoutViewport::new(800, 600),
            &ComputedStyle::default(),
        );

        assert_eq!(layout.children.len(), 2);
        assert_eq!(layout.children[0].rect.y, 0);
        assert_eq!(layout.children[1].rect.y, 32);
        assert_eq!(layout.children[0].rect.width, 800);
    }

    #[test]
    fn inline_box_outer_width_includes_padding_and_border() {
        let mut root = Node::element("body");
        let mut child = Node::element("span");
        child.set_attribute(
            "style",
            "display: inline; padding: 2px; border-width: 1px; border-style: solid;",
        );
        child.append(Node::text("Hi"));
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(100, 100));

        assert_eq!(layout.children[0].rect.width, 12);
        assert_eq!(layout.children[0].box_model.horizontal_outer(), 6);
    }

    #[test]
    fn inline_block_participates_in_inline_flow() {
        let mut root = Node::element("body");
        let mut child = Node::element("div");
        child.set_attribute("style", "display: inline-block; width: 40px;");
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(100, 100));

        assert_eq!(layout.children[0].display, Display::Inline);
        assert_eq!(layout.children[0].rect.width, 40);
    }

    #[test]
    fn html_defaults_distinguish_block_inline_and_non_rendered_elements() {
        let mut root = Node::element("body");
        let mut span = Node::element("span");
        span.append(Node::text("inline"));
        let mut div = Node::element("div");
        div.append(Node::text("block"));
        let head = Node::element("head");
        root.append(span);
        root.append(div);
        root.append(head);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 100));

        assert_eq!(layout.children[0].display, Display::Inline);
        assert_eq!(layout.children[1].display, Display::Block);
        assert_eq!(layout.children[2].display, Display::None);
    }

    #[test]
    fn styled_layout_uses_per_node_display_and_dimensions() {
        let mut root = Node::element("body");
        let mut first = Node::element("div");
        first.set_attribute("style", "height: 20px; margin: 4px; padding: 2px;");
        first.append(Node::text("hello"));
        let mut hidden = Node::element("div");
        hidden.set_attribute("style", "display: none;");
        root.append(first);
        root.append(hidden);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(800, 600));

        assert_eq!(layout.children.len(), 2);
        assert_eq!(layout.children[0].display, Display::Block);
        assert_eq!(layout.children[0].rect.height, 24);
        assert_eq!(layout.children[1].display, Display::None);
    }

    #[test]
    fn text_intrinsic_width_matches_bootstrap_glyph_advance() {
        let mut root = Node::element("body");
        let mut text = Node::element("span");
        text.set_attribute("style", "display: inline;");
        text.append(Node::text("Hello"));
        root.append(text);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 100));

        assert_eq!(layout.children[0].rect.width, 30);
    }

    #[test]
    fn min_and_max_width_constrain_used_content_width() {
        let mut root = Node::element("body");
        let mut child = Node::element("div");
        child.set_attribute("style", "width: 80%; min-width: 100px; max-width: 120px;");
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 100));

        assert_eq!(layout.children[0].rect.width, 120);
    }

    #[test]
    fn inline_line_height_honors_explicit_pixel_value() {
        let mut root = Node::element("body");
        let mut child = Node::element("span");
        child.set_attribute("style", "line-height: 24px;");
        child.append(Node::text("Hello"));
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 100));

        assert_eq!(layout.children[0].rect.height, 24);
    }

    #[test]
    fn min_and_max_height_constrain_used_content_height() {
        let mut root = Node::element("body");
        let mut child = Node::element("div");
        child.set_attribute("style", "min-height: 80px; max-height: 120px;");
        child.append(Node::text("short"));
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 200));

        assert_eq!(layout.children[0].rect.height, 80);
    }

    #[test]
    fn max_height_limits_content_before_padding_is_added() {
        let mut root = Node::element("body");
        let mut child = Node::element("div");
        child.set_attribute("style", "max-height: 20px; padding: 4px;");
        child.append(Node::text("content"));
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 200));

        assert_eq!(layout.children[0].rect.height, 24);
    }

    #[test]
    fn percentage_width_uses_containing_width() {
        let mut root = Node::element("body");
        let mut child = Node::element("div");
        child.set_attribute("style", "width: 50%;");
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 200));

        assert_eq!(layout.children[0].rect.width, 100);
    }

    #[test]
    fn auto_horizontal_margins_center_fixed_width_blocks() {
        let mut root = Node::element("body");
        let mut child = Node::element("div");
        child.set_attribute(
            "style",
            "width: 100px; margin-left: auto; margin-right: auto;",
        );
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(300, 100));

        assert_eq!(layout.children[0].rect.x, 100);
        assert_eq!(layout.children[0].rect.width, 100);
        assert_eq!(layout.children[0].box_model.margin_left, 100);
        assert_eq!(layout.children[0].box_model.margin_right, 100);
    }

    #[test]
    fn child_content_starts_after_parent_padding_and_border() {
        let mut root = Node::element("body");
        let mut parent = Node::element("section");
        parent.set_attribute("style", "padding: 6px; border: 2px solid;");
        let mut child = Node::element("div");
        child.set_attribute("style", "width: 20px; height: 10px;");
        parent.append(child);
        root.append(parent);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(100, 100));

        assert_eq!(layout.children[0].children[0].rect.x, 8);
        assert_eq!(layout.children[0].children[0].rect.y, 8);
    }

    #[test]
    fn adjacent_block_vertical_margins_collapse() {
        let mut root = Node::element("body");
        let mut first = Node::element("div");
        first.set_attribute("style", "height: 20px; margin-bottom: 10px;");
        let mut second = Node::element("div");
        second.set_attribute("style", "height: 20px; margin-top: 30px;");
        root.append(first);
        root.append(second);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 200));

        assert_eq!(layout.children[0].rect.y, 0);
        assert_eq!(layout.children[1].rect.y, 50);
        assert_eq!(layout.rect.height, 70);
    }

    #[test]
    fn nested_layout_coordinates_are_absolute() {
        let mut root = Node::element("body");
        let mut parent = Node::element("section");
        parent.set_attribute("style", "margin-left: 12px;");
        let mut child = Node::element("div");
        child.set_attribute("style", "margin-left: 8px;");
        parent.append(child);
        root.append(parent);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 200));

        assert_eq!(layout.children[0].rect.x, 12);
        assert_eq!(layout.children[0].children[0].rect.x, 20);
    }

    #[test]
    fn border_box_width_and_height_include_padding_and_border() {
        let mut root = Node::element("body");
        let mut child = Node::element("div");
        child.set_attribute(
            "style",
            "box-sizing: border-box; width: 100px; height: 50px; padding: 10px; border-width: 2px;",
        );
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 200));

        assert_eq!(layout.children[0].rect.width, 76);
        assert_eq!(layout.children[0].rect.height, 50);
    }

    #[test]
    fn border_width_keywords_have_stable_pixel_metrics() {
        let style = {
            let mut style = ComputedStyle::default();
            style.set("border-width", "thick");
            style
        };
        let model = box_model_from_style(&style);
        assert_eq!(model.border_top, 5);
        assert_eq!(model.border_right, 5);
    }

    #[test]
    fn box_model_outer_dimensions_are_summed_safely() {
        let model = BoxModel {
            margin_left: 4,
            margin_right: 6,
            padding_left: 8,
            padding_right: 10,
            border_left: 1,
            border_right: 2,
            ..BoxModel::default()
        };
        assert_eq!(model.horizontal_outer(), 31);
    }
}
