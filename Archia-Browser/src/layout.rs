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
            Some("block") | Some("flow-root") => Self::Block,
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
        layout_styled_node(root, 0, 0, viewport.width, viewport.width, viewport.height)
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
    viewport_width: u32,
    viewport_height: u32,
) -> LayoutNode {
    let display = display_for_styled_node(node);
    let mut output = LayoutNode::new(display);
    if display == Display::None {
        return output;
    }

    let mut box_model = box_model_from_style(&node.style, containing_width);
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
        let min_content_width = if border_box {
            min_width.saturating_sub(padding_border_x)
        } else {
            min_width
        };
        content_width = content_width.max(min_content_width);
    }
    if let Some(max_width) = parse_length(node.style.get("max-width"), containing_width) {
        let max_content_width = if border_box {
            max_width.saturating_sub(padding_border_x)
        } else {
            max_width
        };
        content_width = content_width.min(max_content_width);
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

    let padding_border_y = box_model
        .padding_top
        .saturating_add(box_model.padding_bottom)
        .saturating_add(box_model.border_top)
        .saturating_add(box_model.border_bottom);
    let explicit_height = parse_length(node.style.get("height"), viewport_height);
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

    for child in &node.children {
        let child_display = display_for_styled_node(child);
        if child_display == Display::None {
            output.children.push(LayoutNode::new(Display::None));
            continue;
        }

        if child_display == Display::Inline {
            if is_line_break(child) {
                let break_height = inline_line_height.max(used_inline_line_height(child));
                let mut child_layout = layout_styled_node(
                    child,
                    content_origin_x.saturating_add(inline_x as i32),
                    content_origin_y.saturating_add(cursor_y),
                    content_width.saturating_sub(inline_x),
                    viewport_width,
                    viewport_height,
                );
                child_layout.rect.height = break_height;
                output.children.push(child_layout);
                cursor_y = cursor_y.saturating_add(break_height as i32);
                inline_x = 0;
                inline_line_height = 0;
                continue;
            }

            let width = intrinsic_inline_width(child, content_width.saturating_sub(inline_x));
            let line_height = used_inline_line_height(child);
            if allows_inline_wrap(node)
                && inline_x > 0
                && inline_x.saturating_add(width) > content_width
            {
                cursor_y = cursor_y.saturating_add(inline_line_height as i32);
                inline_x = 0;
                inline_line_height = 0;
            }
            let child_layout = layout_styled_node(
                child,
                content_origin_x.saturating_add(inline_x as i32),
                content_origin_y.saturating_add(cursor_y),
                content_width.saturating_sub(inline_x),
                viewport_width,
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

            let child_is_absolute = is_absolute_positioned(child);
            let child_is_fixed = is_fixed_positioned(child);
            let child_x = if child_is_absolute || child_is_fixed {
                positioned_child_x(
                    child,
                    content_origin_x,
                    content_width,
                    viewport_width,
                    child_is_fixed,
                )
            } else {
                content_origin_x
            };
            let child_y = if child_is_absolute || child_is_fixed {
                positioned_child_y(
                    child,
                    content_origin_y,
                    cursor_y,
                    content_width,
                    output.rect.height,
                    viewport_height,
                    child_is_fixed,
                )
            } else {
                content_origin_y.saturating_add(cursor_y)
            };
            let child_layout = layout_styled_node(
                child,
                child_x,
                child_y,
                content_width,
                viewport_width,
                viewport_height,
            );
            if !child_is_absolute && !child_is_fixed {
                cursor_y = cursor_y.saturating_add(
                    child_layout
                        .rect
                        .height
                        .saturating_add(child_layout.box_model.vertical_outer())
                        as i32,
                );
            }
            output.children.push(child_layout);
        }
    }

    if inline_x > 0 {
        cursor_y = cursor_y.saturating_add(inline_line_height as i32);
    }

    align_inline_lines(&mut output, &node.style, content_origin_x, content_width);
    align_inline_vertical_align(&mut output, node);

    let mut content_height = explicit_height.unwrap_or(cursor_y.max(0) as u32);
    if border_box && explicit_height.is_some() {
        content_height = content_height
            .saturating_sub(box_model.padding_top)
            .saturating_sub(box_model.padding_bottom)
            .saturating_sub(box_model.border_top)
            .saturating_sub(box_model.border_bottom);
    }
    if let Some(min_height) = parse_length(node.style.get("min-height"), viewport_height) {
        let min_content_height = if border_box {
            min_height.saturating_sub(padding_border_y)
        } else {
            min_height
        };
        content_height = content_height.max(min_content_height);
    }
    if let Some(max_height) = parse_length(node.style.get("max-height"), viewport_height) {
        let max_content_height = if border_box {
            max_height.saturating_sub(padding_border_y)
        } else {
            max_height
        };
        content_height = content_height.min(max_content_height);
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

    if node
        .style
        .get("position")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("relative"))
    {
        let offset_x = parse_signed_offset(node.style.get("left"), containing_width)
            .or_else(|| {
                parse_signed_offset(node.style.get("right"), containing_width)
                    .map(|value| value.saturating_neg())
            })
            .unwrap_or(0);
        let offset_y = parse_signed_offset(node.style.get("top"), viewport_height)
            .or_else(|| {
                parse_signed_offset(node.style.get("bottom"), viewport_height)
                    .map(|value| value.saturating_neg())
            })
            .unwrap_or(0);
        if offset_x != 0 || offset_y != 0 {
            shift_layout_tree(&mut output, offset_x, offset_y);
        }
    }

    output
}

fn align_inline_lines(
    output: &mut LayoutNode,
    style: &ComputedStyle,
    content_origin_x: i32,
    content_width: u32,
) {
    let alignment = match style.get("text-align").map(str::trim) {
        Some("center") => 1,
        Some("right" | "end") => 2,
        Some("justify") => 3,
        Some("justify-all") => 4,
        _ => 0,
    };
    if alignment == 0 || output.children.is_empty() {
        return;
    }

    let mut start = 0;
    while start < output.children.len() {
        if output.children[start].display != Display::Inline {
            start += 1;
            continue;
        }

        let line_y = output.children[start].rect.y;
        let mut end = start;
        let mut right = i64::from(output.children[start].rect.x)
            + i64::from(output.children[start].rect.width)
            + i64::from(output.children[start].box_model.margin_right)
            + i64::from(output.children[start].box_model.padding_right)
            + i64::from(output.children[start].box_model.border_right);
        while end + 1 < output.children.len()
            && output.children[end + 1].display == Display::Inline
            && output.children[end + 1].rect.y == line_y
        {
            end += 1;
            let child = &output.children[end];
            right = right.max(
                i64::from(child.rect.x)
                    + i64::from(child.rect.width)
                    + i64::from(child.box_model.margin_right)
                    + i64::from(child.box_model.padding_right)
                    + i64::from(child.box_model.border_right),
            );
        }

        let left = i64::from(output.children[start].rect.x)
            - i64::from(output.children[start].box_model.margin_left);
        let line_width = right.saturating_sub(left).max(0) as u32;
        let free_space = content_width.saturating_sub(line_width);
        if alignment == 3 || alignment == 4 {
            let child_count = end.saturating_sub(start).saturating_add(1);
            let has_later_line = output.children[end + 1..]
                .iter()
                .any(|child| child.display == Display::Inline && child.rect.y > line_y);
            if child_count > 1 && (alignment == 4 || has_later_line) {
                let gap_count = child_count - 1;
                let gap = free_space / gap_count as u32;
                let remainder = free_space % gap_count as u32;
                for (offset, child) in output.children[start..=end].iter_mut().enumerate() {
                    if offset == 0 {
                        continue;
                    }
                    let extra = remainder.min(offset as u32);
                    let shift = gap.saturating_mul(offset as u32).saturating_add(extra) as i32;
                    if shift != 0 {
                        shift_layout_tree(child, shift, 0);
                    }
                }
            }
        } else {
            let shift = if alignment == 1 {
                free_space / 2
            } else {
                free_space
            } as i32;

            if shift != 0 {
                for child in &mut output.children[start..=end] {
                    shift_layout_tree(child, shift, 0);
                }
            }
        }
        start = end + 1;
    }

    let _ = content_origin_x;
}

fn align_inline_vertical_align(
    output: &mut LayoutNode,
    styled_node: &crate::style_tree::StyledNode,
) {
    let mut start = 0usize;
    while start < output.children.len() {
        if output.children[start].display != Display::Inline {
            start += 1;
            continue;
        }

        let line_y = output.children[start].rect.y;
        let mut end = start;
        while end + 1 < output.children.len()
            && output.children[end + 1].display == Display::Inline
            && output.children[end + 1].rect.y == line_y
        {
            end += 1;
        }

        let line_top = (start..=end)
            .filter_map(|index| {
                output.children.get(index).map(|child| {
                    child
                        .rect
                        .y
                        .saturating_sub(child.box_model.margin_top as i32)
                })
            })
            .min()
            .unwrap_or(line_y);
        let line_bottom = (start..=end)
            .filter_map(|index| {
                output.children.get(index).map(|child| {
                    child
                        .rect
                        .y
                        .saturating_sub(child.box_model.margin_top as i32)
                        .saturating_add(
                            child
                                .rect
                                .height
                                .saturating_add(child.box_model.vertical_outer())
                                as i32,
                        )
                })
            })
            .max()
            .unwrap_or(line_top);
        let line_height = line_bottom.saturating_sub(line_top);

        for index in start..=end {
            let Some(child) = output.children.get_mut(index) else {
                continue;
            };
            let Some(source) = styled_node.children.get(index) else {
                continue;
            };
            let alignment = source
                .style
                .get("vertical-align")
                .map(str::trim)
                .map(str::to_ascii_lowercase);

            let outer_height = child
                .rect
                .height
                .saturating_add(child.box_model.vertical_outer());
            let current_outer_top = child
                .rect
                .y
                .saturating_sub(child.box_model.margin_top as i32);

            let target_outer_top = match alignment.as_deref() {
                Some("top" | "text-top") => line_top,
                Some("bottom" | "text-bottom") => line_bottom.saturating_sub(outer_height as i32),
                Some("middle") => {
                    line_top.saturating_add(line_height.saturating_sub(outer_height as i32) / 2)
                }
                Some(value) if value.ends_with("px") => {
                    let offset = parse_signed_px(Some(value)).unwrap_or(0);
                    current_outer_top.saturating_sub(offset)
                }
                _ => continue,
            };

            let shift = target_outer_top.saturating_sub(current_outer_top);
            if shift != 0 {
                shift_layout_tree(child, 0, shift);
            }
        }

        start = end + 1;
    }
}

fn is_absolute_positioned(node: &crate::style_tree::StyledNode) -> bool {
    node.style
        .get("position")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("absolute"))
}

