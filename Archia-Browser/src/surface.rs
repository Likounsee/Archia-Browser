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
            value if value.starts_with('#') => Self::parse_hex(value),
            value if value.starts_with("rgb(") && value.ends_with(')') => {
                let channels = value[4..value.len() - 1]
                    .split(',')
                    .map(str::trim)
                    .map(str::parse::<u8>)
                    .collect::<Result<Vec<_>, _>>()
                    .ok()?;
                (channels.len() == 3).then(|| Self(channels[0], channels[1], channels[2], 255))
            }
            value if value.starts_with("rgba(") && value.ends_with(')') => {
                let mut channels = value[5..value.len() - 1].split(',').map(str::trim);
                let r = channels.next()?.parse::<u8>().ok()?;
                let g = channels.next()?.parse::<u8>().ok()?;
                let b = channels.next()?.parse::<u8>().ok()?;
                let a = channels.next()?.parse::<u8>().ok()?;
                channels.next().is_none().then(|| Self(r, g, b, a))
            }
            _ => None,
        }
    }

    fn parse_hex(value: &str) -> Option<Self> {
        let hex = &value[1..];
        match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()?;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()?;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()?;
                Some(Self(r * 17, g * 17, b * 17, 255))
            }
            4 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()?;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()?;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()?;
                let a = u8::from_str_radix(&hex[3..4], 16).ok()?;
                Some(Self(r * 17, g * 17, b * 17, a * 17))
            }
            6 => {
                let rgb = u32::from_str_radix(hex, 16).ok()?;
                Some(Self((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255))
            }
            8 => {
                let rgba = u32::from_str_radix(hex, 16).ok()?;
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

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn width(&self) -> u32 {
        self.surface.width()
    }

    pub fn height(&self) -> u32 {
        self.surface.height()
    }

    pub fn clear(&mut self, color: Color) {
        for pixel in self.pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[color.0, color.1, color.2, color.3]);
        }
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<Color> {
        if x >= self.width() || y >= self.height() {
            return None;
        }
        let index = ((y * self.surface.width + x) * 4) as usize;
        Some(Color(
            self.pixels[index],
            self.pixels[index + 1],
            self.pixels[index + 2],
            self.pixels[index + 3],
        ))
    }

    pub fn blend_pixel(&mut self, x: u32, y: u32, color: Color) {
        if x >= self.width() || y >= self.height() {
            return;
        }
        let index = ((y * self.surface.width + x) * 4) as usize;
        let alpha = u16::from(color.3);
        if alpha == 255 {
            self.pixels[index..index + 4].copy_from_slice(&[color.0, color.1, color.2, color.3]);
            return;
        }
        if alpha == 0 {
            return;
        }
        let inverse = 255_u16.saturating_sub(alpha);
        let dst = [
            self.pixels[index],
            self.pixels[index + 1],
            self.pixels[index + 2],
            self.pixels[index + 3],
        ];
        let out_alpha = alpha + (u16::from(dst[3]) * inverse + 127) / 255;
        let channel = |src: u8, dst: u8| {
            ((u16::from(src) * alpha + u16::from(dst) * inverse + 127) / 255) as u8
        };
        self.pixels[index..index + 4].copy_from_slice(&[
            channel(color.0, dst[0]),
            channel(color.1, dst[1]),
            channel(color.2, dst[2]),
            out_alpha.min(255) as u8,
        ]);
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
        let (width, height) = (surface.width(), surface.height());
        surface.fill_rect(0, 0, width, height, color);
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
    fn parses_hex_color() {
        assert_eq!(Color::parse("#102030"), Some(Color(16, 32, 48, 255)));
        assert_eq!(Color::parse("#abc"), Some(Color(170, 187, 204, 255)));
        assert_eq!(Color::parse("#10203080"), Some(Color(16, 32, 48, 128)));
    }

    #[test]
    fn parses_rgb_colors() {
        assert_eq!(Color::parse("rgb(1, 2, 3)"), Some(Color(1, 2, 3, 255)));
        assert_eq!(Color::parse("rgba(1, 2, 3, 4)"), Some(Color(1, 2, 3, 4)));
    }

    #[test]
    #[test]
    fn blends_transparent_pixels() {
        let mut surface = SoftwareSurface::new(1, 1);
        surface.clear(Color::WHITE);
        surface.blend_pixel(0, 0, Color(255, 0, 0, 128));
        let pixel = surface.pixel(0, 0).unwrap();
        assert!(pixel.0 > 200);
        assert!(pixel.1 < 200);
        assert_eq!(pixel.3, 255);
    }

    #[test]
    fn reads_pixels() {
        let mut surface = SoftwareSurface::new(2, 2);
        surface.fill_rect(1, 1, 1, 1, Color::RED);
        assert_eq!(surface.pixel(1, 1), Some(Color::RED));
        assert_eq!(surface.pixel(2, 2), None);
    }
}
