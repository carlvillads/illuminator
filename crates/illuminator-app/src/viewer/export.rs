//! Copying and saving screenshots.

use gpui_kit::{AppContext, ClipboardItem, Context, Image, ImageFormat, Window};
use illuminator_core::metadata::strip_extension;
use std::path::PathBuf;

use crate::screenshot::{Rectangle, Screenshot, ScreenshotImage, ScreenshotLine};
use crate::shortcuts::{CopyScreenshot, SaveScreenshot};
use crate::theme;

use super::{Viewer, view_area};

impl Viewer {
    pub(super) fn copy_screenshot(
        &mut self,
        _: &CopyScreenshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(screenshot) = self.screenshot(window) else {
            return;
        };
        cx.spawn(async move |_, cx| {
            match cx
                .background_spawn(async move { screenshot.encode_png() })
                .await
            {
                Ok(png) => cx.update(|cx| {
                    let image = Image::from_bytes(ImageFormat::Png, png);
                    cx.write_to_clipboard(ClipboardItem::new_image(&image));
                }),
                Err(error) => eprintln!("Illuminator: could not copy the image: {error}"),
            }
        })
        .detach();
    }

    pub(super) fn save_screenshot(
        &mut self,
        _: &SaveScreenshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(screenshot) = self.screenshot(window) else {
            return;
        };
        // save the image next to the base layer and name it after it
        let base = &self.layers[0].display;
        let path = base.volume().path();
        let directory = path.parent().map(PathBuf::from).unwrap_or_default();
        let name = base.name();
        let stem = strip_extension(&name);
        let chosen = cx.prompt_for_new_path(&directory, Some(&format!("{stem}.png")));
        cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(path))) = chosen.await else {
                return; // cancelled
            };
            let file = path.clone();
            let saved = cx
                .background_spawn(async move {
                    match screenshot.encode_png() {
                        Ok(png) => std::fs::write(&file, png),
                        Err(error) => Err(std::io::Error::other(error)),
                    }
                })
                .await;
            if let Err(error) = saved {
                eprintln!("Illuminator: saving {}: {error}", path.display());
            }
        })
        .detach();
    }

    fn screenshot(&self, window: &Window) -> Option<Screenshot> {
        let views = self.views(window);
        if views.is_empty() {
            return None;
        }
        let scale = window.scale_factor();
        let to_device_pixels = |value: f32| (value * scale).round().max(0.0) as u32;
        let area = view_area(window).size;
        let mut screenshot = Screenshot {
            width: to_device_pixels(area.width),
            height: to_device_pixels(area.height),
            images: Vec::new(),
            lines: Vec::new(),
        };
        for view in &views {
            let shown = self.visible_images(view);
            if shown.is_empty() {
                continue;
            }
            let image = view.image_size;
            let offset = view.image_offset();
            let rect = Rectangle {
                left: to_device_pixels(view.origin.x + offset.x),
                top: to_device_pixels(view.origin.y + offset.y),
                width: to_device_pixels(image.width),
                height: to_device_pixels(image.height),
            };
            screenshot
                .images
                .extend(shown.into_iter().map(|(image, opacity)| ScreenshotImage {
                    image,
                    rect,
                    opacity,
                }));
            if let Some((x, y)) = self.crosshair_position(view, scale) {
                let [vertical, horizontal] = theme::crosshair_colors(view.axis);
                screenshot.lines.push(ScreenshotLine {
                    rect: Rectangle {
                        left: to_device_pixels(view.origin.x + x),
                        width: to_device_pixels(1.0),
                        ..rect
                    },
                    color: vertical,
                });
                screenshot.lines.push(ScreenshotLine {
                    rect: Rectangle {
                        top: to_device_pixels(view.origin.y + y),
                        height: to_device_pixels(1.0),
                        ..rect
                    },
                    color: horizontal,
                });
            }
        }
        Some(screenshot)
    }
}
