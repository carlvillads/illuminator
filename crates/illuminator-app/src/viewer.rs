//! Main view

use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use illuminator_core::cache::VolumeCache;
use illuminator_core::display::VolumeDisplay;
use illuminator_core::geometry::{Point3D, SlicePlane, ViewPlane};
use illuminator_core::loader::{LoadState, VolumeLoader};
use illuminator_core::slicer::{Coloring, SliceImage, fit_size};
use image::{Frame, RgbaImage};
use std::path::PathBuf;
use std::sync::Arc;

const SIDEBAR_WIDTH: f32 = 300.0;

#[derive(Clone, Copy, PartialEq)]
struct ImageKey {
    plane: SlicePlane,
    coloring: Coloring,
    frame: usize,
    crosshair: Point3D,
    width: usize,
    height: usize,
}

struct PanelImage {
    key: ImageKey,
    image: Arc<RenderImage>,
}

struct Column {
    display: VolumeDisplay,
    images: [Option<PanelImage>; 3],
}

struct PanelLayout {
    origin: Point<f32>,
    size: Size<f32>,
    image_pixels: (usize, usize),
    image_size: Size<f32>,
}

pub struct Viewer {
    loader: VolumeLoader,
    columns: Vec<Column>,
    errors: Vec<String>,
    _loader_events: Task<()>,
}

impl Viewer {
    pub fn new(cache: Arc<VolumeCache>, files: Vec<PathBuf>, cx: &mut Context<Self>) -> Self {
        let (sender, mut changes) = mpsc::unbounded::<()>();
        let loader = VolumeLoader::new(cache, move || {
            let _ = sender.unbounded_send(());
        });
        for file in files {
            loader.enqueue(file);
        }
        let loader_events = cx.spawn(async move |this, cx| {
            while changes.next().await.is_some() {
                while changes.try_recv().is_ok() {}
                if this
                    .update(cx, |viewer, cx| viewer.take_loaded(cx))
                    .is_err()
                {
                    break; // viewer is gone here
                }
            }
        });
        Self {
            loader,
            columns: Vec::new(),
            errors: Vec::new(),
            _loader_events: loader_events,
        }
    }

    fn take_loaded(&mut self, cx: &mut Context<Self>) {
        for loaded in self.loader.take_completed() {
            match VolumeDisplay::new(loaded) {
                Ok(display) => self.columns.push(Column {
                    display,
                    images: [None, None, None],
                }),
                Err(error) => self.errors.push(error.to_string()),
            }
        }
        cx.notify();
    }

