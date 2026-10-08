use super::layout::{Display, LayoutNode, Rect};
use crate::css::ComputedStyle;
use crate::html::{Node, NodeKind};
use crate::style_tree::StyledNode;
use crate::surface::{Color, SoftwareSurface};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CornerRadii {
    top_left: u32,
    top_right: u32,
    bottom_right: u32,
    bottom_left: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaintCommand {
    FillRect {
        rect: super::layout::Rect,
        color: u32,
    },
    FillRoundedRect {
        rect: super::layout::Rect,
        radii: CornerRadii,
        color: u32,
    },
    DrawText {
        x: i32,
        y: i32,
        text: String,
        color: u32,
    },
    PushClip {
        rect: Rect,
    },
    PopClip,
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
        paint_styled_node(root, layout, &mut list, 255, false);
        list
    }

    pub fn rasterize(list: &DisplayList, surface: &mut SoftwareSurface) {
        let mut clip_stack = Vec::new();
        let mut clip = None;

        for command in list.commands() {
            match command {
                PaintCommand::FillRect { rect, color } => {
                    let color = Color(
                        ((color >> 24) & 0xff) as u8,
                        ((color >> 16) & 0xff) as u8,
                        ((color >> 8) & 0xff) as u8,
                        (color & 0xff) as u8,
                    );
                    surface.fill_rect_clipped(rect.x, rect.y, rect.width, rect.height, color, clip);
                }
                PaintCommand::FillRoundedRect { rect, radii, color } => {
                    let color = Color(
                        ((color >> 24) & 0xff) as u8,
                        ((color >> 16) & 0xff) as u8,
                        ((color >> 8) & 0xff) as u8,
                        (color & 0xff) as u8,
                    );
                    surface.fill_rounded_rect_clipped(
                        rect.x,
                        rect.y,
                        rect.width,
                        rect.height,
                        radii.top_left,
                        radii.top_right,
                        radii.bottom_right,
                        radii.bottom_left,
                        color,
                        clip,
                    );
                }
                PaintCommand::DrawText { x, y, text, color } => {
                    let color = Color(
                        ((color >> 24) & 0xff) as u8,
                        ((color >> 16) & 0xff) as u8,
                        ((color >> 8) & 0xff) as u8,
                        (color & 0xff) as u8,
                    );
                    surface.draw_text_clipped(*x, *y, text, color, clip);
                }
                PaintCommand::PushClip { rect } => {
                    clip_stack.push(clip);
                    clip = intersect_clip(clip, Some(*rect));
                }
                PaintCommand::PopClip => {
                    clip = clip_stack.pop().flatten();
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
        paint_node(root, layout, style, &mut list, 255);
        list
    }
}

fn paint_styled_node(
    node: &StyledNode,
    layout: &LayoutNode,
    list: &mut DisplayList,
    parent_opacity: u8,
    parent_visibility_hidden: bool,
) {
    if layout.display == Display::None {
        return;
    }

    let local_visibility = node
        .style
        .get("visibility")
        .map(str::trim)
        .map(str::to_ascii_lowercase);
    let visibility_hidden = match local_visibility.as_deref() {
        Some("hidden" | "collapse") => true,
        Some("visible") => false,
        _ => parent_visibility_hidden,
    };

    let opacity = effective_opacity(parent_opacity, node.style.get("opacity"));

    let mut child_indices: Vec<usize> =
        (0..node.children.len().min(layout.children.len())).collect();
    child_indices.sort_by_key(|&index| stacking_sort_key(&node.children[index].style));

    if !visibility_hidden {
        for &index in child_indices
            .iter()
            .filter(|&&index| stacking_sort_key(&node.children[index].style).0 == 0)
        {
            paint_styled_node(
                &node.children[index],
                &layout.children[index],
                list,
                opacity,
                visibility_hidden,
            );
        }
    }

    if !visibility_hidden && matches!(node.node.kind, NodeKind::Element { .. }) {
        if let Some(background) = node
            .style
            .get("background-color")
            .or_else(|| node.style.get("background"))
        {
            if let Some(color) = parse_color(background) {
                let rect = background_rect(layout);
                let radii = parse_border_radii(&node.style, rect.width, rect.height);
                if radii != CornerRadii::default() {
                    list.push(PaintCommand::FillRoundedRect {
                        rect,
                        radii,
                        color: apply_opacity(color, opacity),
                    });
                } else {
                    list.push(PaintCommand::FillRect {
                        rect,
                        color: apply_opacity(color, opacity),
                    });
                }
            }
        }
        paint_borders(&node.style, layout, list, opacity);
    }

    let clips_children = overflow_clips_children(&node.style);
    if clips_children {
        list.push(PaintCommand::PushClip {
            rect: overflow_clip_rect(layout),
        });
    }

    if !visibility_hidden {
        if let NodeKind::Text(text) = &node.node.kind {
            let normalized = normalize_render_text(text, node.style.get("white-space"));
            list.push(PaintCommand::DrawText {
                x: layout.rect.x,
                y: layout.rect.y,
                text: transform_text(&normalized, node.style.get("text-transform")),
                color: apply_opacity(
                    parse_color(node.style.get("color").unwrap_or("black")).unwrap_or(0x000000ff),
                    opacity,
                ),
            });
        }
    }

    for index in child_indices {
        if stacking_sort_key(&node.children[index].style).0 == 0 {
            continue;
        }
        paint_styled_node(
            &node.children[index],
            &layout.children[index],
            list,
            opacity,
            visibility_hidden,
        );
    }

    if clips_children {
        list.push(PaintCommand::PopClip);
    }
}

fn paint_node(
    node: &Node,
    layout: &LayoutNode,
    style: &ComputedStyle,
    list: &mut DisplayList,
    parent_opacity: u8,
) {
    if layout.display == Display::None
        || style
            .get("visibility")
            .is_some_and(|value| value.trim() == "hidden")
    {
        return;
    }

    let opacity = effective_opacity(parent_opacity, style.get("opacity"));

    if matches!(node.kind, NodeKind::Element { .. }) {
        if let Some(background) = style
            .get("background-color")
            .or_else(|| style.get("background"))
        {
            if let Some(color) = parse_color(background) {
                list.push(PaintCommand::FillRect {
                    rect: background_rect(layout),
                    color: apply_opacity(color, opacity),
                });
            }
        }
        paint_borders(style, layout, list, opacity);
    }

    if let NodeKind::Text(text) = &node.kind {
        list.push(PaintCommand::DrawText {
            x: layout.rect.x,
            y: layout.rect.y,
            text: text.clone(),
            color: apply_opacity(
                parse_color(style.get("color").unwrap_or("black")).unwrap_or(0x000000ff),
                opacity,
            ),
        });
    }

    for (child, child_layout) in node.children.iter().zip(&layout.children) {
        paint_node(child, child_layout, style, list, opacity);
    }
}

fn normalize_render_text(text: &str, white_space: Option<&str>) -> String {
    let mode = white_space
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "normal".to_owned());
    if matches!(mode.as_str(), "pre" | "pre-wrap") {
        return text.to_owned();
    }

    let mut output = String::with_capacity(text.len());
    let mut pending_space = false;
    for character in text.chars() {
        if character.is_whitespace() {
            if mode == "pre-line" && character == '\n' {
                pending_space = false;
                output.push('\n');
            } else {
                pending_space = true;
            }
        } else {
            if pending_space && !output.is_empty() && !output.ends_with('\n') {
                output.push(' ');
            }
            pending_space = false;
            output.push(character);
        }
    }
    output
}

fn transform_text(text: &str, value: Option<&str>) -> String {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("uppercase") => text.to_uppercase(),
        Some("lowercase") => text.to_lowercase(),
        Some("capitalize") => {
            let mut output = String::with_capacity(text.len());
            let mut word_start = true;
            for character in text.chars() {
                if character.is_alphanumeric() {
                    if word_start {
                        output.extend(character.to_uppercase());
                    } else {
                        output.push(character);
                    }
                    word_start = false;
                } else {
                    output.push(character);
                    word_start = true;
                }
            }
            output
        }
        _ => text.to_owned(),
    }
}

fn stacking_sort_key(style: &ComputedStyle) -> (u8, i32) {
    let positioned = style.get("position").is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "relative" | "absolute" | "fixed" | "sticky"
        )
    });
    if !positioned {
        return (1, 0);
    }

    let z_index = style
        .get("z-index")
        .and_then(|value| value.trim().parse::<i32>().ok())
        .unwrap_or(0);
    let layer = if z_index < 0 {
        0
    } else if z_index == 0 {
        2
    } else {
        3
    };
    (layer, z_index)
}

