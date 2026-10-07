use super::layout::{Display, LayoutNode};
use crate::css::ComputedStyle;
use crate::html::{Node, NodeKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintCommand {
    FillRect {
        rect: super::layout::Rect,
        color: u32,
    },
    DrawText {
        x: i32,
        y: i32,
        text_len: u32,
        color: u32,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DisplayList {
    commands: Vec<PaintCommand>,
}

impl DisplayList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, command: PaintCommand) {
        self.commands.push(command);
    }

    pub fn commands(&self) -> &[PaintCommand] {
        &self.commands
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }
}

pub struct SoftwareRenderer;

impl SoftwareRenderer {
    pub fn build_display_list(root: &Node, layout: &LayoutNode, style: &ComputedStyle) -> DisplayList {
        let mut list = DisplayList::new();
        paint_node(root, layout, style, &mut list);
        list
    }
}

fn paint_node(node: &Node, layout: &LayoutNode, style: &ComputedStyle, list: &mut DisplayList) {
    if layout.display == Display::None {
        return;
    }

    if let Some(background) = style.get("background-color").or_else(|| style.get("background")) {
        if let Some(color) = parse_color(background) {
            list.push(PaintCommand::FillRect {
                rect: layout.rect,
                color,
            });
        }
    }

    if let NodeKind::Text(text) = &node.kind {
        list.push(PaintCommand::DrawText {
            x: layout.rect.x,
            y: layout.rect.y,
            text_len: text.chars().count() as u32,
            color: parse_color(style.get("color").unwrap_or("black")).unwrap_or(0x000000ff),
        });
    }

    for (child, child_layout) in node.children.iter().zip(&layout.children) {
        paint_node(child, child_layout, style, list);
    }
}

fn parse_color(value: &str) -> Option<u32> {
    match value.trim().to_ascii_lowercase().as_str() {
        "black" => Some(0x000000ff),
        "white" => Some(0xffffffff),
        "red" => Some(0xff0000ff),
        "green" => Some(0x008000ff),
        "blue" => Some(0x0000ffff),
        "transparent" => Some(0x00000000),
        value if value.starts_with('#') && value.len() == 7 => {
            let rgb = u32::from_str_radix(&value[1..], 16).ok()?;
            Some((rgb << 8) | 0xff)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{LayoutEngine, LayoutViewport};

    #[test]
    fn builds_background_and_text_commands() {
        let mut root = Node::element("body");
        root.append(Node::text("Hello"));

        let mut style = ComputedStyle::default();
        style.set("background-color", "#102030");
        style.set("color", "white");

        let layout = LayoutEngine::layout(&root, LayoutViewport::new(800, 600), &style);
        let list = SoftwareRenderer::build_display_list(&root, &layout, &style);

        assert_eq!(list.len(), 2);
        assert!(matches!(list.commands()[0], PaintCommand::FillRect { .. }));
        assert!(matches!(list.commands()[1], PaintCommand::DrawText { text_len: 5, .. }));
    }

    #[test]
    fn ignores_none_nodes() {
        let root = Node::element("div");
        let mut style = ComputedStyle::default();
        style.set("display", "none");

        let layout = LayoutEngine::layout(&root, LayoutViewport::new(100, 100), &style);
        let list = SoftwareRenderer::build_display_list(&root, &layout, &style);
        assert!(list.commands().is_empty());
    }
}
