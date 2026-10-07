use super::layout::{Display, LayoutNode};
use crate::css::ComputedStyle;
use crate::html::{Node, NodeKind};
use crate::style_tree::StyledNode;
use crate::surface::{Color, SoftwareSurface};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintCommand {
    FillRect {
        rect: super::layout::Rect,
        color: u32,
    },
    DrawText {
        x: i32,
        y: i32,
        text: String,
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
    pub fn build_display_list_styled(root: &StyledNode, layout: &LayoutNode) -> DisplayList {
        let mut list = DisplayList::new();
        paint_styled_node(root, layout, &mut list);
        list
    }

    pub fn rasterize(list: &DisplayList, surface: &mut SoftwareSurface) {
        for command in list.commands() {
            match command {
                PaintCommand::FillRect { rect, color } => {
                    let color = Color(
                        ((color >> 24) & 0xff) as u8,
                        ((color >> 16) & 0xff) as u8,
                        ((color >> 8) & 0xff) as u8,
                        (color & 0xff) as u8,
                    );
                    surface.fill_rect(rect.x, rect.y, rect.width, rect.height, color);
                }
                PaintCommand::DrawText { x, y, text, color } => {
                    let color = Color(
                        ((color >> 24) & 0xff) as u8,
                        ((color >> 16) & 0xff) as u8,
                        ((color >> 8) & 0xff) as u8,
                        (color & 0xff) as u8,
                    );
                    draw_text(surface, x, y, &text, color);
                }
            }
        }
    }

    pub fn build_display_list(
        root: &Node,
        layout: &LayoutNode,
        style: &ComputedStyle,
    ) -> DisplayList {
        let mut list = DisplayList::new();
        paint_node(root, layout, style, &mut list);
        list
    }
}

fn paint_styled_node(node: &StyledNode, layout: &LayoutNode, list: &mut DisplayList) {
    if layout.display == Display::None {
        return;
    }

    if let Some(background) = node
        .style
        .get("background-color")
        .or_else(|| node.style.get("background"))
    {
        if let Some(color) = parse_color(background) {
            list.push(PaintCommand::FillRect {
                rect: layout.rect,
                color,
            });
        }
    }

    if let NodeKind::Text(text) = &node.node.kind {
        list.push(PaintCommand::DrawText {
            x: layout.rect.x,
            y: layout.rect.y,
            text: text.clone(),
            color: parse_color(node.style.get("color").unwrap_or("black")).unwrap_or(0x000000ff),
        });
    }

    for (child, child_layout) in node.children.iter().zip(&layout.children) {
        paint_styled_node(child, child_layout, list);
    }
}

fn paint_node(node: &Node, layout: &LayoutNode, style: &ComputedStyle, list: &mut DisplayList) {
    if layout.display == Display::None {
        return;
    }

    if let Some(background) = style
        .get("background-color")
        .or_else(|| style.get("background"))
    {
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
            text: text.clone(),
            color: parse_color(style.get("color").unwrap_or("black")).unwrap_or(0x000000ff),
        });
    }

    for (child, child_layout) in node.children.iter().zip(&layout.children) {
        paint_node(child, child_layout, style, list);
    }
}

fn parse_color(value: &str) -> Option<u32> {
    let color = Color::parse(value)?;
    Some(
        (u32::from(color.0) << 24)
            | (u32::from(color.1) << 16)
            | (u32::from(color.2) << 8)
            | u32::from(color.3),
    )
}

fn draw_text(surface: &mut SoftwareSurface, x: i32, y: i32, text: &str, color: Color) {
    let mut pen_x = x;
    let mut pen_y = y;
    for ch in text.chars() {
        if ch == '\n' {
            pen_x = x;
            pen_y += 8;
            continue;
        }
        let glyph = glyph(ch);
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    surface.fill_rect(pen_x + col, pen_y + row as i32, 1, 1, color);
                }
            }
        }
        pen_x += 6;
    }
}

fn glyph(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [0x0e, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'B' => [0x1e, 0x11, 0x11, 0x1e, 0x11, 0x11, 0x1e],
        'C' => [0x0f, 0x10, 0x10, 0x10, 0x10, 0x10, 0x0f],
        'D' => [0x1e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1e],
        'E' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x1f],
        'F' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x10],
        'G' => [0x0f, 0x10, 0x10, 0x17, 0x11, 0x11, 0x0f],
        'H' => [0x11, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'I' => [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x1f],
        'J' => [0x01, 0x01, 0x01, 0x01, 0x11, 0x11, 0x0e],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1f],
        'M' => [0x11, 0x1b, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'P' => [0x1e, 0x11, 0x11, 0x1e, 0x10, 0x10, 0x10],
        'Q' => [0x0e, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0d],
        'R' => [0x1e, 0x11, 0x11, 0x1e, 0x14, 0x12, 0x11],
        'S' => [0x0f, 0x10, 0x10, 0x0e, 0x01, 0x01, 0x1e],
        'T' => [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0a, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1b, 0x11],
        'X' => [0x11, 0x11, 0x0a, 0x04, 0x0a, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0a, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1f],
        '0' => [0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e],
        '1' => [0x04, 0x0c, 0x04, 0x04, 0x04, 0x04, 0x0e],
        '2' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1f],
        '3' => [0x1e, 0x01, 0x01, 0x0e, 0x01, 0x01, 0x1e],
        '4' => [0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02],
        '5' => [0x1f, 0x10, 0x10, 0x1e, 0x01, 0x01, 0x1e],
        '6' => [0x0e, 0x10, 0x10, 0x1e, 0x11, 0x11, 0x0e],
        '7' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e],
        '9' => [0x0e, 0x11, 0x11, 0x0f, 0x01, 0x01, 0x0e],
        ' ' => [0; 7],
        '.' => [0, 0, 0, 0, 0, 0, 4],
        '!' => [4, 4, 4, 4, 4, 0, 4],
        _ => [0x1f, 0x11, 0x0a, 0x04, 0x0a, 0x11, 0x1f],
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

        assert!(list.len() >= 2);
        assert!(matches!(list.commands()[0], PaintCommand::FillRect { .. }));
        assert!(matches!(
            &list.commands()[1],
            PaintCommand::DrawText { text, .. } if text == "Hello"
        ));
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
