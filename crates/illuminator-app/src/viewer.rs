//! Main view

use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{ActiveTheme, Selectable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use illuminator_core::cache::VolumeCache;
use illuminator_core::display::VolumeDisplay;
use illuminator_core::geometry::{Point3D, SlicePlane, ViewPlane};
use illuminator_core::loader::{LoadState, VolumeLoader};
use illuminator_core::slicer::{Coloring, Colormap, SliceImage, fit_size};
use image::{Frame, RgbaImage};
use std::path::PathBuf;
use std::sync::Arc;

use crate::shortcuts::{self, *};

const SIDEBAR_WIDTH: f32 = 320.0;
const SCROLL_STEP: f32 = 20.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SyncMode {
    World,
    Proportional,
}

#[derive(Clone, Copy, PartialEq)]
struct ImageKey {
    plane: SlicePlane,
    coloring: Coloring,
    frame: usize,
    crosshair: Option<Point3D>,
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
    load_id: u64,
    error: Option<String>,
}

struct PanelLayout {
    origin: Point<f32>,
    size: Size<f32>,
    image_pixels: (usize, usize),
    image_size: Size<f32>,
}

struct Hit {
    column: usize,
    axis: ViewPlane,
    s: f64,
    t: f64,
}

struct FrameSlider {
    state: Entity<SliderState>,
    last_frame: usize,
    _changes: Subscription,
}

pub struct Viewer {
    cache: Arc<VolumeCache>,
    loader: VolumeLoader,
    columns: Vec<Column>,
    errors: Vec<String>,
    sync: bool,
    sync_mode: SyncMode,
    radiological: bool,
    show_crosshair: bool,
    frame: usize,
    frame_slider: Option<FrameSlider>,
    contrast_drag: Option<(usize, Point<Pixels>)>,
    scroll_remainder: f32,
    active_panel: (usize, ViewPlane),
    focus_handle: FocusHandle,
    _loader_events: Task<()>,
}

impl Viewer {
    pub fn new(
        cache: Arc<VolumeCache>,
        files: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (sender, mut changes) = mpsc::unbounded::<()>();
        let loader = VolumeLoader::new(Arc::clone(&cache), move || {
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
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self {
            cache,
            loader,
            columns: Vec::new(),
            errors: Vec::new(),
            sync: true,
            sync_mode: SyncMode::Proportional,
            radiological: false,
            show_crosshair: true,
            frame: 0,
            frame_slider: None,
            contrast_drag: None,
            scroll_remainder: 0.0,
            active_panel: (0, ViewPlane::Axial),
            focus_handle,
            _loader_events: loader_events,
        }
    }

    fn take_loaded(&mut self, cx: &mut Context<Self>) {
        let mut added = false;
        for loaded in self.loader.take_completed() {
            let load_id = loaded.id;
            match VolumeDisplay::new(loaded) {
                Ok(mut display) => {
                    let error = display.set_frame(self.frame).err().map(|e| e.to_string());
                    let at = self
                        .columns
                        .partition_point(|column| column.load_id < load_id);
                    if at <= self.active_panel.0 && self.active_panel.0 < self.columns.len() {
                        self.active_panel.0 += 1;
                    }
                    self.columns.insert(
                        at,
                        Column {
                            display,
                            images: [None, None, None],
                            load_id,
                            error,
                        },
                    );
                    added = true;
                }
                Err(error) => self.errors.push(error.to_string()),
            }
        }
        if added {
            self.update_frame_slider(cx);
            self.sync_from_first();
        }
        cx.notify();
    }

    fn active(&self) -> Option<(usize, ViewPlane)> {
        let (column, axis) = self.active_panel;
        (column < self.columns.len()).then_some((column, axis))
    }

    fn set_active_panel(
        &mut self,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(hit) = self.hit(position, window) {
            let panel = (hit.column, hit.axis);
            if panel != self.active_panel {
                self.active_panel = panel;
                cx.notify();
            }
        }
    }

    fn step_active_slice(&mut self, steps: f64, cx: &mut Context<Self>) {
        let Some((column, axis)) = self.active() else {
            return;
        };
        if self.columns[column].display.step_slice(axis, steps) {
            self.sync_from(column);
            cx.notify();
        }
    }

    fn step_frame(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(slider) = &self.frame_slider else {
            return;
        };
        let (state, last_frame) = (slider.state.clone(), slider.last_frame);
        let frame = self.frame.saturating_add_signed(delta).min(last_frame);
        if frame != self.frame {
            state.update(cx, |state, cx| state.set_value(frame as f32, window, cx));
            self.set_frame(frame, cx);
        }
    }

    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    fn open_files_action(&mut self, _: &OpenFiles, _: &mut Window, cx: &mut Context<Self>) {
        self.open_files(cx);
    }

    fn next_slice(&mut self, _: &NextSlice, _: &mut Window, cx: &mut Context<Self>) {
        self.step_active_slice(1.0, cx);
    }

    fn previous_slice(&mut self, _: &PreviousSlice, _: &mut Window, cx: &mut Context<Self>) {
        self.step_active_slice(-1.0, cx);
    }

    fn next_slices(&mut self, _: &NextSlices, _: &mut Window, cx: &mut Context<Self>) {
        self.step_active_slice(10.0, cx);
    }

    fn previous_slices(&mut self, _: &PreviousSlices, _: &mut Window, cx: &mut Context<Self>) {
        self.step_active_slice(-10.0, cx);
    }

    fn next_frame(&mut self, _: &NextFrame, window: &mut Window, cx: &mut Context<Self>) {
        self.step_frame(1, window, cx);
    }

    fn previous_frame(&mut self, _: &PreviousFrame, window: &mut Window, cx: &mut Context<Self>) {
        self.step_frame(-1, window, cx);
    }

    fn center_cursor(&mut self, _: &CenterCursor, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((column, _)) = self.active() {
            self.change(
                column,
                |display| {
                    display.reset_cursor();
                    Ok(())
                },
                cx,
            );
        }
    }

    fn auto_contrast(&mut self, _: &AutoContrast, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((column, _)) = self.active() {
            self.change(column, VolumeDisplay::auto_window, cx);
        }
    }

    fn next_colormap(&mut self, _: &NextColormap, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((column, _)) = self.active() {
            let display = &mut self.columns[column].display;
            let current = Colormap::ALL.iter().position(|&c| c == display.colormap);
            let next = current.map_or(0, |index| (index + 1) % Colormap::ALL.len());
            display.colormap = Colormap::ALL[next];
            cx.notify();
        }
    }

    fn toggle_segmentation(
        &mut self,
        _: &ToggleSegmentation,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((column, _)) = self.active() {
            let enabled = !self.columns[column].display.label_mode();
            self.change(column, |display| display.set_label_mode(enabled), cx);
        }
    }

    fn toggle_sync(&mut self, _: &ToggleSync, _: &mut Window, cx: &mut Context<Self>) {
        self.sync = !self.sync;
        self.sync_from_first();
        cx.notify();
    }

    fn toggle_radiological(
        &mut self,
        _: &ToggleRadiological,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.radiological = !self.radiological;
        cx.notify();
    }

    fn toggle_crosshair(&mut self, _: &ToggleCrosshair, _: &mut Window, cx: &mut Context<Self>) {
        self.show_crosshair = !self.show_crosshair;
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

    fn change(
        &mut self,
        column: usize,
        change: impl FnOnce(&mut VolumeDisplay) -> illuminator_core::Result<()>,
        cx: &mut Context<Self>,
    ) {
        let entry = &mut self.columns[column];
        entry.error = change(&mut entry.display).err().map(|e| e.to_string());
        self.sync_from(column);
        cx.notify();
    }

    fn sync_from(&mut self, active: usize) {
        if !self.sync {
            return;
        }
        let source = &self.columns[active].display;
        match self.sync_mode {
            SyncMode::World => {
                if !source.world_linked() {
                    return;
                }
                let cursor = source.cursor();
                for (index, column) in self.columns.iter_mut().enumerate() {
                    if index != active && column.display.world_linked() {
                        column.display.set_cursor(cursor);
                    }
                }
            }
            SyncMode::Proportional => {
                let fraction = source.fractional_cursor();
                for (index, column) in self.columns.iter_mut().enumerate() {
                    if index != active {
                        column.display.set_fractional_cursor(fraction);
                    }
                }
            }
        }
    }

    fn sync_from_first(&mut self) {
        let leader = self.columns.iter().position(|column| {
            self.sync_mode == SyncMode::Proportional || column.display.world_linked()
        });
        if let Some(leader) = leader {
            self.sync_from(leader);
        }
    }

    fn set_frame(&mut self, frame: usize, cx: &mut Context<Self>) {
        self.frame = frame;
        for column in &mut self.columns {
            column.error = column.display.set_frame(frame).err().map(|e| e.to_string());
        }
        cx.notify();
    }

    fn update_frame_slider(&mut self, cx: &mut Context<Self>) {
        let frames = self
            .columns
            .iter()
            .map(|column| column.display.volume().header().timepoints())
            .max()
            .unwrap_or(1);
        let last_frame = frames - 1;
        if last_frame == 0
            || self
                .frame_slider
                .as_ref()
                .is_some_and(|s| s.last_frame == last_frame)
        {
            return;
        }
        let frame = self.frame.min(last_frame) as f32;
        let state = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(last_frame as f32)
                .step(1.0)
                .default_value(frame)
        });
        let changes = cx.subscribe(&state, |viewer, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                viewer.set_frame(value.start().round() as usize, cx);
            }
        });
        self.frame_slider = Some(FrameSlider {
            state,
            last_frame,
            _changes: changes,
        });
    }

    fn layout(&self, column: usize, row: usize, window: &Window) -> PanelLayout {
        let viewport = window.viewport_size();
        let area_width = (f32::from(viewport.width) - SIDEBAR_WIDTH).max(0.0);
        let width = area_width / self.columns.len().max(1) as f32;
        let height = f32::from(viewport.height) / 3.0;
        let scale = window.scale_factor();
        let plane = self.columns[column]
            .display
            .plane(ViewPlane::ALL[row], self.radiological);
        let image_pixels = fit_size(&plane, (width * scale) as usize, (height * scale) as usize);
        PanelLayout {
            origin: point(column as f32 * width, row as f32 * height),
            size: size(width, height),
            image_pixels,
            image_size: size(image_pixels.0 as f32 / scale, image_pixels.1 as f32 / scale),
        }
    }

    fn hit(&self, position: Point<Pixels>, window: &Window) -> Option<Hit> {
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
                return Some(Hit {
                    column,
                    axis: ViewPlane::ALL[row],
                    s: f64::from((local_x - left) / layout.image_size.width),
                    t: f64::from(1.0 - (local_y - top) / layout.image_size.height),
                });
            }
        }
        None
    }

    fn pick(&mut self, position: Point<Pixels>, window: &Window, cx: &mut Context<Self>) {
        let Some(hit) = self.hit(position, window) else {
            return;
        };
        let radiological = self.radiological;
        if self.columns[hit.column]
            .display
            .pick(hit.axis, radiological, hit.s, hit.t)
        {
            self.sync_from(hit.column);
            cx.notify();
        }
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, window: &Window, cx: &mut Context<Self>) {
        let Some(hit) = self.hit(event.position, window) else {
            return;
        };
        self.scroll_remainder += f32::from(event.delta.pixel_delta(px(SCROLL_STEP)).y);
        let steps = (self.scroll_remainder / SCROLL_STEP).trunc();
        if steps == 0.0 {
            return;
        }
        self.scroll_remainder -= steps * SCROLL_STEP;
        if self.columns[hit.column]
            .display
            .step_slice(hit.axis, f64::from(steps))
        {
            self.sync_from(hit.column);
            cx.notify();
        }
    }

    fn drag_contrast(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((column, last)) = self.contrast_drag else {
            return;
        };
        let dx = f64::from(f32::from(position.x - last.x));
        let dy = f64::from(f32::from(position.y - last.y));
        self.columns[column].display.drag_window(dx, dy);
        self.contrast_drag = Some((column, position));
        cx.notify();
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
                    plane: display.plane(axis, self.radiological),
                    coloring: display.coloring(),
                    frame: display.frame(),
                    crosshair: self.show_crosshair.then_some(display.cursor()),
                    width,
                    height,
                };
                let current = &self.columns[column].images[row];
                if width == 0 || height == 0 || current.as_ref().is_some_and(|p| p.key == key) {
                    continue;
                }
                let slice = display.slice(axis, self.radiological, self.show_crosshair);
                let image = to_render_image(slice.render(width, height));
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
                    viewer.focus_handle.focus(window, cx);
                    viewer.set_active_panel(event.position, window, cx);
                    viewer.pick(event.position, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|viewer, event: &MouseDownEvent, window, _| {
                    viewer.contrast_drag = viewer
                        .hit(event.position, window)
                        .map(|hit| (hit.column, event.position));
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|viewer, _: &MouseUpEvent, _, _| viewer.contrast_drag = None),
            )
            .on_mouse_move(cx.listener(|viewer, event: &MouseMoveEvent, window, cx| {
                if event.pressed_button.is_none() {
                    viewer.set_active_panel(event.position, window, cx);
                }
                match event.pressed_button {
                    Some(MouseButton::Left) => viewer.pick(event.position, window, cx),
                    Some(MouseButton::Right) => viewer.drag_contrast(event.position, cx),
                    _ => {}
                }
            }))
            .on_scroll_wheel(cx.listener(|viewer, event: &ScrollWheelEvent, window, cx| {
                viewer.scroll(event, window, cx);
            }));

        if self.columns.is_empty() {
            return area.child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(muted)
                    .child("Drop scans here or use Open volume..."),
            );
        }

        for (index, column) in self.columns.iter().enumerate() {
            let display = &column.display;
            for row in 0..3 {
                let layout = self.layout(index, row, window);
                let labels = display.labels(ViewPlane::ALL[row], self.radiological);
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
                    .child(div().absolute().inset_0().border_1().border_color(
                        if self.active_panel == (index, ViewPlane::ALL[row]) {
                            label_color
                        } else {
                            border
                        },
                    ))
                    .child(
                        v_flex()
                            .absolute()
                            .top_1()
                            .left_2()
                            .child(labels.title)
                            .when(row == 0, |title| {
                                title.child(div().text_xs().text_color(muted).child(display.name()))
                            }),
                    )
                    .child(edge(labels.left).left_2())
                    .child(edge(labels.right).right_2())
                    .child(edge(labels.top).top_1())
                    .child(edge(labels.bottom).bottom_1())
                    .when(
                        !display.geometry().contains_world(display.cursor()),
                        |panel| {
                            panel.child(
                                div()
                                    .absolute()
                                    .bottom_1()
                                    .left_2()
                                    .text_color(label_color)
                                    .child("Cursor outside volume"),
                            )
                        },
                    );
                area = area.child(panel);
            }
        }
        area
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, sidebar) = (theme.muted_foreground, theme.border, theme.sidebar);
        let separator = || div().border_t_1().border_color(border);
        v_flex()
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
            .child(self.render_loading(cx))
            .child(separator())
            .child(self.render_view_controls(cx))
            .children((0..self.columns.len()).map(|index| {
                v_flex()
                    .gap_3()
                    .child(separator())
                    .child(self.render_volume(index, cx))
            }))
            .child(separator())
            .child(v_flex().gap_1().text_xs().text_color(muted).child(format!(
                "Max cache size: {:.1} GiB",
                self.cache.budget() as f64 / (1u64 << 30) as f64
            )))
    }

    fn render_loading(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        let mut list = v_flex().gap_3().child(
            h_flex().gap_2().child(
                Button::new("open")
                    .primary()
                    .small()
                    .label("Open volume(s)")
                    .on_click(cx.listener(|viewer, _, _, cx| viewer.open_files(cx))),
            ),
        );
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
                    .text_color(danger)
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
            list = list.child(div().text_color(danger).child(error.clone()));
        }
        if !self.errors.is_empty() {
            list = list.child(
                Button::new("dismiss-errors")
                    .ghost()
                    .xsmall()
                    .label("Dismiss load errors")
                    .on_click(cx.listener(|viewer, _, _, cx| {
                        viewer.errors.clear();
                        cx.notify();
                    })),
            );
        }
        list
    }

    /// Settings shared by all volumes
    fn render_view_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mode_button = |id: &'static str, label: &'static str, mode: SyncMode| {
            Button::new(id)
                .xsmall()
                .label(label)
                .selected(self.sync_mode == mode)
                .on_click(cx.listener(move |viewer, _, _, cx| {
                    viewer.sync_mode = mode;
                    viewer.sync_from_first();
                    cx.notify();
                }))
        };
        v_flex()
            .gap_2()
            .child(format!("Loaded volumes: {}", self.columns.len()))
            .child(
                Checkbox::new("sync")
                    .label("Sync crosshairs")
                    .checked(self.sync)
                    .on_click(cx.listener(|viewer, checked: &bool, _, cx| {
                        viewer.sync = *checked;
                        viewer.sync_from_first();
                        cx.notify();
                    })),
            )
            .when(self.sync, |controls| {
                controls.child(
                    h_flex()
                        .gap_1()
                        .child(mode_button(
                            "sync-world",
                            "World (aligned)",
                            SyncMode::World,
                        ))
                        .child(mode_button(
                            "sync-proportional",
                            "Proportional",
                            SyncMode::Proportional,
                        )),
                )
            })
            .child(
                Checkbox::new("radiological")
                    .label("Radiological convention")
                    .checked(self.radiological)
                    .on_click(cx.listener(|viewer, checked: &bool, _, cx| {
                        viewer.radiological = *checked;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("crosshair")
                    .label("Show crosshairs")
                    .checked(self.show_crosshair)
                    .on_click(cx.listener(|viewer, checked: &bool, _, cx| {
                        viewer.show_crosshair = *checked;
                        cx.notify();
                    })),
            )
            .when_some(self.frame_slider.as_ref(), |controls, slider| {
                controls
                    .child(format!(
                        "Global frame: {} of {}",
                        self.frame, slider.last_frame
                    ))
                    .child(Slider::new(&slider.state))
            })
            .child(
                v_flex()
                    .text_xs()
                    .children(shortcuts::help().into_iter().map(|(keys, action)| {
                        h_flex()
                            .gap_2()
                            .child(div().w(px(90.0)).child(keys))
                            .child(action)
                    })),
            )
    }

    /// The information and controls of one volume
    fn render_volume(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        let column = &self.columns[index];
        let display = &column.display;
        let geometry = display.geometry();
        let h = display.volume().header();
        let id = |name: &'static str| (name, index as u64);

        let cursor_text = match display.cursor_sample() {
            Some(([x, y, z], value)) if display.label_mode() => {
                format!("{x}, {y}, {z}\nLabel: {value}")
            }
            Some(([x, y, z], value)) => format!("{x}, {y}, {z}\nValue: {value}"),
            None => "Outside volume".to_string(),
        };

        let mut section = v_flex()
            .gap_2()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("Volume {}", index + 1)),
            )
            .child(display.name())
            .child(div().text_xs().text_color(muted).child(format!(
                "{} x {} x {} | {}",
                h.width(),
                h.height(),
                h.depth(),
                h.datatype.name()
            )))
            .child(div().text_xs().text_color(muted).child(format!(
                "Frame {} of {} | sform {} | qform {}",
                display.frame() + 1,
                h.timepoints(),
                h.sform_code,
                h.qform_code
            )))
            .when(display.units_unknown(), |section| {
                section.child(
                    Checkbox::new(id("assume-mm"))
                        .label("Assume mm")
                        .checked(display.options().assume_mm)
                        .on_click(cx.listener(move |viewer, checked: &bool, _, cx| {
                            let checked = *checked;
                            viewer.change(index, |display| display.set_assume_mm(checked), cx);
                        })),
                )
            })
            .when(display.can_be_anatomical(), |section| {
                section.child(
                    Checkbox::new(id("native"))
                        .label("Native voxel axes")
                        .checked(display.options().native)
                        .on_click(cx.listener(move |viewer, checked: &bool, _, cx| {
                            let checked = *checked;
                            viewer.change(index, |display| display.set_native(checked), cx);
                        })),
                )
            })
            .child(div().text_xs().text_color(muted).child(format!(
                "{} units: {}",
                geometry.transform_name, geometry.unit_name
            )))
            .when(
                self.sync && self.sync_mode == SyncMode::World && geometry.anatomical,
                |section| {
                    section.child(
                        Checkbox::new(id("link"))
                            .label("Link in world space")
                            .checked(display.link_world)
                            .on_click(cx.listener(move |viewer, checked: &bool, _, cx| {
                                let checked = *checked;
                                viewer.change(
                                    index,
                                    |display| {
                                        display.link_world = checked;
                                        Ok(())
                                    },
                                    cx,
                                );
                            })),
                    )
                },
            )
            .when(geometry.anatomical, |section| {
                let cursor = display.cursor();
                section.child(format!(
                    "RAS [mm]: {:.3}, {:.3}, {:.3}",
                    cursor.x, cursor.y, cursor.z
                ))
            })
            .child(cursor_text)
            .child(
                Button::new(id("center"))
                    .xsmall()
                    .label("Center cursor")
                    .on_click(cx.listener(move |viewer, _, _, cx| {
                        viewer.change(
                            index,
                            |display| {
                                display.reset_cursor();
                                Ok(())
                            },
                            cx,
                        );
                    })),
            )
            .child(
                Checkbox::new(id("labels"))
                    .label("Segmentation map")
                    .checked(display.label_mode())
                    .on_click(cx.listener(move |viewer, checked: &bool, _, cx| {
                        let checked = *checked;
                        viewer.change(index, |display| display.set_label_mode(checked), cx);
                    })),
            );

        section = if display.label_mode() {
            section.child(format!("{} labels", display.label_count()))
        } else {
            let window = display.window;
            section
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new(id("auto"))
                                .xsmall()
                                .label("Auto current frame")
                                .on_click(cx.listener(move |viewer, _, _, cx| {
                                    viewer.change(index, VolumeDisplay::auto_window, cx);
                                })),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child(format!("L: {:.6}  W: {:.6}", window.level, window.width)),
                        ),
                )
                .when(display.range().is_none(), |section| {
                    section.child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("No finite values in measured frame!"),
                    )
                })
                .child(h_flex().gap_1().flex_wrap().children(
                    Colormap::ALL.into_iter().enumerate().map(|(n, colormap)| {
                        Button::new(("colormap", (index * Colormap::ALL.len() + n) as u64))
                            .xsmall()
                            .label(colormap.name())
                            .selected(display.colormap == colormap)
                            .on_click(cx.listener(move |viewer, _, _, cx| {
                                viewer.columns[index].display.colormap = colormap;
                                cx.notify();
                            }))
                    }),
                ))
        };

        section.when_some(column.error.clone(), |section, error| {
            section.child(div().text_color(danger).child(error)).child(
                Button::new(id("dismiss-error"))
                    .ghost()
                    .xsmall()
                    .label("Dismiss error")
                    .on_click(cx.listener(move |viewer, _, _, cx| {
                        viewer.columns[index].error = None;
                        cx.notify();
                    })),
            )
        })
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
            .track_focus(&self.focus_handle)
            .key_context(shortcuts::CONTEXT)
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(Self::open_files_action))
            .on_action(cx.listener(Self::next_slice))
            .on_action(cx.listener(Self::previous_slice))
            .on_action(cx.listener(Self::next_slices))
            .on_action(cx.listener(Self::previous_slices))
            .on_action(cx.listener(Self::next_frame))
            .on_action(cx.listener(Self::previous_frame))
            .on_action(cx.listener(Self::center_cursor))
            .on_action(cx.listener(Self::auto_contrast))
            .on_action(cx.listener(Self::next_colormap))
            .on_action(cx.listener(Self::toggle_segmentation))
            .on_action(cx.listener(Self::toggle_sync))
            .on_action(cx.listener(Self::toggle_radiological))
            .on_action(cx.listener(Self::toggle_crosshair))
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
