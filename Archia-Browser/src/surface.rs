use crate::{
    core::memory::{MemoryBudget, MemoryReservation},
    html::{Node, NodeKind},
};
use std::sync::Arc;

const MAX_SOFTWARE_SURFACE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub u8, pub u8, pub u8, pub u8);

impl Color {
    pub const BLACK: Self = Self(0, 0, 0, 255);
    pub const WHITE: Self = Self(255, 255, 255, 255);
    pub const RED: Self = Self(255, 0, 0, 255);

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().to_ascii_lowercase();
        match value.as_str() {
            "black" => Some(Self::BLACK),
            "white" => Some(Self::WHITE),
            "red" => Some(Self(255, 0, 0, 255)),
            "green" => Some(Self(0, 128, 0, 255)),
            "blue" => Some(Self(0, 0, 255, 255)),
            "transparent" => Some(Self(0, 0, 0, 0)),
            value if value.len() == 4 && value.starts_with('#') => {
                let mut digits = value[1..].chars();
                let r = u8::from_str_radix(&digits.next()?.to_string().repeat(2), 16).ok()?;
                let g = u8::from_str_radix(&digits.next()?.to_string().repeat(2), 16).ok()?;
                let b = u8::from_str_radix(&digits.next()?.to_string().repeat(2), 16).ok()?;
                Some(Self(r, g, b, 255))
            }
            value if value.len() == 7 && value.starts_with('#') => {
                let rgb = u32::from_str_radix(&value[1..], 16).ok()?;
                Some(Self((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255))
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Surface {
    width: u32,
    height: u32,
}

impl Surface {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixel_count(&self) -> usize {
        (self.width as usize).saturating_mul(self.height as usize)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoftwareSurface {
    surface: Surface,
    pixels: Vec<u8>,
}

/// A software surface whose pixel allocation is held against a shared memory budget.
#[derive(Debug)]
pub struct BudgetedSoftwareSurface {
    surface: SoftwareSurface,
    _reservation: MemoryReservation,
}

impl std::ops::Deref for BudgetedSoftwareSurface {
    type Target = SoftwareSurface;

    fn deref(&self) -> &Self::Target {
        &self.surface
    }
}

impl std::ops::DerefMut for BudgetedSoftwareSurface {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.surface
    }
}

impl SoftwareSurface {
    /// Create a bounded surface, falling back to an empty surface on failure.
    pub fn new(width: u32, height: u32) -> Self {
        Self::try_new(width, height).unwrap_or_else(|| Self {
            surface: Surface::new(0, 0),
            pixels: Vec::new(),
        })
    }

    /// Fallibly allocate a surface while reserving its pixel bytes from a shared budget.
    ///
    /// The returned wrapper owns both the pixels and their reservation, so the budget
    /// remains charged for exactly as long as the accounted surface is retained.
    /// Existing callers of `try_new` keep their per-surface cap but do not participate
    /// in a shared budget until they opt into this constructor.
    pub fn try_new_with_budget(
        width: u32,
        height: u32,
        budget: Arc<MemoryBudget>,
    ) -> Option<BudgetedSoftwareSurface> {
        let byte_len = if width == 0 || height == 0 {
            0
        } else {
            usize::try_from(width)
                .ok()?
                .checked_mul(usize::try_from(height).ok()?)?
                .checked_mul(4)?
        };
        if byte_len > MAX_SOFTWARE_SURFACE_BYTES {
            return None;
        }
        let reservation = MemoryReservation::try_new(budget, byte_len)?;
        let surface = Self::try_new(width, height)?;
        Some(BudgetedSoftwareSurface {
            surface,
            _reservation: reservation,
        })
    }

    /// Fallibly allocate an RGBA surface with a 64 MiB pixel-storage limit.
    pub fn try_new(width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            return Some(Self {
                surface: Surface::new(0, 0),
                pixels: Vec::new(),
            });
        }
        let byte_len = usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(height).ok()?)?
            .checked_mul(4)?;
        if byte_len > MAX_SOFTWARE_SURFACE_BYTES {
            return None;
        }
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(byte_len).ok()?;
        pixels.resize(byte_len, 0);
        Some(Self {
            surface: Surface::new(width, height),
            pixels,
        })
    }

    pub fn surface(&self) -> Surface {
        self.surface
    }

    pub fn width(&self) -> u32 {
        self.surface.width()
    }

    pub fn height(&self) -> u32 {
        self.surface.height()
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<Color> {
        if x >= self.width() || y >= self.height() {
            return None;
        }
        let index = (y as usize)
            .checked_mul(self.width() as usize)?
            .checked_add(x as usize)?
            .checked_mul(4)?;
        let pixel = self.pixels.get(index..index.checked_add(4)?)?;
        Some(Color(pixel[0], pixel[1], pixel[2], pixel[3]))
    }

    pub fn draw_text(&mut self, x: i32, y: i32, text: &str, color: Color) {
        let mut cursor_x = x;
        let mut cursor_y = y;
        for ch in text.chars() {
            if ch == '\r' {
                continue;
            }
            if ch == '\n' {
                cursor_x = x;
                cursor_y = cursor_y.saturating_add(8);
                continue;
            }
            if ch == '\t' {
                cursor_x = cursor_x.saturating_add(24);
                continue;
            }
            if let Some(glyph) = glyph(ch) {
                for (row, bits) in glyph.iter().enumerate() {
                    for column in 0..5 {
                        if bits & (1 << (4 - column)) != 0 {
                            self.fill_rect(
                                cursor_x.saturating_add(column),
                                cursor_y.saturating_add(row as i32),
                                1,
                                1,
                                color,
                            );
                        }
                    }
                }
            }
            cursor_x = cursor_x.saturating_add(6);
        }
    }

    pub fn clear(&mut self, color: Color) {
        for pixel in self.pixels.as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&[color.0, color.1, color.2, color.3]);
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, width: u32, height: u32, color: Color) {
        self.fill_rect_clipped(x, y, width, height, color, None);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn fill_rounded_rect_clipped(
        &mut self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        top_left_x: u32,
        top_left_y: u32,
        top_right_x: u32,
        top_right_y: u32,
        bottom_right_x: u32,
        bottom_right_y: u32,
        bottom_left_x: u32,
        bottom_left_y: u32,
        color: Color,
        clip: Option<crate::layout::Rect>,
    ) {
        if top_left_x == 0
            && top_left_y == 0
            && top_right_x == 0
            && top_right_y == 0
            && bottom_right_x == 0
            && bottom_right_y == 0
            && bottom_left_x == 0
            && bottom_left_y == 0
        {
            self.fill_rect_clipped(x, y, width, height, color, clip);
            return;
        }
        let rect = crate::layout::Rect::new(x, y, width, height);
        let Some(rect) = intersect_rect(rect, clip) else {
            return;
        };
        let x0 = rect.x.max(0) as u32;
        let y0 = rect.y.max(0) as u32;
        let x1 = (rect.x.max(0) as u32)
            .saturating_add(rect.width)
            .min(self.surface.width);
        let y1 = (rect.y.max(0) as u32)
            .saturating_add(rect.height)
            .min(self.surface.height);
        let radii = [
            (
                top_left_x.min(width / 2) as i64,
                top_left_y.min(height / 2) as i64,
            ),
            (
                top_right_x.min(width / 2) as i64,
                top_right_y.min(height / 2) as i64,
            ),
            (
                bottom_right_x.min(width / 2) as i64,
                bottom_right_y.min(height / 2) as i64,
            ),
            (
                bottom_left_x.min(width / 2) as i64,
                bottom_left_y.min(height / 2) as i64,
            ),
        ];
        let left = x as i64;
        let top = y as i64;
        let right = left + width as i64 - 1;
        let bottom = top + height as i64 - 1;
        for py in y0..y1 {
            for px in x0..x1 {
                let px = px as i64;
                let py = py as i64;
                let corner = if px < left + radii[0].0 && py < top + radii[0].1 {
                    Some((
                        left + radii[0].0 - 1,
                        top + radii[0].1 - 1,
                        radii[0].0,
                        radii[0].1,
                    ))
                } else if px > right - radii[1].0 && py < top + radii[1].1 {
                    Some((
                        right - radii[1].0 + 1,
                        top + radii[1].1 - 1,
                        radii[1].0,
                        radii[1].1,
                    ))
                } else if px > right - radii[2].0 && py > bottom - radii[2].1 {
                    Some((
                        right - radii[2].0 + 1,
                        bottom - radii[2].1 + 1,
                        radii[2].0,
                        radii[2].1,
                    ))
                } else if px < left + radii[3].0 && py > bottom - radii[3].1 {
                    Some((
                        left + radii[3].0 - 1,
                        bottom - radii[3].1 + 1,
                        radii[3].0,
                        radii[3].1,
                    ))
                } else {
                    None
                };
                if let Some((cx, cy, radius_x, radius_y)) = corner {
                    // Geometry originates in untrusted document dimensions. Use i128
                    // for the ellipse equation: i64 products overflow when CSS
                    // supplies very large corner radii even though the raster area
                    // itself is clipped to the small, bounded surface.
                    let dx = i128::from(px - cx);
                    let dy = i128::from(py - cy);
                    let radius_x = i128::from(radius_x);
                    let radius_y = i128::from(radius_y);
                    let lhs = dx * dx * radius_y * radius_y + dy * dy * radius_x * radius_x;
                    let rhs = radius_x * radius_x * radius_y * radius_y;
                    if lhs > rhs {
                        continue;
                    }
                }
                let index = ((py as u32 * self.surface.width + px as u32) * 4) as usize;
                self.pixels[index..index + 4]
                    .copy_from_slice(&[color.0, color.1, color.2, color.3]);
            }
        }
    }

    pub fn fill_rect_clipped(
        &mut self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        color: Color,
        clip: Option<crate::layout::Rect>,
    ) {
        let rect = crate::layout::Rect::new(x, y, width, height);
        let Some(rect) = intersect_rect(rect, clip) else {
            return;
        };
        let x0 = rect.x.max(0) as u32;
        let y0 = rect.y.max(0) as u32;
        let x1 = (rect.x.max(0) as u32)
            .saturating_add(rect.width)
            .min(self.surface.width);
        let y1 = (rect.y.max(0) as u32)
            .saturating_add(rect.height)
            .min(self.surface.height);

        for py in y0..y1 {
            for px in x0..x1 {
                let Some(index) = (py as usize)
                    .checked_mul(self.surface.width as usize)
                    .and_then(|row| row.checked_add(px as usize))
                    .and_then(|pixel| pixel.checked_mul(4))
                else {
                    continue;
                };
                let Some(pixel) = index
                    .checked_add(4)
                    .and_then(|end| self.pixels.get_mut(index..end))
                else {
                    continue;
                };
                pixel.copy_from_slice(&[color.0, color.1, color.2, color.3]);
            }
        }
    }

    pub fn draw_text_clipped(
        &mut self,
        x: i32,
        y: i32,
        text: &str,
        color: Color,
        clip: Option<crate::layout::Rect>,
    ) {
        let mut cursor_x = x;
        let mut cursor_y = y;
        for ch in text.chars() {
            if ch == '\r' {
                continue;
            }
            if ch == '\n' {
                cursor_x = x;
                cursor_y = cursor_y.saturating_add(8);
                continue;
            }
            if ch == '\t' {
                cursor_x = cursor_x.saturating_add(24);
                continue;
            }
            if let Some(glyph) = glyph(ch) {
                for (row, bits) in glyph.iter().enumerate() {
                    for column in 0..5 {
                        if bits & (1 << (4 - column)) != 0 {
                            self.fill_rect_clipped(
                                cursor_x.saturating_add(column),
                                cursor_y.saturating_add(row as i32),
                                1,
                                1,
                                color,
                                clip,
                            );
                        }
                    }
                }
            }
            cursor_x = cursor_x.saturating_add(6);
        }
    }
}

fn intersect_rect(
    rect: crate::layout::Rect,
    clip: Option<crate::layout::Rect>,
) -> Option<crate::layout::Rect> {
    let Some(clip) = clip else {
        return Some(rect);
    };
    let left = rect.x.max(clip.x);
    let top = rect.y.max(clip.y);
    let right = (rect.x as i64 + rect.width as i64).min(clip.x as i64 + clip.width as i64);
    let bottom = (rect.y as i64 + rect.height as i64).min(clip.y as i64 + clip.height as i64);
    if right <= left as i64 || bottom <= top as i64 {
        None
    } else {
        Some(crate::layout::Rect::new(
            left,
            top,
            (right - left as i64) as u32,
            (bottom - top as i64) as u32,
        ))
    }
}

fn glyph(ch: char) -> Option<[u8; 7]> {
    let glyph = match ch.to_ascii_uppercase() {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [15, 16, 16, 16, 16, 16, 15],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [15, 16, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [31, 4, 4, 4, 4, 4, 31],
        'J' => [7, 2, 2, 2, 2, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 27, 17],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ' ' => [0, 0, 0, 0, 0, 0, 0],
        '.' => [0, 0, 0, 0, 0, 0, 4],
        ',' => [0, 0, 0, 0, 0, 4, 8],
        '!' => [4, 4, 4, 4, 4, 0, 4],
        '?' => [14, 17, 1, 2, 4, 0, 4],
        ':' => [0, 4, 0, 0, 4, 0, 0],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        _ => return None,
    };
    Some(glyph)
}

pub fn paint_background(surface: &mut SoftwareSurface, node: &Node, color: Color) {
    if matches!(node.kind, NodeKind::Element { .. }) {
        surface.fill_rect(0, 0, surface.width(), surface.height(), color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extreme_rounded_rectangle_radii_do_not_overflow() {
        let mut surface = SoftwareSurface::new(2, 2);
        surface.fill_rounded_rect_clipped(
            0,
            0,
            u32::MAX,
            u32::MAX,
            u32::MAX,
            u32::MAX,
            u32::MAX,
            u32::MAX,
            u32::MAX,
            u32::MAX,
            u32::MAX,
            u32::MAX,
            Color::RED,
            None,
        );
        assert_eq!(surface.pixels().len(), 16);
    }

    #[test]
    fn software_surface_has_rgba_storage() {
        let surface = SoftwareSurface::new(8, 4);
        assert_eq!(surface.pixels().len(), 8 * 4 * 4);
    }

    #[test]
    fn fallible_surface_constructor_rejects_oversized_dimensions_before_allocating() {
        assert!(SoftwareSurface::try_new(u32::MAX, u32::MAX).is_none());
        assert!(SoftwareSurface::try_new(8192, 2049).is_none());
    }

    #[test]
    fn budgeted_surface_reserves_and_releases_pixel_storage() {
        let budget = Arc::new(MemoryBudget::new(8 * 4 * 4));
        {
            let surface =
                SoftwareSurface::try_new_with_budget(8, 4, Arc::clone(&budget)).unwrap();
            assert_eq!(surface.pixels().len(), 8 * 4 * 4);
            assert_eq!(budget.used(), 8 * 4 * 4);
            assert_eq!(budget.available(), 0);
        }
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn budgeted_surface_refuses_allocations_over_the_shared_budget() {
        let budget = Arc::new(MemoryBudget::new(8 * 4 * 4 - 1));
        assert!(SoftwareSurface::try_new_with_budget(8, 4, Arc::clone(&budget)).is_none());
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn oversized_surface_constructor_fails_closed_to_empty() {
        let surface = SoftwareSurface::new(u32::MAX, u32::MAX);
        assert_eq!((surface.width(), surface.height()), (0, 0));
        assert!(surface.pixels().is_empty());
        assert_eq!(surface.pixel(0, 0), None);
    }

    #[test]
    fn zero_dimension_surface_is_normalized_to_empty() {
        let surface = SoftwareSurface::new(0, u32::MAX);
        assert_eq!((surface.width(), surface.height()), (0, 0));
        assert!(surface.pixels().is_empty());
    }

    #[test]
    fn surface_pixel_count_saturates_instead_of_overflowing() {
        let surface = Surface::new(u32::MAX, u32::MAX);
        assert_eq!(
            surface.pixel_count(),
            (u32::MAX as usize).saturating_mul(u32::MAX as usize)
        );
    }

    #[test]
    fn draw_text_rasterizes_glyphs() {
        let mut surface = SoftwareSurface::new(8, 8);
        surface.draw_text(0, 0, "A", Color::BLACK);
        assert_eq!(surface.pixel(1, 0), Some(Color::BLACK));
        assert_eq!(surface.pixel(0, 0), Some(Color(0, 0, 0, 0)));
        assert_eq!(surface.pixel(0, 3), Some(Color::BLACK));
    }

    #[test]
    fn drawing_text_at_extreme_coordinates_does_not_overflow() {
        let mut surface = SoftwareSurface::new(8, 8);
        surface.draw_text(i32::MAX, i32::MAX, "A", Color::BLACK);
        surface.draw_text_clipped(i32::MAX, i32::MAX, "A", Color::BLACK, None);
        assert!(surface.pixels().iter().all(|byte| *byte == 0));
    }

    #[test]
    fn draw_text_handles_newlines_and_tabs() {
        let mut surface = SoftwareSurface::new(40, 16);
        surface.draw_text(0, 0, "A\tB\nC", Color::BLACK);

        assert_eq!(surface.pixel(1, 0), Some(Color::BLACK));
        assert_eq!(surface.pixel(30, 0), Some(Color::BLACK));
        assert_eq!(surface.pixel(1, 8), Some(Color::BLACK));
    }

    #[test]
    fn rounded_rect_leaves_transparent_corners() {
        let mut surface = SoftwareSurface::new(12, 12);
        surface.fill_rounded_rect_clipped(0, 0, 12, 12, 4, 4, 4, 4, 4, 4, 4, 4, Color::RED, None);
        assert_eq!(surface.pixel(0, 0), Some(Color(0, 0, 0, 0)));
        assert_eq!(surface.pixel(5, 1), Some(Color::RED));
        assert_eq!(surface.pixel(6, 6), Some(Color::RED));
    }

    #[test]
    fn fill_rect_writes_pixels() {
        let mut surface = SoftwareSurface::new(4, 4);
        surface.fill_rect(1, 1, 2, 2, Color::RED);
        let offset = 20usize;
        assert_eq!(&surface.pixels()[offset..offset + 4], &[255, 0, 0, 255]);
    }

    #[test]
    fn parses_hex_color() {
        assert_eq!(Color::parse("#102030"), Some(Color(16, 32, 48, 255)));
        assert_eq!(Color::parse("#abc"), Some(Color(170, 187, 204, 255)));
    }
}