fn parse_border_radii(style: &ComputedStyle, width: u32, height: u32) -> CornerRadii {
    let Some(value) = style.get("border-radius").map(str::trim) else {
        return CornerRadii::default();
    };
    let horizontal = value.split('/').next().unwrap_or("").split_whitespace();
    let values = horizontal
        .map(|token| parse_radius_component(token, width.min(height)))
        .collect::<Option<Vec<_>>>();
    let Some(values) = values else {
        return CornerRadii::default();
    };
    if values.is_empty() || values.len() > 4 {
        return CornerRadii::default();
    }
    let radii = match values.as_slice() {
        [a] => CornerRadii {
            top_left: *a,
            top_right: *a,
            bottom_right: *a,
            bottom_left: *a,
        },
        [a, b] => CornerRadii {
            top_left: *a,
            top_right: *b,
            bottom_right: *a,
            bottom_left: *b,
        },
        [a, b, c] => CornerRadii {
            top_left: *a,
            top_right: *b,
            bottom_right: *c,
            bottom_left: *b,
        },
        [a, b, c, d] => CornerRadii {
            top_left: *a,
            top_right: *b,
            bottom_right: *c,
            bottom_left: *d,
        },
        _ => return CornerRadii::default(),
    };
    scale_corner_radii(radii, width, height)
}

