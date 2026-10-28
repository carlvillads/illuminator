//! Screenshotting.

use gpui_kit::RenderImage;
use image::{ImageFormat, Rgba, RgbaImage};
use std::io::Cursor;
use std::sync::Arc;

use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rectangle {
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
}

/// One image in the composite drawn into `rect` at `opacity`
pub struct ScreenshotImage {
    pub image: Arc<RenderImage>,
    pub rect: Rectangle,
    pub opacity: f32,
}

pub struct ScreenshotLine {
    pub rect: Rectangle,
    pub color: u32,
}

pub struct Screenshot {
    pub width: u32,
    pub height: u32,
    pub images: Vec<ScreenshotImage>,
    pub lines: Vec<ScreenshotLine>,
}

impl Screenshot {
    pub fn compose(&self) -> RgbaImage {
        let [_, r, g, b] = theme::VIEW_BACKGROUND.to_be_bytes();
        let mut composite = RgbaImage::from_pixel(self.width, self.height, Rgba([r, g, b, 255]));
        for placed in &self.images {
            draw_image(&mut composite, &placed.image, placed.rect, placed.opacity);
        }
        for line in &self.lines {
            let [r, g, b, a] = line.color.to_be_bytes();
            for (x, y) in pixels_within(&composite, line.rect) {
                blend_over(
                    composite.get_pixel_mut(x, y),
                    [r, g, b],
                    f32::from(a) / 255.0,
                );
            }
        }
        composite
    }

    pub fn encode_png(&self) -> image::ImageResult<Vec<u8>> {
        let mut bytes = Vec::new();
        self.compose()
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)?;
        Ok(bytes)
    }
}

fn draw_image(composite: &mut RgbaImage, image: &RenderImage, rect: Rectangle, opacity: f32) {
    let Some(bytes) = image.as_bytes(0) else {
        return;
    };
    let size = image.size(0);
    let (width, height) = (size.width.0 as u32, size.height.0 as u32);
    if width == 0 || height == 0 || rect.width == 0 || rect.height == 0 {
        return;
    }
    for (x, y) in pixels_within(composite, rect) {
        let source_x = ((x - rect.left) * width / rect.width).min(width - 1);
        let source_y = ((y - rect.top) * height / rect.height).min(height - 1);
        let index = ((source_y * width + source_x) * 4) as usize;
        let [b, g, r, a] = [0, 1, 2, 3].map(|channel| bytes[index + channel]);
        blend_over(
            composite.get_pixel_mut(x, y),
            [r, g, b],
            f32::from(a) / 255.0 * opacity,
        );
    }
}

/// The pixels of rect inside of the composite
fn pixels_within(
    composite: &RgbaImage,
    rect: Rectangle,
) -> impl Iterator<Item = (u32, u32)> + use<> {
    let right = (rect.left + rect.width).min(composite.width());
    let bottom = (rect.top + rect.height).min(composite.height());
    (rect.top..bottom).flat_map(move |y| (rect.left..right).map(move |x| (x, y)))
}

fn blend_over(pixel: &mut Rgba<u8>, color: [u8; 3], alpha: f32) {
    for channel in 0..3 {
        let old = f32::from(pixel[channel]);
        pixel[channel] = (f32::from(color[channel]) * alpha + old * (1.0 - alpha)).round() as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::{Rectangle, Screenshot, ScreenshotImage, ScreenshotLine};
    use gpui_kit::RenderImage;
    use image::{Frame, Rgba, RgbaImage};
    use std::sync::Arc;

    fn solid_image(width: u32, height: u32, bgra: [u8; 4]) -> Arc<RenderImage> {
        let buffer = RgbaImage::from_pixel(width, height, Rgba(bgra));
        Arc::new(RenderImage::new(vec![Frame::new(buffer)]))
    }

    #[test]
    fn layers_blend_like_the_screen() {
        let rect = |left, top, width, height| Rectangle {
            left,
            top,
            width,
            height,
        };
        let views = Screenshot {
            width: 4,
            height: 2,
            images: vec![
                ScreenshotImage {
                    image: solid_image(1, 1, [0, 0, 255, 255]),
                    rect: rect(1, 0, 2, 1),
                    opacity: 1.0,
                },
                ScreenshotImage {
                    image: solid_image(1, 1, [255, 0, 0, 255]),
                    rect: rect(2, 0, 5, 1),
                    opacity: 0.5,
                },
                ScreenshotImage {
                    image: solid_image(1, 1, [255, 255, 255, 0]),
                    rect: rect(0, 0, 4, 2),
                    opacity: 1.0,
                },
            ],
            lines: vec![ScreenshotLine {
                rect: rect(0, 1, 4, 1),
                color: 0x00ccffb3,
            }],
        };
        let composite = views.compose();
        assert_eq!(composite.get_pixel(0, 0), &Rgba([0, 0, 0, 255]));
        assert_eq!(composite.get_pixel(1, 0), &Rgba([255, 0, 0, 255]));
        assert_eq!(composite.get_pixel(2, 0), &Rgba([128, 0, 128, 255]));
        assert_eq!(composite.get_pixel(3, 0), &Rgba([0, 0, 128, 255]));
        assert_eq!(composite.get_pixel(0, 1), &Rgba([0, 143, 179, 255]));
        assert!(views.encode_png().unwrap().starts_with(b"\x89PNG"));
    }
}
