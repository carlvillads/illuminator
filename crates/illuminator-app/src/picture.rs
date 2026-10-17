//! Screenshotting.

use gpui_kit::RenderImage;
use image::{ImageFormat, Rgba, RgbaImage};
use std::io::Cursor;
use std::sync::Arc;

use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
}

/// One image in the picture drawn into `rect` at `opacity`
pub struct PictureImage {
    pub image: Arc<RenderImage>,
    pub rect: Rect,
    pub opacity: f32,
}

pub struct PictureLine {
    pub rect: Rect,
    pub color: u32,
}

pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub images: Vec<PictureImage>,
    pub lines: Vec<PictureLine>,
}

impl Picture {
    pub fn compose(&self) -> RgbaImage {
        let [_, r, g, b] = theme::VIEW_BACKGROUND.to_be_bytes();
        let mut picture = RgbaImage::from_pixel(self.width, self.height, Rgba([r, g, b, 255]));
        for placed in &self.images {
            draw_image(&mut picture, &placed.image, placed.rect, placed.opacity);
        }
        for line in &self.lines {
            let [r, g, b, a] = line.color.to_be_bytes();
            for (x, y) in pixels(&picture, line.rect) {
                over(picture.get_pixel_mut(x, y), [r, g, b], f32::from(a) / 255.0);
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
    use super::{Picture, PictureImage, PictureLine, Rect};
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
                PictureImage {
                    image: solid(1, 1, [0, 0, 255, 255]),
                    rect: rect(1, 0, 2, 1),
                    opacity: 1.0,
                },
                PictureImage {
                    image: solid(1, 1, [255, 0, 0, 255]),
                    rect: rect(2, 0, 5, 1),
                    opacity: 0.5,
                },
                PictureImage {
                    image: solid(1, 1, [255, 255, 255, 0]),
                    rect: rect(0, 0, 4, 2),
                    opacity: 1.0,
                },
            ],
            lines: vec![PictureLine {
                rect: rect(0, 1, 4, 1),
                color: 0x00ccffb3,
            }],
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