fn parse_radius_component(token: &str, reference: u32) -> Option<u32> {
    if let Some(percent) = token.strip_suffix('%') {
        let percent = percent.trim().parse::<u32>().ok()?;
        return reference.saturating_mul(percent).checked_div(100);
    }
    token.strip_suffix("px")?.trim().parse::<u32>().ok()
}

fn scale_corner_radii(radii: CornerRadii, width: u32, height: u32) -> CornerRadii {
    let sum_top = radii.top_left.saturating_add(radii.top_right);
    let sum_bottom = radii.bottom_left.saturating_add(radii.bottom_right);
    let sum_left = radii.top_left.saturating_add(radii.bottom_left);
    let sum_right = radii.top_right.saturating_add(radii.bottom_right);
    let factor = [
        sum_top as f32 / width.max(1) as f32,
        sum_bottom as f32 / width.max(1) as f32,
        sum_left as f32 / height.max(1) as f32,
        sum_right as f32 / height.max(1) as f32,
    ]
    .into_iter()
    .fold(1.0_f32, f32::max);
    if factor <= 1.0 {
        return radii;
    }
    CornerRadii {
        top_left: (radii.top_left as f32 / factor).floor() as u32,
        top_right: (radii.top_right as f32 / factor).floor() as u32,
        bottom_right: (radii.bottom_right as f32 / factor).floor() as u32,
        bottom_left: (radii.bottom_left as f32 / factor).floor() as u32,
    }
}

fn effective_opacity(parent: u8, value: Option<&str>) -> u8 {
    let local = value
        .and_then(|value| value.trim().parse::<f32>().ok())
        .map(|value| value.clamp(0.0, 1.0))
        .map(|value| (value * 255.0).round() as u8)
        .unwrap_or(255);
    ((u16::from(parent) * u16::from(local) + 127) / 255) as u8
}

fn apply_opacity(color: u32, opacity: u8) -> u32 {
    let alpha = ((color & 0xff) as u16 * u16::from(opacity) + 127) / 255;
    (color & 0xffffff00) | u32::from(alpha as u8)
}

fn intersect_clip(current: Option<Rect>, next: Option<Rect>) -> Option<Rect> {
    match (current, next) {
        (None, clip) | (clip, None) => clip,
        (Some(first), Some(second)) => {
            let left = first.x.max(second.x);
            let top = first.y.max(second.y);
            let right =
                (first.x as i64 + first.width as i64).min(second.x as i64 + second.width as i64);
            let bottom =
                (first.y as i64 + first.height as i64).min(second.y as i64 + second.height as i64);
            if right <= left as i64 || bottom <= top as i64 {
                Some(Rect::new(left, top, 0, 0))
            } else {
                Some(Rect::new(
                    left,
                    top,
                    (right - left as i64) as u32,
                    (bottom - top as i64) as u32,
                ))
            }
        }
    }
}