fn is_fixed_positioned(node: &crate::style_tree::StyledNode) -> bool {
    node.style
        .get("position")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("fixed"))
}

fn positioned_child_x(
    node: &crate::style_tree::StyledNode,
    origin_x: i32,
    containing_width: u32,
    viewport_width: u32,
    fixed: bool,
) -> i32 {
    let origin_x = if fixed { 0 } else { origin_x };
    let containing_width = if fixed {
        viewport_width
    } else {
        containing_width
    };
    let margin_left = parse_length(node.style.get("margin-left"), containing_width).unwrap_or(0);
    let margin_right = parse_length(node.style.get("margin-right"), containing_width).unwrap_or(0);
    if let Some(left) = parse_signed_offset(node.style.get("left"), containing_width) {
        return origin_x.saturating_add(left);
    }
    if let Some(right) = parse_signed_offset(node.style.get("right"), containing_width) {
        let width = parse_length(node.style.get("width"), containing_width).unwrap_or(0);
        return origin_x
            .saturating_add(
                containing_width
                    .saturating_sub(width)
                    .saturating_sub(margin_left)
                    .saturating_sub(margin_right) as i32,
            )
            .saturating_sub(right);
    }
    origin_x
}

fn positioned_child_y(
    node: &crate::style_tree::StyledNode,
    origin_y: i32,
    flow_y: i32,
    containing_width: u32,
    containing_height: u32,
    viewport_height: u32,
    fixed: bool,
) -> i32 {
    let origin_y = if fixed { 0 } else { origin_y };
    let containing_height = if fixed {
        viewport_height
    } else {
        containing_height
    };
    if let Some(top) = parse_signed_offset(node.style.get("top"), containing_height) {
        return origin_y.saturating_add(top);
    }
    if let Some(bottom) = parse_signed_offset(node.style.get("bottom"), containing_height) {
        let height = parse_length(node.style.get("height"), containing_height).unwrap_or(0);
        let margin_top = parse_length(node.style.get("margin-top"), containing_width).unwrap_or(0);
        let margin_bottom =
            parse_length(node.style.get("margin-bottom"), containing_width).unwrap_or(0);
        return origin_y
            .saturating_add(
                containing_height
                    .saturating_sub(height)
                    .saturating_sub(margin_top)
                    .saturating_sub(margin_bottom) as i32,
            )
            .saturating_sub(bottom);
    }
    origin_y.saturating_add(flow_y)
}

