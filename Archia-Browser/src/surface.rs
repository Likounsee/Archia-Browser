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

    pub fn draw_text_placeholder(&mut self, x: i32, y: i32, text_len: u32, color: Color) {
        let width = text_len.saturating_mul(6);
        if width == 0 {
            return;
        }
        self.fill_rect(x, y, width, 1, color);
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
        let source_alpha = color.3 as u16;
        if source_alpha == 255 {
            self.pixels[index..index + 4].copy_from_slice(&[color.0, color.1, color.2, color.3]);
            return;
        }
        if source_alpha == 0 {
            return;
        }

        let destination_alpha = self.pixels[index + 3] as u16;
        let inverse = 255 - source_alpha;
        let output_alpha = source_alpha + destination_alpha * inverse / 255;
        if output_alpha == 0 {
            return;
        }

        for channel in 0..3 {
            let source = [color.0, color.1, color.2][channel] as u16;
            let destination = self.pixels[index + channel] as u16;
            let value = (source * source_alpha * 255
                + destination * destination_alpha * inverse)
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