fn overflow_clips_children(style: &ComputedStyle) -> bool {
    ["overflow", "overflow-x", "overflow-y"]
        .iter()
        .any(|property| {
            style.get(property).is_some_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "hidden" | "clip" | "auto" | "scroll"
                )
            })
        })
}

fn overflow_clip_rect(layout: &LayoutNode) -> Rect {
    Rect::new(
        layout
            .rect
            .x
            .saturating_sub(layout.box_model.padding_left as i32),
        layout
            .rect
            .y
            .saturating_sub(layout.box_model.padding_top as i32),
        layout
            .rect
            .width
            .saturating_add(layout.box_model.padding_left)
            .saturating_add(layout.box_model.padding_right),
        layout
            .rect
            .height
            .saturating_add(layout.box_model.padding_top)
            .saturating_add(layout.box_model.padding_bottom),
    )
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

fn paint_borders(style: &ComputedStyle, layout: &LayoutNode, list: &mut DisplayList, opacity: u8) {
    let border_style = style.get("border-style").map(str::trim).unwrap_or("none");
    if matches!(border_style, "none" | "hidden") {
        return;
    }
    let Some(color) = style
        .get("border-color")
        .and_then(parse_color)
        .or_else(|| style.get("color").and_then(parse_color))
    else {
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
            color: apply_opacity(color, opacity),
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
            color: apply_opacity(color, opacity),
        });
    }
    if border.border_left > 0 {
        list.push(PaintCommand::FillRect {
            rect: super::layout::Rect::new(outer_x, outer_y, border.border_left, outer_height),
            color: apply_opacity(color, opacity),
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
            color: apply_opacity(color, opacity),
        });
    }
}