fn display_for_styled_node(node: &crate::style_tree::StyledNode) -> Display {
    if node.style.get("display").is_some() {
        return Display::from_style(&node.style);
    }
    if matches!(&node.node.kind, NodeKind::Element { .. })
        && node.node.attribute("hidden").is_some()
    {
        return Display::None;
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

fn allows_inline_wrap(node: &crate::style_tree::StyledNode) -> bool {
    !node
        .style
        .get("white-space")
        .is_some_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "nowrap" | "pre"))
}

fn is_line_break(node: &crate::style_tree::StyledNode) -> bool {
    matches!(&node.node.kind, NodeKind::Element { name, .. } if name == "br")
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

fn intrinsic_inline_width(node: &crate::style_tree::StyledNode, containing_width: u32) -> u32 {
    match &node.node.kind {
        NodeKind::Text(text) => text.chars().count().min(u32::MAX as usize) as u32 * 6,
        _ => {
            let children_width = node
                .children
                .iter()
                .map(|child| intrinsic_inline_width(child, containing_width))
                .fold(0, u32::saturating_add);
            let content_width = children_width
                .max(parse_length(node.style.get("width"), containing_width).unwrap_or(0));
            content_width.saturating_add(
                box_model_from_style(&node.style, containing_width).horizontal_outer(),
            )
        }
    }
}

fn used_inline_line_height(node: &crate::style_tree::StyledNode) -> u32 {
    let font_size = parse_px(node.style.get("font-size")).unwrap_or(16);
    parse_line_height(node.style.get("line-height"), font_size)
        .unwrap_or_else(|| intrinsic_inline_height(node))
        .max(1)
}

fn parse_line_height(value: Option<&str>, font_size: u32) -> Option<u32> {
    let value = value?.trim();
    if let Some(percent) = value.strip_suffix('%') {
        let percent = percent.trim().parse::<u32>().ok()?;
        return Some(
            font_size
                .saturating_mul(percent)
                .checked_div(100)
                .unwrap_or(0),
        );
    }
    if let Some(px) = value.strip_suffix("px") {
        return px.trim().parse().ok();
    }
    let multiplier = value.parse::<f32>().ok()?;
    if !multiplier.is_finite() || multiplier < 0.0 {
        return None;
    }
    Some((font_size as f32 * multiplier).round() as u32)
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

fn box_model_from_style(style: &ComputedStyle, containing_width: u32) -> BoxModel {
    BoxModel {
        margin_top: parse_length(style.get("margin-top"), containing_width)
            .unwrap_or_else(|| parse_length(style.get("margin"), containing_width).unwrap_or(0)),
        margin_right: parse_length(style.get("margin-right"), containing_width)
            .unwrap_or_else(|| parse_length(style.get("margin"), containing_width).unwrap_or(0)),
        margin_bottom: parse_length(style.get("margin-bottom"), containing_width)
            .unwrap_or_else(|| parse_length(style.get("margin"), containing_width).unwrap_or(0)),
        margin_left: parse_length(style.get("margin-left"), containing_width)
            .unwrap_or_else(|| parse_length(style.get("margin"), containing_width).unwrap_or(0)),
        padding_top: parse_length(style.get("padding-top"), containing_width)
            .unwrap_or_else(|| parse_length(style.get("padding"), containing_width).unwrap_or(0)),
        padding_right: parse_length(style.get("padding-right"), containing_width)
            .unwrap_or_else(|| parse_length(style.get("padding"), containing_width).unwrap_or(0)),
        padding_bottom: parse_length(style.get("padding-bottom"), containing_width)
            .unwrap_or_else(|| parse_length(style.get("padding"), containing_width).unwrap_or(0)),
        padding_left: parse_length(style.get("padding-left"), containing_width)
            .unwrap_or_else(|| parse_length(style.get("padding"), containing_width).unwrap_or(0)),
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
    if value.len() >= 6 && value[..5].eq_ignore_ascii_case("calc(") && value.ends_with(')') {
        return parse_calc_length(&value[5..value.len() - 1], containing_width);
    }
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

fn parse_calc_length(expression: &str, containing_width: u32) -> Option<u32> {
    let expression = expression
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    if expression.is_empty() {
        return None;
    }

    let bytes = expression.as_bytes();
    let mut index = 0usize;
    let mut total = 0_i64;
    let mut add = true;
    let mut saw_term = false;

    while index < bytes.len() {
        if bytes[index] == b'+' || bytes[index] == b'-' {
            if !saw_term || index + 1 == bytes.len() {
                return None;
            }
            add = bytes[index] == b'+';
            index += 1;
        }

        let start = index;
        while index < bytes.len() && bytes[index] != b'+' && bytes[index] != b'-' {
            index += 1;
        }
        let term = &expression[start..index];
        if term.is_empty() {
            return None;
        }

        let value = if let Some(percent) = term.strip_suffix('%') {
            let percent = percent.parse::<i64>().ok()?;
            (i64::from(containing_width) * percent).checked_div(100)?
        } else {
            let px = term.strip_suffix("px")?;
            px.parse::<i64>().ok()?
        };

        total = if add {
            total.checked_add(value)?
        } else {
            total.checked_sub(value)?
        };
        saw_term = true;
    }

    u32::try_from(total).ok()
}

fn parse_px(value: Option<&str>) -> Option<u32> {
    let value = value?.trim();
    value.strip_suffix("px")?.trim().parse().ok()
}

fn parse_signed_px(value: Option<&str>) -> Option<i32> {
    let value = value?.trim();
    value.strip_suffix("px")?.trim().parse().ok()
}

fn parse_signed_offset(value: Option<&str>, containing_size: u32) -> Option<i32> {
    let value = value?.trim();
    if let Some(percent) = value.strip_suffix('%') {
        let percent = percent.trim().parse::<i32>().ok()?;
        return containing_size
            .saturating_mul(percent.unsigned_abs())
            .checked_div(100)
            .map(|pixels| {
                if percent < 0 {
                    -(pixels as i32)
                } else {
                    pixels as i32
                }
            });
    }
    parse_signed_px(Some(value))
}

fn shift_layout_tree(node: &mut LayoutNode, offset_x: i32, offset_y: i32) {
    node.rect.x = node.rect.x.saturating_add(offset_x);
    node.rect.y = node.rect.y.saturating_add(offset_y);
    for child in &mut node.children {
        shift_layout_tree(child, offset_x, offset_y);
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
    fn br_forces_a_new_inline_line() {
        let mut root = Node::element("body");
        let mut first = Node::element("span");
        first.append(Node::text("first"));
        let br = Node::element("br");
        let mut second = Node::element("span");
        second.append(Node::text("second"));
        root.append(first);
        root.append(br);
        root.append(second);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 100));

        assert_eq!(layout.children[0].rect.y, 0);
        assert_eq!(layout.children[1].rect.y, 0);
        assert_eq!(layout.children[1].rect.height, 16);
        assert_eq!(layout.children[2].rect.y, 16);
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
    fn text_align_centers_inline_content() {
        let mut root = Node::element("body");
        root.set_attribute("style", "text-align: center; width: 100px;");
        let mut child = Node::element("span");
        child.append(Node::text("hello"));
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(100, 100));

        assert_eq!(layout.children[0].rect.x, 35);
    }

    #[test]
    fn text_align_right_moves_inline_content_to_line_end() {
        let mut root = Node::element("body");
        root.set_attribute("style", "text-align: right; width: 100px;");
        let mut child = Node::element("span");
        child.append(Node::text("hello"));
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(100, 100));

        assert_eq!(layout.children[0].rect.x, 70);
    }

    #[test]
    fn text_align_justify_distributes_non_final_line_space() {
        let mut root = Node::element("body");
        root.set_attribute("style", "text-align: justify; width: 100px;");

        for text in ["aaaaa", "bbbbb", "ccccc"] {
            let mut child = Node::element("span");
            child.append(Node::text(text));
            root.append(child);
        }

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(100, 100));

        assert_eq!(layout.children[0].rect.x, 0);
        assert_eq!(layout.children[1].rect.x, 50);
        assert_eq!(layout.children[2].rect.x, 0);
        assert_eq!(layout.children[2].rect.y, 16);
    }

    #[test]
    fn text_align_justify_keeps_final_line_start_aligned() {
        let mut root = Node::element("body");
        root.set_attribute("style", "text-align: justify; width: 100px;");

        let mut first = Node::element("span");
        first.append(Node::text("aaaaa"));
        let mut second = Node::element("span");
        second.append(Node::text("bbbbb"));
        root.append(first);
        root.append(second);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(100, 100));

        assert_eq!(layout.children[0].rect.x, 0);
        assert_eq!(layout.children[1].rect.x, 30);
    }

    #[test]
    fn vertical_align_bottom_moves_short_inline_box_to_line_bottom() {
        let mut root = Node::element("body");
        let mut tall = Node::element("span");
        tall.set_attribute("style", "line-height: 30px;");
        tall.append(Node::text("tall"));
        let mut short = Node::element("span");
        short.set_attribute("style", "line-height: 10px; vertical-align: bottom;");
        short.append(Node::text("short"));
        root.append(tall);
        root.append(short);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 100));

        assert_eq!(layout.children[0].rect.y, 0);
        assert_eq!(layout.children[1].rect.y, 20);
    }

    #[test]
    fn vertical_align_middle_centers_short_inline_box() {
        let mut root = Node::element("body");
        let mut tall = Node::element("span");
        tall.set_attribute("style", "line-height: 30px;");
        tall.append(Node::text("tall"));
        let mut short = Node::element("span");
        short.set_attribute("style", "line-height: 10px; vertical-align: middle;");
        short.append(Node::text("short"));
        root.append(tall);
        root.append(short);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 100));

        assert_eq!(layout.children[0].rect.y, 0);
        assert_eq!(layout.children[1].rect.y, 10);
    }

    #[test]
    fn white_space_nowrap_keeps_inline_content_on_one_line() {
        let mut root = Node::element("body");
        root.set_attribute("style", "white-space: nowrap;");
        let mut first = Node::element("span");
        first.append(Node::text("1234567890"));
        let mut second = Node::element("span");
        second.append(Node::text("abcdefghij"));
        root.append(first);
        root.append(second);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(60, 100));

        assert_eq!(layout.children[0].rect.y, 0);
        assert_eq!(layout.children[1].rect.y, 0);
    }

    #[test]
    fn hidden_attribute_removes_element_from_layout() {
        let mut root = Node::element("body");
        let mut hidden = Node::element("div");
        hidden.set_attribute("hidden", "");
        hidden.append(Node::text("not visible"));
        let mut visible = Node::element("div");
        visible.set_attribute("style", "height: 10px;");
        root.append(hidden);
        root.append(visible);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 100));

        assert_eq!(layout.children[0].display, Display::None);
        assert_eq!(layout.children[1].rect.y, 0);
    }

    #[test]
    fn explicit_display_overrides_hidden_attribute() {
        let mut root = Node::element("body");
        let mut shown = Node::element("div");
        shown.set_attribute("hidden", "");
        shown.set_attribute("style", "display: block; height: 10px;");
        root.append(shown);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 100));

        assert_eq!(layout.children[0].display, Display::Block);
        assert_eq!(layout.children[0].rect.height, 10);
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
    fn unitless_and_percentage_line_height_scale_from_font_size() {
        let mut root = Node::element("body");
        let mut unitless = Node::element("span");
        unitless.set_attribute("style", "font-size: 20px; line-height: 1.5;");
        unitless.append(Node::text("unitless"));
        let mut percentage = Node::element("span");
        percentage.set_attribute("style", "font-size: 20px; line-height: 150%;");
        percentage.append(Node::text("percentage"));
        root.append(unitless);
        root.append(percentage);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 100));

        assert_eq!(layout.children[0].rect.height, 30);
        assert_eq!(layout.children[1].rect.height, 30);
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
        assert_eq!(layout.children[0].rect.height, 26);
    }

    #[test]
    fn absolute_positioned_children_leave_normal_flow_untouched() {
        let mut root = Node::element("body");
        let mut parent = Node::element("div");
        parent.set_attribute("style", "position: relative; width: 200px; height: 100px;");
        let mut absolute = Node::element("div");
        absolute.set_attribute(
            "style",
            "position: absolute; left: 20px; top: 10px; width: 30px; height: 20px;",
        );
        let mut normal = Node::element("div");
        normal.set_attribute("style", "height: 15px;");
        parent.append(absolute);
        parent.append(normal);
        root.append(parent);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(300, 200));

        assert_eq!(layout.children[0].children[0].rect.x, 20);
        assert_eq!(layout.children[0].children[0].rect.y, 10);
        assert_eq!(layout.children[0].children[1].rect.y, 0);
    }

    #[test]
    fn fixed_positioned_children_use_viewport_origin_and_leave_flow() {
        let mut root = Node::element("body");
        let mut fixed = Node::element("div");
        fixed.set_attribute(
            "style",
            "position: fixed; right: 10px; bottom: 20px; width: 30px; height: 20px;",
        );
        let mut normal = Node::element("div");
        normal.set_attribute("style", "height: 15px;");
        root.append(fixed);
        root.append(normal);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(300, 200));

        assert_eq!(layout.children[0].rect.x, 260);
        assert_eq!(layout.children[0].rect.y, 160);
        assert_eq!(layout.children[1].rect.y, 0);
    }

    #[test]
    fn border_box_min_and_max_width_include_padding_and_border() {
        let mut root = Node::element("body");
        let mut child = Node::element("div");
        child.set_attribute(
            "style",
            "box-sizing: border-box; width: 20px; min-width: 50px; padding: 10px; border-width: 2px;",
        );
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 100));

        assert_eq!(layout.children[0].rect.width, 26);

        let mut root = Node::element("body");
        let mut child = Node::element("div");
        child.set_attribute(
            "style",
            "box-sizing: border-box; width: 80px; max-width: 50px; padding: 10px; border-width: 2px;",
        );
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 100));

        assert_eq!(layout.children[0].rect.width, 26);
    }

    #[test]
    fn border_box_min_and_max_height_include_padding_and_border() {
        let mut root = Node::element("body");
        let mut child = Node::element("div");
        child.set_attribute(
            "style",
            "box-sizing: border-box; height: 20px; min-height: 50px; padding: 10px 0; border-width: 2px;",
        );
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 100));

        assert_eq!(layout.children[0].rect.height, 26);

        let mut root = Node::element("body");
        let mut child = Node::element("div");
        child.set_attribute(
            "style",
            "box-sizing: border-box; height: 80px; max-height: 50px; padding: 10px 0; border-width: 2px;",
        );
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 100));

        assert_eq!(layout.children[0].rect.height, 26);
    }

    #[test]
    fn calc_lengths_combine_percentages_and_pixels() {
        let mut root = Node::element("body");
        let mut child = Node::element("div");
        child.set_attribute(
            "style",
            "width: calc(50% - 10px); margin-left: calc(10% + 5px);",
        );
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 100));

        assert_eq!(layout.children[0].rect.width, 90);
        assert_eq!(layout.children[0].box_model.margin_left, 25);
    }

    #[test]
    fn percentage_height_uses_viewport_height() {
        let mut root = Node::element("body");
        let mut child = Node::element("div");
        child.set_attribute("style", "height: 25%;");
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 80));

        assert_eq!(layout.children[0].rect.height, 20);
    }

    #[test]
    fn relative_position_offsets_box_and_descendants_without_changing_flow() {
        let mut root = Node::element("body");
        let mut first = Node::element("div");
        first.set_attribute(
            "style",
            "position: relative; left: 10px; top: 5px; height: 20px;",
        );
        let mut nested = Node::element("span");
        nested.append(Node::text("child"));
        first.append(nested);
        let mut second = Node::element("div");
        second.set_attribute("style", "height: 10px;");
        root.append(first);
        root.append(second);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 100));

        assert_eq!(layout.children[0].rect.x, 10);
        assert_eq!(layout.children[0].rect.y, 5);
        assert_eq!(layout.children[0].children[0].rect.x, 10);
        assert_eq!(layout.children[0].children[0].rect.y, 5);
        assert_eq!(layout.children[1].rect.x, 0);
        assert_eq!(layout.children[1].rect.y, 20);
    }

    #[test]
    fn relative_position_accepts_percentage_offsets() {
        let mut root = Node::element("body");
        let mut child = Node::element("div");
        child.set_attribute(
            "style",
            "position: relative; left: 10%; top: 25%; width: 20px; height: 10px;",
        );
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 80));

        assert_eq!(layout.children[0].rect.x, 20);
        assert_eq!(layout.children[0].rect.y, 20);
    }

    #[test]
    fn relative_position_uses_bottom_and_right_when_opposite_offsets_are_absent() {
        let mut root = Node::element("body");
        let mut child = Node::element("div");
        child.set_attribute("style", "position: relative; right: 7px; bottom: 3px;");
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(100, 100));

        assert_eq!(layout.children[0].rect.x, -7);
        assert_eq!(layout.children[0].rect.y, -3);
    }

    #[test]
    fn percentage_margins_and_padding_use_containing_width() {
        let mut root = Node::element("body");
        let mut child = Node::element("div");
        child.set_attribute(
            "style",
            "width: 50%; margin: 5%; padding: 10%; border-width: 2px;",
        );
        root.append(child);

        let styled =
            crate::style_tree::StyleEngine::style(&root, &crate::css::StyleSheet::default());
        let layout = LayoutEngine::layout_styled(&styled, LayoutViewport::new(200, 200));

        assert_eq!(layout.children[0].rect.x, 10);
        assert_eq!(layout.children[0].rect.width, 100);
        assert_eq!(layout.children[0].box_model.margin_left, 10);
        assert_eq!(layout.children[0].box_model.padding_left, 20);
        assert_eq!(layout.children[0].box_model.padding_top, 20);
        assert_eq!(layout.children[0].box_model.horizontal_outer(), 64);
    }

    #[test]
    fn border_width_keywords_have_stable_pixel_metrics() {
        let style = {
            let mut style = ComputedStyle::default();
            style.set("border-width", "thick");
            style
        };
        let model = box_model_from_style(&style, 100);
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
