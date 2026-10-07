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
            value if value.starts_with('#') && value.len() == 4 => {
                let digits = value.as_bytes();
                let expand = |digit: u8| -> Option<u8> {
                    let hex = char::from(digit).to_digit(16)? as u8;
                    Some(hex * 17)
                };
                Some(Self(
                    expand(digits[1])?,
                    expand(digits[2])?,
                    expand(digits[3])?,
                    255,
                ))
            }
            value if value.starts_with('#') && value.len() == 7 => {
                let rgb = u32::from_str_radix(&value[1..], 16).ok()?;
                Some(Self((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255))
            }
            value if value.starts_with('#') && value.len() == 9 => {
                let rgba = u32::from_str_radix(&value[1..], 16).ok()?;
                Some(Self(
                    (rgba >> 24) as u8,
                    (rgba >> 16) as u8,
                    (rgba >> 8) as u8,
                    rgba as u8,
                ))
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
        for character in text.chars() {
            let glyph = glyph_5x7(character);
            for (row, bits) in glyph.iter().enumerate() {
                for column in 0..5 {
                    if bits & (1 << (4 - column)) != 0 {
                        let px = cursor_x.saturating_add(column);
                        let py = y.saturating_add(row as i32);
                        if px >= 0 && py >= 0 {
                            self.blend_pixel(px as u32, py as u32, color);
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

    pub fn blend_pixel(&mut self, x: u32, y: u32, color: Color) {
        if x >= self.width() || y >= self.height() {
            return;
        }
        let index = ((y * self.width() + x) * 4) as usize;
        let source_alpha = color.3 as u64;
        if source_alpha == 255 {
            self.pixels[index..index + 4].copy_from_slice(&[color.0, color.1, color.2, color.3]);
            return;
        }
        if source_alpha == 0 {
            return;
        }

        let destination_alpha = self.pixels[index + 3] as u64;
        let inverse = 255_u64 - source_alpha;
        let output_alpha = source_alpha + destination_alpha * inverse / 255;
        if output_alpha == 0 {
            return;
        }

        for channel in 0..3 {
            let source = [color.0, color.1, color.2][channel] as u64;
            let destination = self.pixels[index + channel] as u64;
            let value = (source * source_alpha * 255 + destination * destination_alpha * inverse)
                / (output_alpha * 255);
            self.pixels[index + channel] = value.min(255) as u8;
        }
        self.pixels[index + 3] = output_alpha.min(255) as u8;
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
                self.blend_pixel(px, py, color);
            }
        }
    }
}

fn glyph_5x7(character: char) -> [u8; 7] {
    let character = character.to_ascii_uppercase();
    match character {
        'A' => [0x0e, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'B' => [0x1e, 0x11, 0x11, 0x1e, 0x11, 0x11, 0x1e],
        'C' => [0x0f, 0x10, 0x10, 0x10, 0x10, 0x10, 0x0f],
        'D' => [0x1e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1e],
        'E' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x1f],
        'F' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x10],
        'G' => [0x0f, 0x10, 0x10, 0x17, 0x11, 0x11, 0x0f],
        'H' => [0x11, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'I' => [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x1f],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x12, 0x12, 0x0c],
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
        'V' => [0x11, 0x11, 0x11, 0x11, 0x0a, 0x0a, 0x04],
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
        _ => [0x1f, 0x11, 0x15, 0x11, 0x15, 0x11, 0x1f],
    }
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
    fn fill_rect_writes_pixels() {
        let mut surface = SoftwareSurface::new(4, 4);
        surface.fill_rect(1, 1, 2, 2, Color::RED);
        let offset = ((1 * 4 + 1) * 4) as usize;
        assert_eq!(&surface.pixels()[offset..offset + 4], &[255, 0, 0, 255]);
    }

    #[test]
    fn rasterizes_text_into_pixels() {
        let mut surface = SoftwareSurface::new(8, 8);
        surface.draw_text(0, 0, "A", Color::BLACK);
        assert!(surface.pixel(2, 0).is_some_and(|pixel| pixel.3 == 255));
        assert!(surface.pixels().iter().any(|value| *value != 0));
    }

    #[test]
    fn parses_short_and_alpha_hex_colors() {
        assert_eq!(Color::parse("#abc"), Some(Color(170, 187, 204, 255)));
        assert_eq!(Color::parse("#10203080"), Some(Color(16, 32, 48, 128)));
    }

    #[test]
    fn blends_transparent_pixels_source_over() {
        let mut surface = SoftwareSurface::new(1, 1);
        surface.clear(Color::WHITE);
        surface.blend_pixel(0, 0, Color(255, 0, 0, 128));
        let pixel = surface.pixel(0, 0).unwrap();
        assert!(pixel.0 > 200);
        assert!(pixel.1 > 100 && pixel.1 < 200);
        assert_eq!(pixel.3, 255);
    }

    #[test]
    fn parses_hex_color() {
        assert_eq!(Color::parse("#102030"), Some(Color(16, 32, 48, 255)));
    }
}
