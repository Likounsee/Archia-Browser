use super::layout::{Display, LayoutNode};
use crate::css::ComputedStyle;
use crate::html::{Node, NodeKind};
use crate::style_tree::StyledNode;
use crate::surface::{Color, SoftwareSurface};

#[derive(Debug, Clone, PartialEq, Eq)]
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
                    surface.draw_text(*x, *y, text, color);
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

    if matches!(node.node.kind, NodeKind::Element { .. }) {
        if let Some(background) = node
            .style
            .get("background-color")
            .or_else(|| node.style.get("background"))
        {
            if let Some(color) = parse_color(background) {
                list.push(PaintCommand::FillRect {
                    rect: background_rect(layout),
                    color,
                });
            }
        }
        paint_borders(node.style.get("border-color"), layout, list);
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

    if matches!(node.kind, NodeKind::Element { .. }) {
        if let Some(background) = style
            .get("background-color")
            .or_else(|| style.get("background"))
        {
            if let Some(color) = parse_color(background) {
                list.push(PaintCommand::FillRect {
                    rect: background_rect(layout),
                    color,
                });
            }
        }
        paint_borders(style.get("border-color"), layout, list);
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

fn background_rect(layout: &LayoutNode) -> super::layout::Rect {
    let x = layout
        .rect
        .x
        .saturating_sub(layout.box_model.padding_left as i32);
    let y = layout
        .rect
        .y
        .saturating_sub(layout.box_model.padding_top as i32);
    let width = layout
        .rect
        .width
        .saturating_add(layout.box_model.padding_left)
        .saturating_add(layout.box_model.padding_right);
    let height = layout
        .rect
        .height
        .saturating_add(layout.box_model.padding_top)
        .saturating_add(layout.box_model.padding_bottom);
    super::layout::Rect::new(x, y, width, height)
}

fn paint_borders(border_color: Option<&str>, layout: &LayoutNode, list: &mut DisplayList) {
    let Some(color) = border_color.and_then(parse_color) else {
        return;
    };
    let border = &layout.box_model;
    let outer_x = layout
        .rect
        .x
        .saturating_sub(border.padding_left.saturating_add(border.border_left) as i32);
    let outer_y = layout
        .rect
        .y
        .saturating_sub(border.padding_top.saturating_add(border.border_top) as i32);
    let outer_width = layout
        .rect
        .width
        .saturating_add(border.padding_left)
        .saturating_add(border.padding_right)
        .saturating_add(border.border_left)
        .saturating_add(border.border_right);
    let outer_height = layout
        .rect
        .height
        .saturating_add(border.padding_top)
        .saturating_add(border.padding_bottom)
        .saturating_add(border.border_top)
        .saturating_add(border.border_bottom);

    if border.border_top > 0 {
        list.push(PaintCommand::FillRect {
            rect: super::layout::Rect::new(outer_x, outer_y, outer_width, border.border_top),
            color,
        });
    }
    if border.border_bottom > 0 {
        list.push(PaintCommand::FillRect {
            rect: super::layout::Rect::new(
                outer_x,
                outer_y.saturating_add(outer_height.saturating_sub(border.border_bottom) as i32),
                outer_width,
                border.border_bottom,
            ),
            color,
        });
    }
    if border.border_left > 0 {
        list.push(PaintCommand::FillRect {
            rect: super::layout::Rect::new(outer_x, outer_y, border.border_left, outer_height),
            color,
        });
    }
    if border.border_right > 0 {
        list.push(PaintCommand::FillRect {
            rect: super::layout::Rect::new(
                outer_x.saturating_add(outer_width.saturating_sub(border.border_right) as i32),
                outer_y,
                border.border_right,
                outer_height,
            ),
            color,
        });
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
        assert!(matches!(
            list.commands()[1],
            PaintCommand::DrawText { ref text, .. } if text == "Hello"
        ));
    }

    #[test]
    fn paints_background_over_content_and_padding_box() {
        let mut root = Node::element("div");
        root.set_attribute("style", "background-color: red; padding: 4px;");

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(100, 100));
        let list = SoftwareRenderer::build_display_list_styled(&styled, &layout);

        assert_eq!(
            list.commands()[0],
            PaintCommand::FillRect {
                rect: crate::layout::Rect::new(0, 0, 100, 8),
                color: 0xff0000ff,
            }
        );
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
