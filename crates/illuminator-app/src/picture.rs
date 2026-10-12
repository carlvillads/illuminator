//! Screenshotting

use gpui_kit::RenderImage;
use image::{ImageFormat, Rgba, RgbaImage};
use std::io::Cursor;
use std::sync::Arc;

const CROSSHAIR_LINE: ([u8; 3], f32) = ([0x00, 0xcc, 0xff], 0xb3 as f32 / 255.0);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
}

pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub images: Vec<(Arc<RenderImage>, Rect, f32)>,
    pub lines: Vec<Rect>,
}

impl Picture {
    pub fn compose(&self) -> RgbaImage {
        let mut picture = RgbaImage::from_pixel(self.width, self.height, Rgba([0, 0, 0, 255]));
        for (image, rect, opacity) in &self.images {
            draw_image(&mut picture, image, *rect, *opacity);
        }
        let (color, alpha) = CROSSHAIR_LINE;
        for line in &self.lines {
            for (x, y) in pixels(&picture, *line) {
                over(picture.get_pixel_mut(x, y), color, alpha);
            }
        }
        picture
    }

    pub fn png(&self) -> image::ImageResult<Vec<u8>> {
        let mut bytes = Vec::new();
        self.compose()
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)?;
        Ok(bytes)
    }
}

fn draw_image(picture: &mut RgbaImage, image: &RenderImage, rect: Rect, opacity: f32) {
    let Some(bytes) = image.as_bytes(0) else {
        return;
    };
    let size = image.size(0);
    let (width, height) = (size.width.0 as u32, size.height.0 as u32);
    if width == 0 || height == 0 || rect.width == 0 || rect.height == 0 {
        return;
    }
    for (x, y) in pixels(picture, rect) {
        let source_x = ((x - rect.left) * width / rect.width).min(width - 1);
        let source_y = ((y - rect.top) * height / rect.height).min(height - 1);
        let index = ((source_y * width + source_x) * 4) as usize;
        let [b, g, r, a] = [0, 1, 2, 3].map(|channel| bytes[index + channel]);
        over(
            picture.get_pixel_mut(x, y),
            [r, g, b],
            f32::from(a) / 255.0 * opacity,
        );
    }
}

/// The pixels of rect inside of the picture
fn pixels(picture: &RgbaImage, rect: Rect) -> impl Iterator<Item = (u32, u32)> + use<> {
    let right = (rect.left + rect.width).min(picture.width());
    let bottom = (rect.top + rect.height).min(picture.height());
    (rect.top..bottom).flat_map(move |y| (rect.left..right).map(move |x| (x, y)))
}

fn over(pixel: &mut Rgba<u8>, color: [u8; 3], alpha: f32) {
    for channel in 0..3 {
        let old = f32::from(pixel[channel]);
        pixel[channel] = (f32::from(color[channel]) * alpha + old * (1.0 - alpha)).round() as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::{Picture, Rect};
    use gpui_kit::RenderImage;
    use image::{Frame, Rgba, RgbaImage};
    use std::sync::Arc;

    fn solid(width: u32, height: u32, bgra: [u8; 4]) -> Arc<RenderImage> {
        let buffer = RgbaImage::from_pixel(width, height, Rgba(bgra));
        Arc::new(RenderImage::new(vec![Frame::new(buffer)]))
    }

    #[test]
    fn layers_blend_like_the_screen() {
        let rect = |left, top, width, height| Rect {
            left,
            top,
            width,
            height,
        };
        let views = Picture {
            width: 4,
            height: 2,
            images: vec![
                (solid(1, 1, [0, 0, 255, 255]), rect(1, 0, 2, 1), 1.0),
                (solid(1, 1, [255, 0, 0, 255]), rect(2, 0, 5, 1), 0.5),
                (solid(1, 1, [255, 255, 255, 0]), rect(0, 0, 4, 2), 1.0),
            ],
            lines: vec![rect(0, 1, 4, 1)],
        };
        let picture = views.compose();
        assert_eq!(picture.get_pixel(0, 0), &Rgba([0, 0, 0, 255]));
        assert_eq!(picture.get_pixel(1, 0), &Rgba([255, 0, 0, 255]));
        assert_eq!(picture.get_pixel(2, 0), &Rgba([128, 0, 128, 255]));
        assert_eq!(picture.get_pixel(3, 0), &Rgba([0, 0, 128, 255]));
        assert_eq!(picture.get_pixel(0, 1), &Rgba([0, 143, 179, 255]));
        assert!(views.png().unwrap().starts_with(b"\x89PNG"));
    }
}
