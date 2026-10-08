use crate::html::{Node, NodeKind};

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
        self.width as usize * self.height as usize
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoftwareSurface {
    surface: Surface,
    pixels: Vec<u8>,
}

impl SoftwareSurface {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            surface: Surface::new(width, height),
            pixels: vec![0; width as usize * height as usize * 4],
        }
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
        let index = ((y * self.width() + x) * 4) as usize;
        Some(Color(
            self.pixels[index],
            self.pixels[index + 1],
            self.pixels[index + 2],
            self.pixels[index + 3],
        ))
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
                            self.fill_rect(cursor_x + column, cursor_y + row as i32, 1, 1, color);
                        }
                    }
                }
            }
            cursor_x = cursor_x.saturating_add(6);
        }
    }

    pub fn clear(&mut self, color: Color) {
        for pixel in self.pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[color.0, color.1, color.2, color.3]);
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, width: u32, height: u32, color: Color) {
        let x0 = x.max(0) as u32;
        let y0 = y.max(0) as u32;
        let x1 = (x.max(0) as u32)
            .saturating_add(width)
            .min(self.surface.width);
        let y1 = (y.max(0) as u32)
            .saturating_add(height)
            .min(self.surface.height);

        for py in y0..y1 {
            for px in x0..x1 {
                let index = ((py * self.surface.width + px) * 4) as usize;
                self.pixels[index..index + 4]
                    .copy_from_slice(&[color.0, color.1, color.2, color.3]);
            }
        }
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
    fn software_surface_has_rgba_storage() {
        let surface = SoftwareSurface::new(8, 4);
        assert_eq!(surface.pixels().len(), 8 * 4 * 4);
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
    fn draw_text_handles_newlines_and_tabs() {
        let mut surface = SoftwareSurface::new(40, 16);
        surface.draw_text(0, 0, "A\tB\nC", Color::BLACK);

        assert_eq!(surface.pixel(1, 0), Some(Color::BLACK));
        assert_eq!(surface.pixel(30, 0), Some(Color::BLACK));
        assert_eq!(surface.pixel(1, 8), Some(Color::BLACK));
    }

    #[test]
    fn fill_rect_writes_pixels() {
        let mut surface = SoftwareSurface::new(4, 4);
        surface.fill_rect(1, 1, 2, 2, Color::RED);
        let offset = ((1 * 4 + 1) * 4) as usize;
        assert_eq!(&surface.pixels()[offset..offset + 4], &[255, 0, 0, 255]);
    }

    #[test]
    fn parses_hex_color() {
        assert_eq!(Color::parse("#102030"), Some(Color(16, 32, 48, 255)));
        assert_eq!(Color::parse("#abc"), Some(Color(170, 187, 204, 255)));
    }
}