    fn open_files(&mut self, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Open".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await {
                let _ = this.update(cx, |viewer, _| {
                    for path in paths {
                        viewer.loader.enqueue(path);
                    }
                });
            }
        })
        .detach();
    }

    fn layout(&self, column: usize, row: usize, window: &Window) -> PanelLayout {
        let viewport = window.viewport_size();
        let area_width = (f32::from(viewport.width) - SIDEBAR_WIDTH).max(0.0);
        let width = area_width / self.columns.len().max(1) as f32;
        let height = f32::from(viewport.height) / 3.0;
        let scale = window.scale_factor();
        let plane = self.columns[column]
            .display
            .plane(ViewPlane::ALL[row], false);
        let image_pixels = fit_size(&plane, (width * scale) as usize, (height * scale) as usize);
        PanelLayout {
            origin: point(column as f32 * width, row as f32 * height),
            size: size(width, height),
            image_pixels,
            image_size: size(image_pixels.0 as f32 / scale, image_pixels.1 as f32 / scale),
        }
    }

    fn pick(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let (x, y) = (f32::from(position.x), f32::from(position.y));
        for column in 0..self.columns.len() {
            for row in 0..3 {
                let layout = self.layout(column, row, window);
                let local_x = x - layout.origin.x;
                let local_y = y - layout.origin.y;
                if local_x < 0.0
                    || local_y < 0.0
                    || local_x >= layout.size.width
                    || local_y >= layout.size.height
                {
                    continue;
                }
                let left = (layout.size.width - layout.image_size.width) / 2.0;
                let top = (layout.size.height - layout.image_size.height) / 2.0;
                let s = (local_x - left) / layout.image_size.width;
                let t = 1.0 - (local_y - top) / layout.image_size.height;
                let axis = ViewPlane::ALL[row];
                if self.columns[column]
                    .display
                    .pick(axis, false, f64::from(s), f64::from(t))
                {
                    cx.notify();
                }
                return;
            }
        }
    }

    /// Renders any updated panels since last draw
    fn update_images(&mut self, window: &mut Window, cx: &mut App) {
        for column in 0..self.columns.len() {
            for row in 0..3 {
                let layout = self.layout(column, row, window);
                let (width, height) = layout.image_pixels;
                let display = &self.columns[column].display;
                let axis = ViewPlane::ALL[row];
                let key = ImageKey {
                    plane: display.plane(axis, false),
                    coloring: display.coloring(),
                    frame: display.frame(),
                    crosshair: display.cursor(),
                    width,
                    height,
                };
                let current = &self.columns[column].images[row];
                if width == 0 || height == 0 || current.as_ref().is_some_and(|p| p.key == key) {
                    continue;
                }
                let image = to_render_image(display.slice(axis, false, true).render(width, height));
                let replaced = self.columns[column].images[row].replace(PanelImage { key, image });
                if let Some(old) = replaced {
                    cx.drop_image(old.image, Some(window));
                }
            }
        }
    }

    fn render_panels(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, label_color) = (theme.muted_foreground, theme.border, theme.info);
        let mut area = div()
            .relative()
            .flex_1()
            .h_full()
            .bg(gpui::black())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|viewer, event: &MouseDownEvent, window, cx| {
                    viewer.pick(event.position, window, cx);
                }),
            )
            .on_mouse_move(cx.listener(|viewer, event: &MouseMoveEvent, window, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    viewer.pick(event.position, window, cx);
                }
            }));

        if self.columns.is_empty() {
            return area.child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(muted)
                    .child("Drop nifti files here, or use 'Open volume...'"),
            );
        }

        for (index, column) in self.columns.iter().enumerate() {
            for row in 0..3 {
                let layout = self.layout(index, row, window);
                let labels = column.display.labels(ViewPlane::ALL[row], false);
                let mut panel = div()
                    .absolute()
                    .left(px(layout.origin.x))
                    .top(px(layout.origin.y))
                    .w(px(layout.size.width))
                    .h(px(layout.size.height))
                    .flex()
                    .items_center()
                    .justify_center()
                    .overflow_hidden()
                    .text_sm();
                if let Some(image) = &column.images[row] {
                    panel = panel.child(
                        img(image.image.clone())
                            .w(px(layout.image_size.width))
                            .h(px(layout.image_size.height)),
                    );
                }

                let edge =
                    |text: &'static str| div().absolute().text_color(label_color).child(text);
                panel = panel
                    .child(div().absolute().inset_0().border_1().border_color(border))
                    .child(
                        v_flex()
                            .absolute()
                            .top_1()
                            .left_2()
                            .child(labels.title)
                            .when(row == 0, |title| {
                                title.child(
                                    div()
                                        .text_xs()
                                        .text_color(muted)
                                        .child(column.display.name()),
                                )
                            }),
                    )
                    .child(edge(labels.left).left_2())
                    .child(edge(labels.right).right_2())
                    .child(edge(labels.top).top_1())
                    .child(edge(labels.bottom).bottom_1());
                area = area.child(panel);
            }
        }
        area
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, sidebar) = (theme.muted_foreground, theme.border, theme.sidebar);
        let mut list = v_flex()
            .id("sidebar")
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_none()
            .p_3()
            .gap_3()
            .overflow_y_scroll()
            .bg(sidebar)
            .border_l_1()
            .border_color(border)
            .text_sm()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("open")
                            .primary()
                            .small()
                            .label("Open volume...")
                            .on_click(cx.listener(|viewer, _, _, cx| viewer.open_files(cx))),
                    )
                    .child(div().text_color(muted).child("or drop files here")),
            );

        // Loads that are still running, or that failed or were cancelled.
        let statuses = self.loader.statuses();
        let any_finished = statuses.iter().any(|status| !status.state.is_active());
        for status in statuses {
            if status.state == LoadState::Ready {
                continue;
            }
            let id = status.id;
            let name = status
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let detail = match &status.state {
                LoadState::Queued | LoadState::Loading => h_flex()
                    .gap_2()
                    .child(
                        Progress::new(("load-progress", id))
                            .value(status.progress as f32 * 100.0)
                            .flex_1(),
                    )
                    .child(
                        Button::new(("load-cancel", id))
                            .ghost()
                            .xsmall()
                            .label("Cancel")
                            .on_click(cx.listener(move |viewer, _, _, _| viewer.loader.cancel(id))),
                    )
                    .into_any_element(),
                LoadState::Failed(error) => div()
                    .text_color(cx.theme().danger)
                    .child(error.clone())
                    .into_any_element(),
                _ => div()
                    .text_color(muted)
                    .child(status.stage.clone())
                    .into_any_element(),
            };
            list = list.child(
                v_flex()
                    .gap_1()
                    .child(name)
                    .when(status.state.is_active(), |item| {
                        item.child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(status.stage.clone()),
                        )
                    })
                    .child(detail),
            );
        }
        if any_finished {
            list = list.child(
                Button::new("dismiss")
                    .ghost()
                    .xsmall()
                    .label("Clear finished loads")
                    .on_click(cx.listener(|viewer, _, _, cx| {
                        viewer.loader.dismiss_finished();
                        cx.notify();
                    })),
            );
        }
        for error in &self.errors {
            list = list.child(div().text_color(cx.theme().danger).child(error.clone()));
        }

        // summary of each volume and the value under the cursor
        list = list.child(
            div()
                .pt_2()
                .border_t_1()
                .border_color(border)
                .child(format!("Loaded volumes: {}", self.columns.len())),
        );
        for column in &self.columns {
            let display = &column.display;
            let h = display.volume().header();
            let cursor = match display.cursor_sample() {
                Some(([x, y, z], value)) => format!("Voxel {x}, {y}, {z} = {value:.6}"),
                None => "Cursor outside volume".to_string(),
            };
            list = list.child(
                v_flex()
                    .gap_1()
                    .child(display.name())
                    .child(div().text_xs().text_color(muted).child(format!(
                        "{} x {} x {} x {} | {}",
                        h.width(),
                        h.height(),
                        h.depth(),
                        h.timepoints(),
                        h.datatype.name()
                    )))
                    .child(div().text_xs().child(cursor)),
            );
        }
        list
    }
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.update_images(window, cx);
        let theme = cx.theme();
        let (background, foreground) = (theme.background, theme.foreground);
        h_flex()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .on_drop(cx.listener(|viewer, paths: &ExternalPaths, _, _| {
                for path in paths.paths() {
                    viewer.loader.enqueue(path.clone());
                }
            }))
            .child(self.render_panels(window, cx))
            .child(self.render_sidebar(cx))
    }
}

/// Slice to GPUI image (BGRA)
fn to_render_image(image: SliceImage) -> Arc<RenderImage> {
    let mut pixels = image.pixels;
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let buffer = RgbaImage::from_raw(image.width as u32, image.height as u32, pixels)
        .expect("a slice image has width * height * 4 btyes");
    Arc::new(RenderImage::new(vec![Frame::new(buffer)]))
}
