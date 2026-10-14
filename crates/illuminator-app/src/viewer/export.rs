//! export.rs
//!
//! Copying and saving screenshots.
//!

use super::*;
use illuminator_core::meta::strip_extension;

impl Viewer {
    pub(super) fn copy_image(
        &mut self,
        _: &CopyImage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(picture) = self.picture(window) else {
            return;
        };
        cx.spawn(async move |_, cx| {
            match cx.background_spawn(async move { picture.png() }).await {
                Ok(png) => cx.update(|cx| {
                    let image = Image::from_bytes(ImageFormat::Png, png);
                    cx.write_to_clipboard(ClipboardItem::new_image(&image));
                }),
                Err(error) => eprintln!("Illuminator: could not copy the image: {error}"),
            }
        })
        .detach();
    }

    pub(super) fn save_image(
        &mut self,
        _: &SaveImage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(picture) = self.picture(window) else {
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
                    match picture.png() {
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

    fn picture(&self, window: &Window) -> Option<Picture> {
        let views = self.views(window);
        if views.is_empty() {
            return None;
        }
        let scale = window.scale_factor();
        let device = |value: f32| (value * scale).round().max(0.0) as u32;
        let area = view_area(window);
        let mut picture = Picture {
            width: device(area.width),
            height: device(area.height),
            images: Vec::new(),
            lines: Vec::new(),
        };
        for view in &views {
            let shown = self.shown(view);
            if shown.is_empty() {
                continue;
            }
            let image = view.image_size;
            let offset = view.image_offset();
            let rect = Rect {
                left: device(view.origin.x + offset.x),
                top: device(view.origin.y + offset.y),
                width: device(image.width),
                height: device(image.height),
            };
            picture
                .images
                .extend(shown.into_iter().map(|(image, opacity)| PictureImage {
                    image,
                    rect,
                    opacity,
                }));
            if let Some((x, y)) = self.crosshair_at(view, scale) {
                picture.lines.push(Rect {
                    left: device(view.origin.x + x),
                    width: device(1.0),
                    ..rect
                });
                picture.lines.push(Rect {
                    top: device(view.origin.y + y),
                    height: device(1.0),
                    ..rect
                });
            }
        }
        Some(picture)
    }
}
