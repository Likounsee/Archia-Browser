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
            Some("inline") => Self::Inline,
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
    let display = Display::from_style(&node.style);
    let mut output = LayoutNode::new(display);
    if display == Display::None {
        return output;
    }

    let box_model = box_model_from_style(&node.style);
    let margin_x = box_model.margin_left.saturating_add(box_model.margin_right);
    let padding_border_x = box_model
        .padding_left
        .saturating_add(box_model.padding_right)
        .saturating_add(box_model.border_left)
        .saturating_add(box_model.border_right);
    let content_width = parse_px(node.style.get("width")).unwrap_or_else(|| {
        containing_width
            .saturating_sub(margin_x)
            .saturating_sub(padding_border_x)
    });

    output.box_model = box_model;
    output.rect.x = x.saturating_add(box_model.margin_left as i32);
    output.rect.y = y.saturating_add(box_model.margin_top as i32);
    output.rect.width = content_width;

    let explicit_height = parse_px(node.style.get("height"));
    let mut cursor_y = 0_i32;
    let mut inline_x = 0_u32;
    let mut inline_line_height = 0_u32;

    for child in &node.children {
        let child_display = Display::from_style(&child.style);
        if child_display == Display::None {
            continue;
        }

        if child_display == Display::Inline {
            let width = intrinsic_inline_width(child);
            let line_height = intrinsic_inline_height(child).max(16);
            if inline_x > 0 && inline_x.saturating_add(width) > content_width {
                cursor_y = cursor_y.saturating_add(inline_line_height as i32);
                inline_x = 0;
                inline_line_height = 0;
            }
            let child_layout = layout_styled_node(
                child,
                inline_x as i32,
                cursor_y,
                content_width.saturating_sub(inline_x),
                viewport_height,
            );
            inline_x = inline_x.saturating_add(child_layout.rect.width);
            inline_line_height = inline_line_height.max(line_height);
            output.children.push(child_layout);
        } else {
            if inline_x > 0 {
                cursor_y = cursor_y.saturating_add(inline_line_height as i32);
                inline_x = 0;
                inline_line_height = 0;
            }
            let child_layout =
                layout_styled_node(child, 0, cursor_y, content_width, viewport_height);
            cursor_y = cursor_y.saturating_add(
                child_layout
                    .rect
                    .height
                    .saturating_add(child_layout.box_model.vertical_outer()) as i32,
            );
            output.children.push(child_layout);
        }
    }

    if inline_x > 0 {
        cursor_y = cursor_y.saturating_add(inline_line_height as i32);
    }

    let content_height = explicit_height.unwrap_or(cursor_y.max(0) as u32);
    output.rect.height = content_height
        .saturating_add(box_model.padding_top)
        .saturating_add(box_model.padding_bottom)
        .saturating_add(box_model.border_top)
        .saturating_add(box_model.border_bottom)
        .min(viewport_height.max(content_height));
    output
}

fn intrinsic_inline_width(node: &crate::style_tree::StyledNode) -> u32 {
    match &node.node.kind {
        NodeKind::Text(text) => text.chars().count().min(u32::MAX as usize) as u32 * 8,
        _ => node
            .children
            .iter()
            .map(intrinsic_inline_width)
            .fold(0, u32::saturating_add)
            .max(parse_px(node.style.get("width")).unwrap_or(0)),
    }
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

fn parse_border_width(value: Option<&str>) -> Option<u32> {
    parse_px(value)
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
