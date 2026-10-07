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
        Self { x, y, width, height }
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

fn layout_children(node: &Node, output: &mut LayoutNode, containing_width: u32) {
    let mut cursor_y = 0_i32;

    for child in &node.children {
        let child_display = if matches!(child.kind, NodeKind::Text(_)) {
            Display::Inline
        } else {
            Display::Block
        };
        if child_display == Display::None {
            continue;
        }

        let height = intrinsic_height(child);
        let mut child_layout = LayoutNode::new(child_display);
        child_layout.rect = Rect::new(0, cursor_y, containing_width, height);
        layout_children(child, &mut child_layout, containing_width);
        cursor_y = cursor_y.saturating_add(height as i32);
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
            child_height.max(16)
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