fn parse_color(value: &str) -> Option<u32> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.starts_with("rgb(") || normalized.starts_with("rgba(") {
        return parse_rgb_function(&normalized);
    }
    match value.trim().to_ascii_lowercase().as_str() {
        "black" => Some(0x000000ff),
        "white" => Some(0xffffffff),
        "red" => Some(0xff0000ff),
        "green" => Some(0x008000ff),
        "blue" => Some(0x0000ffff),
        "transparent" => Some(0x00000000),
        value if value.starts_with('#') && value.len() == 4 => {
            let mut digits = value[1..].chars();
            let r = u8::from_str_radix(&digits.next()?.to_string().repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&digits.next()?.to_string().repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&digits.next()?.to_string().repeat(2), 16).ok()?;
            Some(((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xff)
        }
        value if value.starts_with('#') && value.len() == 7 => {
            let rgb = u32::from_str_radix(&value[1..], 16).ok()?;
            Some((rgb << 8) | 0xff)
        }
        _ => None,
    }
}

fn parse_rgb_function(value: &str) -> Option<u32> {
    let is_rgba = value.starts_with("rgba(");
    let prefix_len = if is_rgba { 5 } else { 4 };
    let inner = value.strip_suffix(')')?.get(prefix_len..)?;
    let components = inner.split(',').map(str::trim).collect::<Vec<_>>();
    if components.len() != if is_rgba { 4 } else { 3 } {
        return None;
    }

    let channels = components[..3]
        .iter()
        .map(|component| component.parse::<u8>().ok())
        .collect::<Option<Vec<_>>>()?;
    let alpha = if is_rgba {
        let value = components[3].parse::<f32>().ok()?;
        if !(0.0..=1.0).contains(&value) {
            return None;
        }
        (value * 255.0).round() as u8
    } else {
        255
    };

    Some(
        ((channels[0] as u32) << 24)
            | ((channels[1] as u32) << 16)
            | ((channels[2] as u32) << 8)
            | alpha as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{LayoutEngine, LayoutViewport};

    #[test]
    fn opacity_applies_to_vertical_border_sides() {
        let mut root = Node::element("div");
        root.set_attribute(
            "style",
            "width: 20px; height: 10px; border: 2px solid red; opacity: 0.5;",
        );

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(100, 100));
        let list = SoftwareRenderer::build_display_list_styled(&styled, &layout);

        assert!(list.commands().iter().any(|command| matches!(
            command,
            PaintCommand::FillRect { color: 0xff000080, rect, .. } if rect.width == 2
        )));
    }

    #[test]
    fn text_transform_changes_drawn_text() {
        let mut root = Node::element("div");
        root.set_attribute("style", "text-transform: uppercase; color: black;");
        root.append(Node::text("Hello world"));

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(100, 50));
        let list = SoftwareRenderer::build_display_list_styled(&styled, &layout);

        assert!(list.commands().iter().any(|command| matches!(
            command,
            PaintCommand::DrawText { text, .. } if text == "HELLO WORLD"
        )));
    }

    #[test]
    fn parses_rgb_and_rgba_colors() {
        assert_eq!(parse_color("rgb(16, 32, 48)"), Some(0x102030ff));
        assert_eq!(parse_color("rgba(16, 32, 48, 0.5)"), Some(0x10203080));
        assert_eq!(parse_color("rgba(16, 32, 48, 2)"), None);
    }

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
    fn border_radius_paints_a_rounded_background_command() {
        let mut root = Node::element("div");
        root.set_attribute(
            "style",
            "width: 20px; height: 20px; background: red; border-radius: 6px;",
        );
        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(40, 40));
        let list = SoftwareRenderer::build_display_list_styled(&styled, &layout);
        assert!(list.commands().iter().any(|command| matches!(
            command,
            PaintCommand::FillRoundedRect {
                radii: CornerRadii {
                    top_left: 6,
                    top_right: 6,
                    bottom_right: 6,
                    bottom_left: 6,
                },
                ..
            }
        )));
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
                rect: crate::layout::Rect::new(-4, -4, 100, 16),
                color: 0xff0000ff,
            }
        );
    }

    #[test]
    fn hidden_visibility_suppresses_painting() {
        let mut root = Node::element("div");
        root.set_attribute("style", "visibility: hidden; background-color: red;");
        root.append(Node::text("Hidden"));

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(100, 50));
        let list = SoftwareRenderer::build_display_list_styled(&styled, &layout);

        assert!(list.commands().is_empty());
    }

    #[test]
    fn border_currentcolor_falls_back_to_text_color() {
        let mut node = Node::element("div");
        node.set_attribute("style", "color: blue; border: 2px solid;");
        let styled =
            crate::style_tree::StyleEngine::style(&node, &crate::css::StyleSheet::default());
        let layout =
            LayoutEngine::layout_styled(&styled, crate::layout::LayoutViewport::new(100, 100));
        let list = SoftwareRenderer::build_display_list_styled(&styled, &layout);

        assert!(list.commands().iter().any(|command| {
            matches!(
                command,
                PaintCommand::FillRect {
                    color: 0x0000ffff,
                    ..
                }
            )
        }));
    }

    #[test]
    fn paints_explicit_solid_borders() {
        let mut root = Node::element("div");
        root.set_attribute(
            "style",
            "width: 20px; height: 10px; border-width: 2px; border-style: solid; border-color: blue;",
        );

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(100, 100));
        let list = SoftwareRenderer::build_display_list_styled(&styled, &layout);

        assert!(list.commands().iter().any(|command| matches!(
            command,
            PaintCommand::FillRect {
                color: 0x0000ffff,
                ..
            }
        )));
    }

    #[test]
    fn overflow_hidden_adds_a_clip_around_descendants() {
        let mut root = Node::element("div");
        root.set_attribute(
            "style",
            "width: 20px; height: 10px; padding: 2px; overflow: hidden;",
        );
        let mut child = Node::element("span");
        child.append(Node::text("This text overflows"));
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(100, 100));
        let list = SoftwareRenderer::build_display_list_styled(&styled, &layout);

        assert!(list
            .commands()
            .iter()
            .any(|command| matches!(command, PaintCommand::PushClip { .. })));
        assert!(matches!(
            list.commands().last(),
            Some(PaintCommand::PopClip)
        ));
    }

    #[test]
    fn overflow_axis_properties_add_descendant_clips() {
        for property in ["overflow-x", "overflow-y"] {
            let mut root = Node::element("div");
            root.set_attribute(
                "style",
                &format!("width: 20px; height: 10px; {property}: hidden;"),
            );
            root.append(Node::text("overflow"));

            let styled =
                crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
            let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(100, 100));
            let list = SoftwareRenderer::build_display_list_styled(&styled, &layout);

            assert!(list
                .commands()
                .iter()
                .any(|command| matches!(command, PaintCommand::PushClip { .. })));
        }
    }

    #[test]
    fn overflow_clip_auto_and_scroll_add_descendant_clips() {
        for overflow in ["clip", "auto", "scroll"] {
            let mut root = Node::element("div");
            root.set_attribute(
                "style",
                &format!("width: 20px; height: 10px; overflow: {overflow};"),
            );
            root.append(Node::text("overflow"));

            let styled =
                crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
            let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(100, 100));
            let list = SoftwareRenderer::build_display_list_styled(&styled, &layout);

            assert!(list
                .commands()
                .iter()
                .any(|command| matches!(command, PaintCommand::PushClip { .. })));
            assert!(matches!(
                list.commands().last(),
                Some(PaintCommand::PopClip)
            ));
        }
    }

    #[test]
    fn nested_clips_intersect_in_rasterization() {
        let list = DisplayList {
            commands: vec![
                PaintCommand::PushClip {
                    rect: Rect::new(0, 0, 10, 10),
                },
                PaintCommand::PushClip {
                    rect: Rect::new(5, 5, 10, 10),
                },
                PaintCommand::FillRect {
                    rect: Rect::new(0, 0, 20, 20),
                    color: 0xff0000ff,
                },
                PaintCommand::PopClip,
                PaintCommand::PopClip,
            ],
        };
        let mut surface = SoftwareSurface::new(20, 20);
        SoftwareRenderer::rasterize(&list, &mut surface);

        assert_eq!(surface.pixel(4, 4), Some(Color(0, 0, 0, 0)));
        assert_eq!(surface.pixel(5, 5), Some(Color(255, 0, 0, 255)));
        assert_eq!(surface.pixel(14, 14), Some(Color(0, 0, 0, 0)));
    }

    #[test]
    fn negative_z_index_paints_behind_parent_background() {
        let mut root = Node::element("div");
        root.set_attribute(
            "style",
            "width: 20px; height: 20px; background-color: white;",
        );
        let mut child = Node::element("div");
        child.set_attribute(
            "style",
            "position: absolute; left: 0; top: 0; width: 20px; height: 20px; background-color: red; z-index: -1;",
        );
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(40, 40));
        let list = SoftwareRenderer::build_display_list_styled(&styled, &layout);
        let fills = list
            .commands()
            .iter()
            .filter_map(|command| match command {
                PaintCommand::FillRect { color, .. } => Some(*color),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(fills, vec![0xff0000ff, 0xffffffff]);
    }

    #[test]
    fn z_index_orders_positioned_siblings_by_stack_level() {
        let mut root = Node::element("div");
        let mut low = Node::element("div");
        low.set_attribute(
            "style",
            "position: absolute; left: 0; top: 0; width: 10px; height: 10px; background-color: red; z-index: 1;",
        );
        let mut high = Node::element("div");
        high.set_attribute(
            "style",
            "position: absolute; left: 0; top: 0; width: 10px; height: 10px; background-color: blue; z-index: 2;",
        );
        root.append(high);
        root.append(low);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(100, 100));
        let list = SoftwareRenderer::build_display_list_styled(&styled, &layout);
        let fills = list
            .commands()
            .iter()
            .filter_map(|command| match command {
                PaintCommand::FillRect { color, .. } => Some(*color),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(fills, vec![0xff0000ff, 0x0000ffff]);
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

    #[test]
    fn visibility_hidden_allows_explicitly_visible_descendants() {
        let mut root = Node::element("div");
        root.set_attribute("style", "visibility: hidden;");
        let mut child = Node::element("span");
        child.set_attribute("style", "visibility: visible; color: red;");
        child.append(Node::text("Visible"));
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(100, 50));
        let list = SoftwareRenderer::build_display_list_styled(&styled, &layout);

        assert!(list.commands().iter().any(|command| matches!(
            command,
            PaintCommand::DrawText { text, color, .. }
                if text == "Visible" && *color == 0xff0000ff
        )));
    }
}
