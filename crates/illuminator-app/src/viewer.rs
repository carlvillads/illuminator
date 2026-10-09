//! Main view

use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::chart::LineChart;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::hover_card::HoverCard;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Sizable, WindowExt, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use illuminator_core::cache::VolumeCache;
use illuminator_core::display::{SampleKey, VolumeDisplay};
use illuminator_core::geometry::{Point3D, ViewPlane};
use illuminator_core::histogram::Histogram;
use illuminator_core::labels::label_color;
use illuminator_core::loader::{LoadState, VolumeLoader};
use illuminator_core::slicer::{Coloring, Colormap, SampledSlice, SliceImage, fit_size};
use illuminator_core::suv::SuvInputs;
use illuminator_core::volume::Volume;
use image::{Frame, RgbaImage};
use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::assets::Carbon;
use crate::shortcuts::{self, *};
use crate::statistics::{Snapshot, StatisticsView};
use crate::suv_form::SuvForm;
use crate::zoom::{Visible, Zoom};

const SIDEBAR_WIDTH: f32 = 240.0;
const SCROLL_STEP: f32 = 20.0;
const OVERLAY_OPACITY: f32 = 0.5;
const TOOLBAR_HEIGHT: f32 = 36.0;
const STATUS_HEIGHT: f32 = 24.0;
const CORNER_TEXT: f32 = 10.0;
const HISTOGRAM_BINS: usize = 128;
const HISTOGRAM_HEIGHT: f32 = 56.0;
const ZOOM_SPEED: f64 = 200.0;
const CHART_CURVE_HEIGHT: f32 = 64.0;
const TIMEPOINT_FRAME_RATES: [u32; 5] = [1, 2, 5, 10, 20];
const PRESETS: &[(&str, &[(&str, Preset)])] = &[
    (
        "CT (HU)",
        &[
            ("Brain  W 80 L 40", Preset::Limits(0.0, 80.0)),
            ("Subdural  W 200 L 75", Preset::Limits(-25.0, 175.0)),
            ("Stroke  W 40 L 40", Preset::Limits(20.0, 60.0)),
            ("Soft tissue  W 400 L 50", Preset::Limits(-150.0, 250.0)),
            ("Lung  W 1500 L -600", Preset::Limits(-1350.0, 150.0)),
            ("Bone  W 1800 L 400", Preset::Limits(-500.0, 1300.0)),
        ],
    ),
    (
        "PET (SUV)",
        &[
            ("0 to 5", Preset::Limits(0.0, 5.0)),
            ("0 to 10", Preset::Limits(0.0, 10.0)),
        ],
    ),
    (
        "Histogram",
        &[
            ("Full range", Preset::Percentiles(0.0, 1.0)),
            ("1 to 99 %", Preset::Percentiles(0.01, 0.99)),
            ("5 to 95 %", Preset::Percentiles(0.05, 0.95)),
        ],
    ),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum SyncMode {
    World,
    Proportional,
}

#[derive(Clone, PartialEq)]
struct ImageKey {
    sample: SampleKey,
    coloring: Coloring,
}

struct PanelImage {
    key: ImageKey,
    image: Arc<RenderImage>,
}

#[derive(Default)]
struct Panel {
    image: Option<PanelImage>,
    samples: Option<(SampleKey, Arc<SampledSlice>)>,
    rendering: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Preset {
    Limits(f64, f64),
    Percentiles(f64, f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowEdge {
    Low,
    High,
}

#[derive(Debug, Clone, Copy)]
struct DragEdge {
    load_id: u64,
    edge: WindowEdge,
}

impl Render for DragEdge {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Clone)]
struct DragLayer {
    index: usize,
    name: SharedString,
}

impl Render for DragLayer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(cx.theme().secondary)
            .text_xs()
            .child(self.name.clone())
    }
}

struct Layer {
    display: VolumeDisplay,
    panels: [Panel; 3],
    histogram: Option<Histogram>,
    histogram_frame: Option<usize>,
    histogram_pending: bool,
    curve: Option<TimeCurve>,
    curve_pending: bool,
    suv: Option<(SuvInputs, &'static str)>,
    load_id: u64,
    visible: bool,
    opacity: f32,
    opacity_slider: Entity<SliderState>,
    _opacity_changes: Subscription,
}

/// One visible image panel
struct View {
    origin: Point<f32>,
    size: Size<f32>,
    axis: ViewPlane,
    base: usize,
    layers: Range<usize>,
    image_pixels: (usize, usize),
    image_size: Size<f32>,
    zoom: Zoom,
    fit: (f64, f64),
    visible: Visible,
}

struct TimeCurve {
    volume: u64,
    voxel: [usize; 3],
    values: Vec<f64>,
}

#[derive(Clone, Copy)]
struct PanDrag {
    axis: ViewPlane,
    last: Point<Pixels>,
    image_size: Size<f32>,
    fit: (f64, f64),
}

struct Hit {
    base: usize,
    axis: ViewPlane,
    s: f64,
    t: f64,
    in_view: (f64, f64),
    image_size: Size<f32>,
    fit: (f64, f64),
}

struct FrameSlider {
    state: Entity<SliderState>,
    last_frame: usize,
    _changes: Subscription,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InfoLevel {
    /// Only images
    Off,
    /// letters + slice number
    Minimal,
    Full,
}

impl InfoLevel {
    fn next(self) -> Self {
        match self {
            Self::Off => Self::Minimal,
            Self::Minimal => Self::Full,
            Self::Full => Self::Off,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Minimal => "minimal",
            Self::Full => "all",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    Ortho,
    Compare,
    Grid(ViewPlane),
    Single {
        axis: ViewPlane,
        layer: Option<usize>,
    },
}

impl Layout {
    fn overlaid(self) -> bool {
        matches!(self, Self::Ortho | Self::Single { layer: None, .. })
    }

    fn axis(self) -> Option<ViewPlane> {
        match self {
            Self::Grid(axis) | Self::Single { axis, .. } => Some(axis),
            Self::Ortho | Self::Compare => None,
        }
    }

    fn with_axis(self, axis: ViewPlane) -> Self {
        match self {
            Self::Grid(_) => Self::Grid(axis),
            Self::Single { layer, .. } => Self::Single { axis, layer },
            other => other,
        }
    }

    fn same_kind(self, other: Self) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}

pub struct Viewer {
    cache: Arc<VolumeCache>,
    loader: VolumeLoader,
    layers: Vec<Layer>,
    sync: bool,
    sync_mode: SyncMode,
    radiological: bool,
    show_crosshair: bool,
    frame: usize,
    frame_slider: Option<FrameSlider>,
    time_playing: Option<Task<()>>,
    time_fps: u32,
    time_curve_bounds: Rc<Cell<Bounds<Pixels>>>,
    contrast_drag: Option<(usize, Point<Pixels>)>,
    zoom: [Zoom; 3],
    pan_drag: Option<PanDrag>,
    scroll_remainder: f32,
    active_panel: (usize, ViewPlane),
    focus_handle: FocusHandle,
    layout: Layout,
    previous_layout: Layout,
    hover: Option<Point3D>,
    selected: usize,
    info: InfoLevel,
    reloads: HashMap<u64, u64>,
    retired: Vec<Arc<RenderImage>>,
    _loader_events: Task<()>,
    _open_requests: Task<()>,
}

impl Viewer {
    pub fn new(
        cache: Arc<VolumeCache>,
        files: Vec<PathBuf>,
        mut opened: mpsc::UnboundedReceiver<Vec<PathBuf>>,
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
        let open_requests = cx.spawn_in(window, async move |this, cx| {
            while let Some(paths) = opened.next().await {
                let delivered = this.update_in(cx, |viewer, window, _| {
                    for path in paths {
                        viewer.loader.enqueue(path);
                    }
                    // brings the window to the front.
                    // macOS note: it does that itself for files from Finder and the Dock, we shouldnt force it
                    if cfg!(not(target_os = "macos")) {
                        window.activate_window();
                    }
                });
                if delivered.is_err() {
                    break;
                }
            }
        });
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self {
            cache,
            loader,
            layers: Vec::new(),
            sync: true,
            sync_mode: SyncMode::World,
            radiological: false,
            show_crosshair: true,
            frame: 0,
            frame_slider: None,
            time_playing: None,
            time_fps: 5,
            time_curve_bounds: Rc::default(),
            contrast_drag: None,
            zoom: Default::default(),
            pan_drag: None,
            scroll_remainder: 0.0,
            active_panel: (0, ViewPlane::Axial),
            focus_handle,
            layout: Layout::Ortho,
            previous_layout: Layout::Ortho,
            hover: None,
            selected: 0,
            info: InfoLevel::Minimal,
            reloads: HashMap::new(),
            retired: Vec::new(),
            _loader_events: loader_events,
            _open_requests: open_requests,
        }
    }

    fn take_loaded(&mut self, cx: &mut Context<Self>) {
        let mut added = false;
        for loaded in self.loader.take_completed() {
            let load_id = loaded.id;
            let reload_of = self.reloads.remove(&load_id);
            match VolumeDisplay::new(loaded) {
                Ok(mut display) => {
                    if let Some(old_id) = reload_of {
                        self.replace_layer(old_id, display);
                        added = true;
                        continue;
                    }
                    if let Err(error) = display.set_frame(self.frame) {
                        eprintln!("Illuminator: {}: {error}", display.name());
                    }
                    display.clip_low = true;
                    let at = self
                        .layers
                        .iter()
                        .rposition(|layer| layer.load_id < load_id)
                        .map_or(0, |index| index + 1);
                    self.reindex(|index| Some(if index >= at { index + 1 } else { index }));
                    let suv = match SuvInputs::from_sidecar(display.volume().path()) {
                        Some(Ok(inputs)) => Some((inputs, "BIDS sidecar")),
                        Some(Err(error)) => {
                            eprintln!("Illuminator: {error}");
                            None
                        }
                        None => None,
                    };
                    let (opacity_slider, opacity_changes) = opacity_slider(load_id, cx);
                    self.layers.insert(
                        at,
                        Layer {
                            display,
                            panels: Default::default(),
                            histogram: None,
                            histogram_frame: None,
                            histogram_pending: false,
                            curve: None,
                            curve_pending: false,
                            suv,
                            load_id,
                            visible: true,
                            opacity: OVERLAY_OPACITY,
                            opacity_slider,
                            _opacity_changes: opacity_changes,
                        },
                    );
                    added = true;
                }
                Err(error) => eprintln!("Illuminator: {error}"),
            }
        }
        // report failed loads once, then drop all finished loads, so the sidebar only lists running ones
        for status in self.loader.statuses() {
            if let LoadState::Failed(error) = &status.state {
                eprintln!("Illuminator: {}: {error}", status.path.display());
            }
        }
        self.loader.dismiss_finished();
        if added {
            self.update_frame_slider(cx);
            self.sync_from_first();
        }
        cx.notify();
    }

    fn active(&self) -> Option<(usize, ViewPlane)> {
        let (column, axis) = self.active_panel;
        (column < self.layers.len()).then_some((column, axis))
    }

    fn selected(&self) -> Option<usize> {
        (self.selected < self.layers.len()).then_some(self.selected)
    }

    fn select(&mut self, layer: usize, cx: &mut Context<Self>) {
        if layer < self.layers.len() {
            self.selected = layer;
            if !self.layout.overlaid() {
                self.active_panel.0 = layer;
            }
            cx.notify();
        }
    }

    fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        if !matches!(self.layout, Layout::Single { .. }) {
            self.previous_layout = self.layout;
        }
        self.layout = layout;
        let base = match layout {
            Layout::Single {
                layer: Some(layer), ..
            } => layer,
            _ if layout.overlaid() => 0,
            _ => self.selected,
        };
        self.active_panel = (base, layout.axis().unwrap_or(self.active_panel.1));
        self.sync_from_first();
        cx.notify();
    }

    fn toggle_maximize(
        &mut self,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.layout, Layout::Single { .. }) {
            self.set_layout(self.previous_layout, cx);
        } else if let Some(hit) = self.hit(position, window) {
            let layer = (!self.layout.overlaid()).then_some(hit.base);
            self.set_layout(
                Layout::Single {
                    axis: hit.axis,
                    layer,
                },
                cx,
            );
        }
    }

    /// Remembers the world position under the mouse
    fn set_hover(&mut self, position: Point<Pixels>, window: &Window, cx: &mut Context<Self>) {
        let hover = self
            .hit(position, window)
            .filter(|hit| (0.0..1.0).contains(&hit.s) && (0.0..1.0).contains(&hit.t))
            .map(|hit| {
                let display = &self.layers[hit.base].display;
                display
                    .plane(hit.axis, self.radiological)
                    .point(hit.s, hit.t)
            });
        if hover != self.hover {
            self.hover = hover;
            cx.notify();
        }
    }

    fn set_active_panel(
        &mut self,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(hit) = self.hit(position, window) {
            let panel = (hit.base, hit.axis);
            if panel != self.active_panel {
                if !self.layout.overlaid() {
                    self.selected = hit.base;
                }
                self.active_panel = panel;
                cx.notify();
            }
        }
    }

    fn step_active_slice(&mut self, steps: f64, cx: &mut Context<Self>) {
        let Some((column, axis)) = self.active() else {
            return;
        };
        if self.layers[column].display.step_slice(axis, steps) {
            self.sync_from(column);
            cx.notify();
        }
    }

    fn step_frame(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to_frame(self.frame.saturating_add_signed(delta), window, cx);
    }

    fn go_to_frame(&mut self, frame: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(slider) = &self.frame_slider else {
            return;
        };
        let frame = frame.min(slider.last_frame);
        if frame != self.frame {
            let state = slider.state.clone();
            state.update(cx, |state, cx| state.set_value(frame as f32, window, cx));
            self.set_frame(frame, cx);
        }
    }

    fn toggle_play(&mut self, _: &TogglePlay, window: &mut Window, cx: &mut Context<Self>) {
        if self.time_playing.take().is_none() {
            self.play(window, cx);
        }
        cx.notify();
    }

    fn play(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.frame_slider.is_none() {
            return;
        }
        let interval = Duration::from_secs_f64(1.0 / f64::from(self.time_fps));
        self.time_playing = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                let Ok((next, volumes)) =
                    this.update_in(cx, |viewer, window, _| viewer.next_movie_frame(window))
                else {
                    break; // viewer is gone here
                };
                let timer = cx.background_executor().timer(interval);
                let read_ahead = cx.background_spawn(async move {
                    for volume in volumes {
                        if let Err(error) = volume.read_ahead(next) {
                            eprintln!("Illuminator: reading frame {}: {error}", next + 1);
                        }
                    }
                });
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(5))
                        .await;
                    match this.update_in(cx, |viewer, window, _| viewer.frame_drawn(window)) {
                        Ok(true) => break,
                        Ok(false) => continue,
                        Err(_) => return, // viewer is gone here
                    }
                }
                futures::join!(timer, read_ahead);
                let stepped = this.update_in(cx, |viewer, window, cx| {
                    viewer.go_to_frame(next, window, cx);
                });
                if stepped.is_err() {
                    break;
                }
            }
        }));
    }

    fn next_movie_frame(&self, window: &Window) -> (usize, Vec<Arc<Volume>>) {
        let last = self.frame_slider.as_ref().map_or(0, |s| s.last_frame);
        let next = if self.frame >= last {
            0
        } else {
            self.frame + 1
        };
        let mut shown_sagittally: Vec<usize> = self
            .views(window)
            .iter()
            .filter(|view| view.axis == ViewPlane::Sagittal)
            .flat_map(|view| view.layers.clone())
            .collect();
        shown_sagittally.sort_unstable();
        shown_sagittally.dedup();
        let volumes = shown_sagittally
            .into_iter()
            .map(|index| &self.layers[index])
            .filter(|layer| layer.visible && layer.display.volume().meta().timepoints() > 1)
            .map(|layer| layer.display.shared_volume())
            .collect();
        (next, volumes)
    }

    fn frame_drawn(&self, window: &Window) -> bool {
        self.views(window).iter().all(|view| {
            view.layers.clone().all(|index| {
                let layer = &self.layers[index];
                let panel = &layer.panels[view.axis.index()];
                !layer.visible
                    || (!panel.rendering
                        && panel
                            .image
                            .as_ref()
                            .is_some_and(|image| image.key.sample.frame == layer.display.frame()))
            })
        })
    }

    fn set_frames_per_second(&mut self, rate: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.time_fps = rate;
        if self.time_playing.is_some() {
            self.play(window, cx);
        }
        cx.notify();
    }

    fn pick_frame(&mut self, x: Pixels, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.selected() else {
            return;
        };
        let bounds = self.time_curve_bounds.get();
        let frames = self.layers[index].display.volume().meta().timepoints();
        if bounds.size.width <= px(0.0) || frames < 2 {
            return;
        }
        let fraction = ((x - bounds.left()) / bounds.size.width).clamp(0.0, 1.0);
        let frame = (fraction * (frames - 1) as f32).round() as usize;
        self.go_to_frame(frame, window, cx);
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
        if let Some(column) = self.selected() {
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
        if let Some(index) = self.selected() {
            self.auto_window(index, cx);
        }
    }

    fn auto_window(&mut self, index: usize, cx: &mut Context<Self>) {
        let layer = &mut self.layers[index];
        let frame = layer.display.frame();
        let counted = layer.histogram.as_ref().filter(|histogram| {
            layer.histogram_frame == Some(frame) && histogram.max > histogram.min
        });
        match counted {
            Some(histogram) => {
                let (low, high) = (histogram.min, histogram.max);
                layer.display.set_window_limits(low, high);
                cx.notify();
            }
            None => self.change(index, VolumeDisplay::auto_window, cx),
        }
    }

    fn next_colormap(&mut self, _: &NextColormap, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(column) = self.selected() {
            let display = &mut self.layers[column].display;
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
        if let Some(column) = self.selected() {
            let enabled = !self.layers[column].display.label_mode();
            self.change(column, |display| display.set_label_mode(enabled), cx);
        }
    }

    fn reindex(&mut self, new_index: impl Fn(usize) -> Option<usize>) {
        let count = self.layers.len();
        if self.selected < count {
            // a removed selected layer passes the selection to the one before it
            self.selected = new_index(self.selected).unwrap_or(self.selected.saturating_sub(1));
        }
        if !self.layout.overlaid() && self.active_panel.0 < count {
            self.active_panel.0 = new_index(self.active_panel.0).unwrap_or(self.selected);
        }
        if let Layout::Single {
            axis,
            layer: Some(layer),
        } = self.layout
            && layer < count
        {
            self.layout = match new_index(layer) {
                Some(layer) => Layout::Single {
                    axis,
                    layer: Some(layer),
                },
                None => self.previous_layout,
            };
        }
    }

    fn close_layer(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.layers.len() {
            return;
        }
        self.reindex(|other| match other.cmp(&index) {
            Ordering::Less => Some(other),
            Ordering::Equal => None,
            Ordering::Greater => Some(other - 1),
        });
        let layer = self.layers.remove(index);
        self.retire(layer.panels);
        self.update_frame_slider(cx);
        self.sync_from_first();
        cx.notify();
    }

    fn move_layer(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let count = self.layers.len();
        if from >= count || to >= count || from == to {
            return;
        }
        self.reindex(|index| Some(moved(index, from, to)));
        let layer = self.layers.remove(from);
        self.layers.insert(to, layer);
        self.sync_from_first();
        cx.notify();
    }

    fn reload_layer(&mut self, index: usize) {
        let Some(layer) = self.layers.get(index) else {
            return;
        };
        let path = layer.display.volume().path().to_path_buf();
        let load_id = self.loader.enqueue(path);
        self.reloads.insert(load_id, layer.load_id);
    }

    fn replace_layer(&mut self, load_id: u64, mut display: VolumeDisplay) {
        let Some(layer) = self.layers.iter_mut().find(|l| l.load_id == load_id) else {
            return;
        };
        if let Err(error) = display.keep_settings_of(&layer.display) {
            eprintln!("Illuminator: {}: {error}", display.name());
        }
        layer.display = display;
        layer.histogram = None;
        layer.histogram_frame = None;
        layer.histogram_pending = false;
        let panels = std::mem::take(&mut layer.panels);
        self.retire(panels);
    }

    fn retire(&mut self, panels: [Panel; 3]) {
        self.retired.extend(
            panels
                .into_iter()
                .filter_map(|panel| panel.image)
                .map(|image| image.image),
        );
    }

    fn close_selected(&mut self, _: &CloseLayer, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.selected() {
            self.close_layer(index, cx);
        }
    }

    fn reload_selected(&mut self, _: &ReloadLayer, _: &mut Window, _: &mut Context<Self>) {
        if let Some(index) = self.selected() {
            self.reload_layer(index);
        }
    }

    fn move_selected_up(&mut self, _: &MoveLayerUp, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.selected()
            && index > 0
        {
            self.move_layer(index, index - 1, cx);
        }
    }

    fn move_selected_down(&mut self, _: &MoveLayerDown, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.selected() {
            self.move_layer(index, index + 1, cx);
        }
    }

    fn toggle_outline(&mut self, _: &ToggleOutline, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.selected() {
            let display = &mut self.layers[index].display;
            display.outline = !display.outline;
            cx.notify();
        }
    }

    fn toggle_overlay(&mut self, _: &ToggleOverlay, _: &mut Window, cx: &mut Context<Self>) {
        let layout = if self.layout.overlaid() {
            Layout::Compare
        } else {
            Layout::Ortho
        };
        self.set_layout(layout, cx);
    }

    fn layout_ortho(&mut self, _: &LayoutOrtho, _: &mut Window, cx: &mut Context<Self>) {
        self.set_layout(Layout::Ortho, cx);
    }

    fn layout_compare(&mut self, _: &LayoutCompare, _: &mut Window, cx: &mut Context<Self>) {
        self.set_layout(Layout::Compare, cx);
    }

    fn layout_grid(&mut self, _: &LayoutGrid, _: &mut Window, cx: &mut Context<Self>) {
        let axis = self.layout.axis().unwrap_or(self.active_panel.1);
        self.set_layout(Layout::Grid(axis), cx);
    }

    fn layout_single(&mut self, _: &LayoutSingle, _: &mut Window, cx: &mut Context<Self>) {
        let axis = self.layout.axis().unwrap_or(self.active_panel.1);
        self.set_layout(Layout::Single { axis, layer: None }, cx);
    }

    fn toggle_layer(&mut self, _: &ToggleLayer, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(layer) = self.selected() {
            let column = &mut self.layers[layer];
            column.visible = !column.visible;
            cx.notify();
        }
    }

    fn next_layer(&mut self, _: &NextLayer, _: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.is_empty() {
            self.select((self.selected + 1) % self.layers.len(), cx);
        }
    }

    fn previous_layer(&mut self, _: &PreviousLayer, _: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.is_empty() {
            let count = self.layers.len();
            self.select((self.selected + count - 1) % count, cx);
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
        self.zoom = Default::default();
        cx.notify();
    }

    fn toggle_info(&mut self, _: &ToggleInfo, _: &mut Window, cx: &mut Context<Self>) {
        self.info = self.info.next();
        cx.notify();
    }

    fn show_help(&mut self, _: &ShowHelp, window: &mut Window, cx: &mut Context<Self>) {
        let cache = format!(
            "Decompressed volumes are cached in {} >= {:.1} GiB",
            self.cache.directory().display(),
            self.cache.budget() as f64 / (1u64 << 30) as f64
        );
        window.open_dialog(cx, move |dialog, _, cx| {
            let theme = cx.theme();
            let (muted, border) = (theme.muted_foreground, theme.border);
            dialog.title("Shortcuts").width(px(440.0)).child(
                v_flex()
                    .gap_1()
                    .text_sm()
                    .children(shortcuts::help().into_iter().map(|(keys, action)| {
                        h_flex()
                            .gap_3()
                            .child(
                                div().w(px(120.0)).child(
                                    div()
                                        .px_1()
                                        .rounded_sm()
                                        .border_1()
                                        .border_color(border)
                                        .text_xs()
                                        .child(keys),
                                ),
                            )
                            .child(action)
                    }))
                    .child(
                        div()
                            .pt_3()
                            .text_xs()
                            .text_color(muted)
                            .child(cache.clone()),
                    ),
            )
        });
    }

    fn set_suv(&mut self, index: usize, on: bool, cx: &mut Context<Self>) {
        let layer = &mut self.layers[index];
        let scale = match (on, layer.suv) {
            (false, _) => 1.0,
            (true, Some((inputs, _))) => match inputs.factor() {
                Ok(factor) => factor,
                Err(error) => {
                    eprintln!("Illuminator: {error}");
                    return;
                }
            },
            (true, None) => return,
        };
        if let Err(error) = layer.display.set_scale(scale) {
            eprintln!("Illuminator: {}: {error}", layer.display.name());
        }
        layer.histogram_frame = None;
        cx.notify();
    }

    pub(crate) fn set_suv_inputs(
        &mut self,
        load_id: u64,
        inputs: SuvInputs,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self
            .layers
            .iter()
            .position(|layer| layer.load_id == load_id)
        {
            self.layers[index].suv = Some((inputs, "manual entry"));
            self.set_suv(index, true, cx);
        }
    }

    fn edit_suv(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let layer = &self.layers[index];
        let (load_id, known) = (layer.load_id, layer.suv.map(|(inputs, _)| inputs));
        let viewer = cx.entity().downgrade();
        let form = cx.new(|cx| SuvForm::new(load_id, known, viewer, window, cx));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("SUV (bw)")
                .width(px(420.0))
                .child(form.clone())
        });
    }

    fn show_statistics(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let snapshot = |layer: &Layer| Snapshot {
            name: layer.display.name().into(),
            volume: layer.display.shared_volume(),
            geometry: layer.display.geometry().clone(),
            frame: layer.display.frame(),
        };
        let labels = snapshot(&self.layers[index]);
        let candidates = self
            .layers
            .iter()
            .enumerate()
            .filter(|&(other, layer)| other != index && !layer.display.label_mode())
            .map(|(_, layer)| snapshot(layer))
            .collect();
        let view = cx.new(|cx| StatisticsView::new(labels, candidates, cx));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Label statistics")
                .width(px(700.0))
                .child(view.clone())
        });
    }

    fn toggle_crosshair(&mut self, _: &ToggleCrosshair, _: &mut Window, cx: &mut Context<Self>) {
        self.show_crosshair = !self.show_crosshair;
        cx.notify();
    }

    fn crosshair(&self, view: &View, scale: f32) -> Option<[Div; 2]> {
        let display = &self.layers[view.base].display;
        let plane = display.plane(view.axis, self.radiological);
        let (s, t) = view.visible.to_view(plane.project(display.cursor()));
        if !(0.0..=1.0).contains(&s) || !(0.0..=1.0).contains(&t) {
            return None;
        }
        let image = view.image_size;
        let left = (view.size.width - image.width) / 2.0;
        let top = (view.size.height - image.height) / 2.0;
        // "snap" to device screen pixels for sharp lines
        let snap = |value: f32| (value * scale).floor() / scale;
        let x = snap(left + s as f32 * image.width);
        let y = snap(top + (1.0 - t as f32) * image.height);
        let line = || div().absolute().bg(rgba(0x00ccffb3));
        Some([
            line()
                .left(px(x))
                .top(px(top))
                .w(px(1.0))
                .h(px(image.height)),
            line()
                .left(px(left))
                .top(px(y))
                .w(px(image.width))
                .h(px(1.0)),
        ])
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
        if let Err(error) = change(&mut self.layers[column].display) {
            eprintln!("Illuminator: layer {}: {error}", column + 1);
        }
        self.sync_from(column);
        cx.notify();
    }

    fn sync_from(&mut self, active: usize) {
        if self.layout.overlaid() {
            // make sure overlaid layers share world pos
            let cursor = self.layers[active].display.cursor();
            for column in &mut self.layers {
                column.display.set_cursor(cursor);
            }
            return;
        }
        if !self.sync {
            return;
        }
        let source = &self.layers[active].display;
        match self.sync_mode {
            SyncMode::World => {
                if !source.world_linked() {
                    return;
                }
                let cursor = source.cursor();
                for (index, column) in self.layers.iter_mut().enumerate() {
                    if index != active && column.display.world_linked() {
                        column.display.set_cursor(cursor);
                    }
                }
            }
            SyncMode::Proportional => {
                let fraction = source.fractional_cursor();
                for (index, column) in self.layers.iter_mut().enumerate() {
                    if index != active {
                        column.display.set_fractional_cursor(fraction);
                    }
                }
            }
        }
    }

    fn sync_from_first(&mut self) {
        if self.layout.overlaid() {
            if !self.layers.is_empty() {
                self.sync_from(0);
            }
            return;
        }
        let leader = self.layers.iter().position(|column| {
            self.sync_mode == SyncMode::Proportional || column.display.world_linked()
        });
        if let Some(leader) = leader {
            self.sync_from(leader);
        }
    }

    fn set_frame(&mut self, frame: usize, cx: &mut Context<Self>) {
        self.frame = frame;
        for (index, layer) in self.layers.iter_mut().enumerate() {
            if let Err(error) = layer.display.set_frame(frame) {
                eprintln!("Illuminator: layer {}: {error}", index + 1);
            }
        }
        cx.notify();
    }

    fn update_frame_slider(&mut self, cx: &mut Context<Self>) {
        let frames = self
            .layers
            .iter()
            .map(|column| column.display.volume().meta().timepoints())
            .max()
            .unwrap_or(1);
        let last_frame = frames - 1;
        if last_frame == 0 {
            // no 4d layers left
            self.frame_slider = None;
            self.time_playing = None;
            return;
        }
        if self
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

    /// Every view on screen in reading order
    fn views(&self, window: &Window) -> Vec<View> {
        let count = self.layers.len();
        if count == 0 {
            return Vec::new();
        }
        let area = view_area(window);
        let cells: Vec<(usize, ViewPlane)> = match self.layout {
            Layout::Ortho => ViewPlane::ALL.map(|axis| (0, axis)).to_vec(),
            Layout::Compare => ViewPlane::ALL
                .iter()
                .flat_map(|&axis| (0..count).map(move |layer| (layer, axis)))
                .collect(),
            Layout::Grid(axis) => (0..count).map(|layer| (layer, axis)).collect(),
            Layout::Single { axis, layer } => {
                vec![(layer.filter(|&layer| layer < count).unwrap_or(0), axis)]
            }
        };
        let columns = match self.layout {
            Layout::Compare => count,
            _ => grid_columns(cells.len(), area),
        };
        let rows = cells.len().div_ceil(columns);
        let cell = size(area.width / columns as f32, area.height / rows as f32);
        let scale = window.scale_factor();
        let overlaid = self.layout.overlaid();
        cells
            .into_iter()
            .enumerate()
            .map(|(n, (base, axis))| {
                let plane = self.layers[base].display.plane(axis, self.radiological);
                let (width, height) = (
                    (cell.width * scale) as usize,
                    (cell.height * scale) as usize,
                );
                let whole = fit_size(&plane, width, height);
                let fit = (
                    whole.0 as f64 / width.max(1) as f64,
                    whole.1 as f64 / height.max(1) as f64,
                );
                let zoom = self.zoom[axis.index()];
                let visible = zoom.visible(fit);
                // the visible part may have another shape than the whole plane so fit it again
                let image_pixels = fit_size(&visible.apply(plane), width, height);
                View {
                    origin: point(
                        (n % columns) as f32 * cell.width,
                        (n / columns) as f32 * cell.height,
                    ),
                    size: cell,
                    axis,
                    base,
                    layers: if overlaid { 0..count } else { base..base + 1 },
                    image_pixels,
                    image_size: size(image_pixels.0 as f32 / scale, image_pixels.1 as f32 / scale),
                    zoom,
                    fit,
                    visible,
                }
            })
            .collect()
    }

    fn hit(&self, position: Point<Pixels>, window: &Window) -> Option<Hit> {
        let x = f32::from(position.x);
        let y = f32::from(position.y) - TOOLBAR_HEIGHT;
        self.views(window).into_iter().find_map(|view| {
            let (local_x, local_y) = (x - view.origin.x, y - view.origin.y);
            if local_x < 0.0
                || local_y < 0.0
                || local_x >= view.size.width
                || local_y >= view.size.height
            {
                return None;
            }
            let left = (view.size.width - view.image_size.width) / 2.0;
            let top = (view.size.height - view.image_size.height) / 2.0;
            let in_view = (
                f64::from((local_x - left) / view.image_size.width),
                f64::from(1.0 - (local_y - top) / view.image_size.height),
            );
            let (s, t) = view.visible.to_plane(in_view);
            Some(Hit {
                base: view.base,
                axis: view.axis,
                s,
                t,
                in_view,
                image_size: view.image_size,
                fit: view.fit,
            })
        })
    }

    fn pick(&mut self, position: Point<Pixels>, window: &Window, cx: &mut Context<Self>) {
        let Some(hit) = self.hit(position, window) else {
            return;
        };
        let radiological = self.radiological;
        if self.layers[hit.base]
            .display
            .pick(hit.axis, radiological, hit.s, hit.t)
        {
            self.sync_from(hit.base);
            cx.notify();
        }
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, window: &Window, cx: &mut Context<Self>) {
        let Some(hit) = self.hit(event.position, window) else {
            return;
        };
        if event.modifiers.secondary() {
            let pixels = f64::from(f32::from(event.delta.pixel_delta(px(SCROLL_STEP)).y));
            let zoom = &mut self.zoom[hit.axis.index()];
            // 2^x should make the zooming feel even across different zooms
            *zoom = zoom.zoom_at((pixels / ZOOM_SPEED).exp2(), hit.in_view, hit.fit);
            cx.notify();
            return;
        }
        self.scroll_remainder += f32::from(event.delta.pixel_delta(px(SCROLL_STEP)).y);
        let steps = (self.scroll_remainder / SCROLL_STEP).trunc();
        if steps == 0.0 {
            return;
        }
        self.scroll_remainder -= steps * SCROLL_STEP;
        if self.layers[hit.base]
            .display
            .step_slice(hit.axis, f64::from(steps))
        {
            self.sync_from(hit.base);
            cx.notify();
        }
    }

    fn start_pan(&mut self, position: Point<Pixels>, window: &Window) {
        self.pan_drag = self.hit(position, window).map(|hit| PanDrag {
            axis: hit.axis,
            last: position,
            image_size: hit.image_size,
            fit: hit.fit,
        });
    }

    /// moves the zoomed view with mouse
    fn drag_pan(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.pan_drag else {
            return;
        };
        let dx = f64::from(f32::from(position.x - drag.last.x) / drag.image_size.width);
        // y is downwards
        let dy = -f64::from(f32::from(position.y - drag.last.y) / drag.image_size.height);
        let zoom = &mut self.zoom[drag.axis.index()];
        *zoom = zoom.pan((dx, dy), drag.fit);
        self.pan_drag = Some(PanDrag {
            last: position,
            ..drag
        });
        cx.notify();
    }

    /// Zooms active view plane at the middle
    fn zoom_active(&mut self, factor: f64, window: &Window, cx: &mut Context<Self>) {
        let Some(active) = self.active() else {
            return;
        };
        let views = self.views(window);
        let Some(view) = views.iter().find(|view| (view.base, view.axis) == active) else {
            return;
        };
        let zoom = &mut self.zoom[view.axis.index()];
        *zoom = zoom.zoom_at(factor, (0.5, 0.5), view.fit);
        cx.notify();
    }

    fn zoom_in(&mut self, _: &ZoomIn, window: &mut Window, cx: &mut Context<Self>) {
        self.zoom_active(2.0_f64.sqrt(), window, cx);
    }

    fn zoom_out(&mut self, _: &ZoomOut, window: &mut Window, cx: &mut Context<Self>) {
        self.zoom_active(0.5_f64.sqrt(), window, cx);
    }

    fn reset_zoom(&mut self, _: &ZoomReset, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom = Default::default();
        cx.notify();
    }

    fn drag_contrast(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((column, last)) = self.contrast_drag else {
            return;
        };
        let dx = f64::from(f32::from(position.x - last.x));
        let dy = f64::from(f32::from(position.y - last.y));
        self.layers[column].display.drag_window(dx, dy);
        self.contrast_drag = Some((column, position));
        cx.notify();
    }

    /// Starts a background render for each panel whose image is out of date since last draw
    fn schedule_renders(&mut self, window: &Window, cx: &mut Context<Self>) {
        // overlaid layers are all drawn on the base layers plane
        for view in self.views(window) {
            let (width, height) = view.image_pixels;
            if width == 0 || height == 0 {
                continue;
            }
            let plane = view.visible.apply(
                self.layers[view.base]
                    .display
                    .plane(view.axis, self.radiological),
            );
            let row = view.axis.index();
            for layer in view.layers {
                let entry = &self.layers[layer];
                let panel = &entry.panels[row];
                if !entry.visible || panel.rendering {
                    continue;
                }

                let job = entry.display.job_on(plane, width, height);
                let key = ImageKey {
                    sample: job.key.clone(),
                    coloring: entry.display.coloring(),
                };
                if panel.image.as_ref().is_some_and(|image| image.key == key) {
                    continue;
                }
                let reuse = panel
                    .samples
                    .as_ref()
                    .filter(|(sample, _)| *sample == key.sample)
                    .map(|(_, samples)| Arc::clone(samples));
                let load_id = entry.load_id;

                cx.spawn_in(window, async move |viewer, cx| {
                    let coloring = key.coloring;
                    let (samples, image) = cx
                        .background_spawn(async move {
                            let samples = reuse.unwrap_or_else(|| Arc::new(job.sample()));
                            let image = to_render_image(samples.colorize(coloring));
                            (samples, image)
                        })
                        .await;
                    let _ = viewer.update_in(cx, |viewer, window, cx| {
                        let Some(column) = viewer.layers.iter_mut().find(|c| c.load_id == load_id)
                        else {
                            return;
                        };
                        let panel = &mut column.panels[row];
                        panel.rendering = false;
                        panel.samples = Some((key.sample.clone(), samples));
                        if let Some(old) = panel.image.replace(PanelImage { key, image }) {
                            cx.drop_image(old.image, Some(window));
                        }
                        cx.notify();
                    });
                })
                .detach();
                self.layers[layer].panels[row].rendering = true;
            }
        }
    }

    /// Counts the selected layers values in the background once per frame
    fn schedule_histogram(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected() else {
            return;
        };
        let layer = &mut self.layers[index];
        let frame = layer.display.frame();
        // counting all voxels of every frame would slow a movie down; it counts when it stops
        if self.time_playing.is_some()
            || layer.display.label_mode()
            || layer.histogram_pending
            || layer.histogram_frame == Some(frame)
        {
            return;
        }
        layer.histogram_pending = true;
        let volume = layer.display.shared_volume();
        let volume_id = volume.id();
        let load_id = layer.load_id;
        cx.spawn(async move |viewer, cx| {
            let histogram = cx
                .background_spawn(async move {
                    let progress = illuminator_core::progress::Progress::default();
                    Histogram::of(&volume, frame, HISTOGRAM_BINS, &progress)
                })
                .await;
            let _ = viewer.update(cx, |viewer, cx| {
                let Some(layer) = viewer.layers.iter_mut().find(|l| l.load_id == load_id) else {
                    return;
                };
                layer.histogram_pending = false;
                if layer.display.volume().id() != volume_id {
                    return; // the layer was reloaded in the meantime
                }
                layer.histogram_frame = Some(frame);
                layer.histogram = histogram.unwrap_or_else(|error| {
                    eprintln!("Illuminator: layer histogram: {error}");
                    None
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn schedule_curve(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected() else {
            return;
        };
        let layer = &mut self.layers[index];
        let volume = layer.display.shared_volume();
        if volume.meta().timepoints() < 2 || layer.curve_pending {
            return;
        }
        let Some((voxel, _)) = layer.display.cursor_sample() else {
            layer.curve = None;
            return;
        };
        if layer
            .curve
            .as_ref()
            .is_some_and(|curve| curve.volume == volume.id() && curve.voxel == voxel)
        {
            return;
        }
        layer.curve_pending = true;
        let load_id = layer.load_id;
        cx.spawn(async move |viewer, cx| {
            let curve = cx
                .background_spawn(async move {
                    let values = volume.time_curve(voxel)?;
                    Some(TimeCurve {
                        volume: volume.id(),
                        voxel,
                        values,
                    })
                })
                .await;
            let _ = viewer.update(cx, |viewer, cx| {
                if let Some(layer) = viewer.layers.iter_mut().find(|l| l.load_id == load_id) {
                    layer.curve_pending = false;
                    layer.curve = curve;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Moves one window edge to the value under the mouse in the histogram
    fn drag_edge(
        &mut self,
        drag: DragEdge,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(layer) = self.layers.iter_mut().find(|l| l.load_id == drag.load_id) else {
            return;
        };
        let Some(histogram) = &layer.histogram else {
            return;
        };
        let fraction =
            f64::from(f32::from(position.x - bounds.left()) / f32::from(bounds.size.width));
        let value = histogram.min + fraction * (histogram.max - histogram.min);
        let (low, high) = layer.display.window_limits();
        match drag.edge {
            WindowEdge::Low => layer.display.set_window_limits(value, high),
            WindowEdge::High => layer.display.set_window_limits(low, value),
        }
        cx.notify();
    }

    fn apply_preset(&mut self, index: usize, preset: Preset, cx: &mut Context<Self>) {
        let Some(layer) = self.layers.get_mut(index) else {
            return;
        };
        let limits = match preset {
            Preset::Limits(low, high) => Some((low, high)),
            Preset::Percentiles(low, high) => layer
                .histogram
                .as_ref()
                .map(|histogram| (histogram.percentile(low), histogram.percentile(high))),
        };
        if let Some((low, high)) = limits {
            layer.display.set_window_limits(low, high);
            cx.notify();
        }
    }

    fn render_panels(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, label_color) = (theme.muted_foreground, theme.border, theme.info);
        let scale = window.scale_factor();
        let overlaid = self.layout.overlaid();
        let mut area = div()
            .id("views")
            .relative()
            .flex_1()
            .h_full()
            .bg(gpui::black())
            .on_hover(cx.listener(|viewer, hovered: &bool, _, cx| {
                if !hovered {
                    viewer.hover = None;
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|viewer, event: &MouseDownEvent, window, cx| {
                    viewer.focus_handle.focus(window, cx);
                    if event.click_count == 2 {
                        viewer.toggle_maximize(event.position, window, cx);
                        return;
                    }
                    viewer.set_active_panel(event.position, window, cx);
                    if event.modifiers.alt {
                        viewer.start_pan(event.position, window);
                    } else {
                        viewer.pick(event.position, window, cx);
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|viewer, event: &MouseDownEvent, window, _| {
                    viewer.start_pan(event.position, window);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|viewer, event: &MouseDownEvent, window, _| {
                    viewer.contrast_drag = viewer.hit(event.position, window).map(|hit| {
                        let layer = if viewer.layout.overlaid() {
                            viewer.selected
                        } else {
                            hit.base
                        };
                        (layer, event.position)
                    });
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|viewer, _: &MouseUpEvent, _, _| viewer.contrast_drag = None),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|viewer, _: &MouseUpEvent, _, _| viewer.pan_drag = None),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|viewer, _: &MouseUpEvent, _, _| viewer.pan_drag = None),
            )
            .on_mouse_move(cx.listener(|viewer, event: &MouseMoveEvent, window, cx| {
                viewer.set_hover(event.position, window, cx);
                if event.pressed_button.is_none() {
                    viewer.set_active_panel(event.position, window, cx);
                }
                match event.pressed_button {
                    Some(MouseButton::Left | MouseButton::Middle) if viewer.pan_drag.is_some() => {
                        viewer.drag_pan(event.position, cx)
                    }
                    Some(MouseButton::Left) => viewer.pick(event.position, window, cx),
                    Some(MouseButton::Right) => viewer.drag_contrast(event.position, cx),
                    _ => {}
                }
            }))
            .on_scroll_wheel(cx.listener(|viewer, event: &ScrollWheelEvent, window, cx| {
                viewer.scroll(event, window, cx);
            }));

        if self.layers.is_empty() {
            return area.child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(muted)
                    .child("Drop scans here or open from the toolbar"),
            );
        }

        for view in self.views(window) {
            let display = &self.layers[view.base].display;
            let labels = display.labels(view.axis, self.radiological);
            let mut panel = div()
                .absolute()
                .left(px(view.origin.x))
                .top(px(view.origin.y))
                .w(px(view.size.width))
                .h(px(view.size.height))
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .text_sm();
            let mut stack = div()
                .relative()
                .w(px(view.image_size.width))
                .h(px(view.image_size.height));
            let mut bottom = true;
            for layer in view.layers.clone() {
                let entry = &self.layers[layer];
                let Some(image) = entry.panels[view.axis.index()]
                    .image
                    .as_ref()
                    .filter(|_| entry.visible)
                else {
                    continue;
                };
                // the bottom layer covers the black background so it is always opaque
                let opacity = if bottom { 1.0 } else { entry.opacity };
                bottom = false;
                stack = stack.child(
                    img(image.image.clone())
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .opacity(opacity),
                );
            }
            panel = panel.child(stack);
            if !bottom
                && self.show_crosshair
                && let Some(lines) = self.crosshair(&view, scale)
            {
                panel = panel.children(lines);
            }

            let edge = |text: &'static str| div().absolute().text_color(label_color).child(text);
            panel = panel
                .child(div().absolute().inset_0().border_1().border_color(
                    if self.active_panel == (view.base, view.axis) {
                        label_color
                    } else {
                        border
                    },
                ))
                .when(self.info == InfoLevel::Full, |panel| {
                    panel.child(
                        v_flex()
                            .absolute()
                            .top_1()
                            .left_2()
                            .text_size(px(CORNER_TEXT))
                            .text_color(muted)
                            .child(labels.title)
                            .when(!overlaid, |title| title.child(display.name())),
                    )
                })
                .when(self.info != InfoLevel::Off, |panel| {
                    panel
                        .child(edge(labels.left).left_2())
                        .child(edge(labels.right).right_2())
                        .child(edge(labels.top).top_1())
                        .child(edge(labels.bottom).bottom_1())
                        .child(self.corner_info(&view, muted))
                });
            area = area.child(panel);
        }
        area
    }

    fn corner_info(&self, view: &View, color: Hsla) -> Div {
        let overlaid = view.layers.len() > 1;
        let numbered = |layer: usize, text: String| {
            if overlaid {
                format!("{}: {text}", layer + 1)
            } else {
                text
            }
        };
        let values = view
            .layers
            .clone()
            .filter(|&layer| self.layers[layer].visible)
            .map(|layer| {
                let value = self.layers[layer]
                    .display
                    .cursor_sample()
                    .map_or("-".to_string(), |(_, value)| format_value(value));
                numbered(layer, value)
            });
        let windowed = if overlaid {
            self.selected().unwrap_or(view.base)
        } else {
            view.base
        };
        let window_text = match self.layers[windowed].display.coloring() {
            Coloring::Labels { .. } => "Labels".to_string(),
            Coloring::Intensity { level, width, .. } => {
                format!("W {} L {}", format_value(width), format_value(level))
            }
        };
        let colorbar = match self.layers[windowed].display.coloring() {
            Coloring::Intensity {
                level,
                width,
                colormap,
                ..
            } => Some(colorbar(colormap, level - width / 2.0, level + width / 2.0)),
            Coloring::Labels { .. } => None,
        };
        let base = &self.layers[view.base].display;
        let (slice, count) = base.slice_position(view.axis);
        let corner = || {
            div()
                .absolute()
                .text_size(px(CORNER_TEXT))
                .text_color(color)
        };
        let full = self.info == InfoLevel::Full;
        div()
            .absolute()
            .inset_0()
            .when(full, |info| {
                info.child(
                    corner()
                        .top_1()
                        .right_2()
                        .child(v_flex().items_end().children(values)),
                )
                .child(
                    corner().bottom_1().left_2().child(
                        v_flex()
                            .when(!base.geometry().contains_world(base.cursor()), |text| {
                                text.child("Cursor outside volume")
                            })
                            .child(numbered(windowed, window_text)),
                    ),
                )
                .when_some(colorbar, |info, colorbar| {
                    info.child(
                        corner()
                            .top_0()
                            .bottom_0()
                            .right_2()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .child(colorbar),
                    )
                })
            })
            .child(
                corner()
                    .bottom_1()
                    .right_2()
                    .child(if view.zoom.scale > 1.0 {
                        format!("\u{d7}{:.1} {}/{count}", view.zoom.scale, slice + 1)
                    } else {
                        format!("{}/{count}", slice + 1)
                    }),
            )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, sidebar) = (theme.border, theme.sidebar);
        v_flex()
            .id("sidebar")
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_none()
            .p_2()
            .gap_3()
            .overflow_y_scroll()
            .bg(sidebar)
            .border_l_1()
            .border_color(border)
            .text_sm()
            .child(self.render_loading(cx))
            .child(self.render_layer_list(cx))
            .child(self.render_layer_settings(cx))
            .child(self.render_view_settings(cx))
    }

    fn render_loading(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        v_flex()
            .gap_1()
            .children(self.loader.statuses().into_iter().map(|status| {
                let id = status.id;
                v_flex()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis_start()
                                    .child(file_name(&status.path)),
                            )
                            .child(
                                Button::new(("load-cancel", id))
                                    .ghost()
                                    .xsmall()
                                    .icon(Carbon::Close)
                                    .tooltip("Cancel")
                                    .on_click(
                                        cx.listener(move |viewer, _, _, _| {
                                            viewer.loader.cancel(id)
                                        }),
                                    ),
                            ),
                    )
                    .child(
                        Progress::new(("load-progress", id)).value(status.progress as f32 * 100.0),
                    )
                    .child(div().text_xs().text_color(muted).child(status.stage))
            }))
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, background) = (theme.border, theme.sidebar);
        let context = Some(shortcuts::CONTEXT);
        let axis = self.layout.axis().unwrap_or(self.active_panel.1);
        let layout_button = |id: &'static str,
                             icon: Carbon,
                             tooltip: &'static str,
                             action: &dyn Action,
                             layout: Layout| {
            Button::new(id)
                .ghost()
                .small()
                .icon(icon)
                .selected(self.layout.same_kind(layout))
                .tooltip_with_action(tooltip, action, context)
                .on_click(cx.listener(move |viewer, _, _, cx| viewer.set_layout(layout, cx)))
        };
        let plane_button = |axis: ViewPlane, label: &'static str, tooltip: &'static str| {
            Button::new(label)
                .ghost()
                .small()
                .label(label)
                .tooltip(tooltip)
                .selected(self.layout.axis() == Some(axis))
                .on_click(cx.listener(move |viewer, _, _, cx| {
                    viewer.set_layout(viewer.layout.with_axis(axis), cx);
                }))
        };
        let separator = || div().w(px(1.0)).h(px(18.0)).mx_1().bg(border);

        h_flex()
            .h(px(TOOLBAR_HEIGHT))
            .flex_none()
            .px_2()
            .gap_1()
            .bg(background)
            .border_b_1()
            .border_color(border)
            .child(
                Button::new("open")
                    .ghost()
                    .small()
                    .icon(Carbon::FolderOpen)
                    .tooltip_with_action("Open volumes", &OpenFiles, context)
                    .on_click(cx.listener(|viewer, _, _, cx| viewer.open_files(cx))),
            )
            .child(separator())
            .child(layout_button(
                "layout-ortho",
                Carbon::Ortho,
                "All layers three planes",
                &LayoutOrtho,
                Layout::Ortho,
            ))
            .child(layout_button(
                "layout-compare",
                Carbon::Compare,
                "Side by side three planes",
                &LayoutCompare,
                Layout::Compare,
            ))
            .child(layout_button(
                "layout-grid",
                Carbon::Grid,
                "Side by side one plane",
                &LayoutGrid,
                Layout::Grid(axis),
            ))
            .child(layout_button(
                "layout-single",
                Carbon::Single,
                "Single view",
                &LayoutSingle,
                Layout::Single { axis, layer: None },
            ))
            .when(self.layout.axis().is_some(), |bar| {
                bar.child(separator())
                    .child(plane_button(ViewPlane::Axial, "A", "Axial"))
                    .child(plane_button(ViewPlane::Coronal, "C", "Coronal"))
                    .child(plane_button(ViewPlane::Sagittal, "S", "Sagittal"))
            })
            .child(separator())
            .child(
                Button::new("crosshair")
                    .ghost()
                    .small()
                    .icon(Carbon::Crosshair)
                    .selected(self.show_crosshair)
                    .tooltip_with_action("Crosshair", &ToggleCrosshair, context)
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.toggle_crosshair(&ToggleCrosshair, window, cx);
                    })),
            )
            .child(
                Button::new("radiological")
                    .ghost()
                    .small()
                    .icon(Carbon::Radiological)
                    .selected(self.radiological)
                    .tooltip_with_action("Radiological convention", &ToggleRadiological, context)
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.toggle_radiological(&ToggleRadiological, window, cx);
                    })),
            )
            .child(
                Button::new("info")
                    .ghost()
                    .small()
                    .icon(Carbon::CornerText)
                    .selected(self.info != InfoLevel::Off)
                    .tooltip_with_action(
                        format!("Text in views: {}", self.info.name()),
                        &ToggleInfo,
                        context,
                    )
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.toggle_info(&ToggleInfo, window, cx);
                    })),
            )
            .when(!self.layout.overlaid(), |bar| {
                bar.child(
                    Button::new("sync")
                        .ghost()
                        .small()
                        .icon(Carbon::Link)
                        .selected(self.sync)
                        .tooltip_with_action("Sync cursors", &ToggleSync, context)
                        .on_click(cx.listener(|viewer, _, window, cx| {
                            viewer.toggle_sync(&ToggleSync, window, cx);
                        })),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("help")
                    .ghost()
                    .small()
                    .icon(Carbon::Help)
                    .tooltip_with_action("Shortcuts", &ShowHelp, context)
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.show_help(&ShowHelp, window, cx);
                    })),
            )
    }

    /// Values of all layers at the mouse
    fn render_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, background) = (theme.muted_foreground, theme.border, theme.sidebar);
        let status = StatusBar::new()
            .h(px(STATUS_HEIGHT))
            .flex_none()
            .px_2()
            .gap_4()
            .text_xs()
            .bg(background)
            .border_t_1()
            .border_color(border);
        let Some(point) = self.hover else {
            return status.left(div().text_color(muted).child("Nothing to read"));
        };
        let mut status = status.left(format!("{:.1}, {:.1}, {:.1} mm", point.x, point.y, point.z));
        for (index, layer) in self.layers.iter().enumerate() {
            let value = layer
                .display
                .sample_world(point)
                .map_or("-".to_string(), |(_, value)| format_value(value));
            status = status.child(
                div()
                    .when(!layer.visible, |item| item.text_color(muted))
                    .when(index == self.selected, |item| {
                        item.font_weight(FontWeight::SEMIBOLD)
                    })
                    .child(format!("{}: {value}", index + 1)),
            );
        }
        if let Some(layer) = self.selected()
            && let Some(([x, y, z], _)) = self.layers[layer].display.sample_world(point)
        {
            status = status.right(format!("Layer {} voxel {x}, {y}, {z}", layer + 1));
        }
        status
    }

    /// One row per layer
    fn render_layer_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, active, hover, info) = (
            theme.muted_foreground,
            theme.list_active,
            theme.list_hover,
            theme.info,
        );
        let context = Some(shortcuts::CONTEXT);
        v_flex()
            .gap_0p5()
            .child(section_title("Layers", muted))
            .when(self.layers.is_empty(), |list| {
                list.child(div().text_xs().text_color(muted).child("No volumes open"))
            })
            .children(self.layers.iter().enumerate().map(|(index, layer)| {
                let display = &layer.display;
                let drag = DragLayer {
                    index,
                    name: display.name().into(),
                };
                let row = h_flex()
                    .id(("layer-row", index))
                    .group("layer-row")
                    .gap_1()
                    .h(px(26.0))
                    .px_1()
                    .rounded_sm()
                    .when(index == self.selected, |row| row.bg(active))
                    .hover(|row| row.bg(hover))
                    .on_click(cx.listener(move |viewer, _, _, cx| viewer.select(index, cx)))
                    .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
                    .drag_over::<DragLayer>(move |style, _, _, _| {
                        style.border_t_2().border_color(info)
                    })
                    .on_drop(cx.listener(move |viewer, drag: &DragLayer, _, cx| {
                        viewer.move_layer(drag.index, index, cx);
                    }))
                    .child(
                        Button::new(("layer-visible", index))
                            .ghost()
                            .xsmall()
                            .icon(if layer.visible {
                                Carbon::View
                            } else {
                                Carbon::ViewOff
                            })
                            .on_click(cx.listener(move |viewer, _, _, cx| {
                                let layer = &mut viewer.layers[index];
                                layer.visible = !layer.visible;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .w(px(14.0))
                            .text_xs()
                            .text_color(muted)
                            .child(format!("{}", index + 1)),
                    )
                    .child(div().w(px(18.0)).child(colormap_strip(
                        display.colormap,
                        display.label_mode(),
                        3,
                    )))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis_start()
                            .when(!layer.visible, |name| name.text_color(muted))
                            .child(display.name()),
                    )
                    .child(
                        h_flex()
                            .opacity(0.0)
                            .group_hover("layer-row", |buttons| buttons.opacity(1.0))
                            .child(
                                Button::new(("layer-reload", index))
                                    .ghost()
                                    .xsmall()
                                    .icon(Carbon::Reload)
                                    .tooltip_with_action("Reload from disk", &ReloadLayer, context)
                                    .on_click(cx.listener(move |viewer, _, _, _| {
                                        viewer.reload_layer(index);
                                    })),
                            )
                            .child(
                                Button::new(("layer-close", index))
                                    .ghost()
                                    .xsmall()
                                    .icon(Carbon::Close)
                                    .tooltip_with_action("Close", &CloseLayer, context)
                                    .on_click(cx.listener(move |viewer, _, _, cx| {
                                        viewer.close_layer(index, cx);
                                    })),
                            ),
                    );
                let details = layer_details(display);
                HoverCard::new(("layer-card", index))
                    .anchor(Anchor::RightCenter)
                    .trigger(row)
                    .content(move |_, _, cx| {
                        let muted = cx.theme().muted_foreground;
                        v_flex()
                            .w(px(360.0))
                            .gap_1()
                            .text_xs()
                            .children(details.iter().map(|(label, value)| {
                                h_flex()
                                    .gap_2()
                                    .items_start()
                                    .child(div().w(px(80.0)).text_color(muted).child(*label))
                                    .child(div().flex_1().min_w_0().child(value.clone()))
                            }))
                    })
            }))
    }

    fn render_histogram(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, foreground, info, background) = (
            theme.muted_foreground,
            theme.foreground,
            theme.info,
            theme.secondary,
        );
        let layer = &self.layers[index];
        let load_id = layer.load_id;
        let chart = div()
            .id(("histogram", index))
            .relative()
            .w_full()
            .h(px(HISTOGRAM_HEIGHT))
            .rounded_sm()
            .overflow_hidden()
            .bg(background)
            .on_drag_move(
                cx.listener(move |viewer, event: &DragMoveEvent<DragEdge>, _, cx| {
                    let drag = *event.drag(cx);
                    if drag.load_id == load_id {
                        viewer.drag_edge(drag, event.event.position, event.bounds, cx);
                    }
                }),
            );
        let Some(histogram) = &layer.histogram else {
            return v_flex().child(
                chart
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .text_color(muted)
                    .child("Counting values\u{2026}"),
            );
        };

        let span = histogram.max - histogram.min;
        let at = |value: f64| {
            if span > 0.0 {
                ((value - histogram.min) / span) as f32
            } else {
                0.5
            }
        };
        let (low, high) = layer.display.window_limits();
        let peak = histogram.counts.iter().copied().max().unwrap_or(0).max(1);
        let bins = histogram.counts.len() as f32;
        let bars = histogram.counts.iter().enumerate().map(|(bin, &count)| {
            let center = histogram.min + (bin as f64 + 0.5) * histogram.bin_width();
            let inside = (low..=high).contains(&center);
            let height = ((count as f64).ln_1p() / (peak as f64).ln_1p()) as f32;
            div()
                .absolute()
                .bottom_0()
                .left(relative(bin as f32 / bins))
                .w(relative(1.0 / bins))
                .h(relative(height))
                .bg(if inside {
                    foreground
                } else {
                    muted.opacity(0.5)
                })
        });
        let edge = |edge: WindowEdge, value: f64| {
            let id = match edge {
                WindowEdge::Low => "window-low",
                WindowEdge::High => "window-high",
            };
            div()
                .id((id, index))
                .absolute()
                .top_0()
                .bottom_0()
                .left(relative(at(value).clamp(0.0, 1.0)))
                .ml(px(-4.0))
                .w(px(8.0))
                .flex()
                .justify_center()
                .cursor_ew_resize()
                .child(div().w(px(2.0)).h_full().bg(info))
                .on_drag(DragEdge { load_id, edge }, |drag, _, _, cx| {
                    cx.new(|_| *drag)
                })
        };
        let shaded_left = at(low).clamp(0.0, 1.0);
        let shaded_right = at(high).clamp(0.0, 1.0);

        v_flex()
            .gap_0p5()
            .child(
                chart
                    .children(bars)
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(relative(shaded_left))
                            .w(relative(shaded_right - shaded_left))
                            .bg(info.opacity(0.15)),
                    )
                    .child(edge(WindowEdge::Low, low))
                    .child(edge(WindowEdge::High, high)),
            )
            .child(transfer_strip(histogram, layer.display.coloring()))
            .child(
                h_flex()
                    .justify_between()
                    .text_size(px(CORNER_TEXT))
                    .text_color(muted)
                    .child(format_value(histogram.min))
                    .child(format_value(histogram.max)),
            )
    }

    fn render_time_curve(&self, index: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let layer = &self.layers[index];
        let frames = layer.display.volume().meta().timepoints();
        if frames < 2 {
            return None;
        }
        let theme = cx.theme();
        let (muted, info, background, stroke) = (
            theme.muted_foreground,
            theme.info,
            theme.secondary,
            theme.foreground,
        );
        let title = section_title("Curve at cursor", muted);
        let Some(curve) = &layer.curve else {
            return Some(
                v_flex()
                    .gap_1()
                    .child(title)
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("Cursor is outside the volume"),
                    )
                    .into_any_element(),
            );
        };

        let finite = curve
            .values
            .iter()
            .copied()
            .filter(|value| value.is_finite());
        let low = finite.clone().fold(0.0, f64::min);
        let high = finite.fold(low, f64::max);
        let high = if high > low { high } else { low + 1.0 };
        let points: Vec<(usize, f64)> = curve
            .values
            .iter()
            .map(|&value| if value.is_finite() { value } else { low })
            .enumerate()
            .collect();
        let frame = layer.display.frame();
        let value = points.get(frame).map_or(0.0, |&(_, value)| value);
        let bounds = Rc::clone(&self.time_curve_bounds);
        let chart = LineChart::new(points)
            .x(|&(frame, _)| SharedString::from((frame + 1).to_string()))
            .y(|&(_, value)| value)
            .y_domain(low, high)
            .y_padding(4.0, 0.0)
            .x_axis(false)
            .grid(false)
            .appear(false)
            .linear()
            .stroke(stroke)
            .tooltip_title(|&(frame, _)| format!("Frame {}", frame + 1).into())
            .tooltip_value(|_, value| format_value(value).into());
        let plot = div()
            .id(("time-curve", index))
            .relative()
            .w_full()
            .h(px(CHART_CURVE_HEIGHT))
            .rounded_sm()
            .bg(background)
            .cursor_pointer()
            .child(
                canvas(move |area, _, _| bounds.set(area), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(chart)
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(frame as f32 / (frames - 1) as f32))
                    .w(px(1.0))
                    .bg(info),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|viewer, event: &MouseDownEvent, window, cx| {
                    viewer.pick_frame(event.position.x, window, cx);
                }),
            )
            .on_mouse_move(cx.listener(|viewer, event: &MouseMoveEvent, window, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    viewer.pick_frame(event.position.x, window, cx);
                }
            }));
        Some(
            v_flex()
                .gap_1()
                .child(title)
                .child(plot)
                .child(
                    h_flex()
                        .justify_between()
                        .text_xs()
                        .text_color(muted)
                        .child(format!(
                            "{} \u{2013} {}",
                            format_value(low),
                            format_value(high)
                        ))
                        .child(format!("frame {}: {}", frame + 1, format_value(value))),
                )
                .into_any_element(),
        )
    }

    /// Menu of window presets for layer index
    fn presets_button(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let viewer = cx.entity().downgrade();
        Button::new(("presets", index))
            .ghost()
            .xsmall()
            .icon(Carbon::WindowPreset)
            .tooltip("Window presets")
            .dropdown_menu(move |mut menu, _, _| {
                for (n, (group, presets)) in PRESETS.iter().enumerate() {
                    if n > 0 {
                        menu = menu.separator();
                    }
                    menu = menu.label(*group);
                    for &(name, preset) in *presets {
                        let viewer = viewer.clone();
                        menu = menu.item(PopupMenuItem::new(name).on_click(move |_, _, cx| {
                            let _ = viewer.update(cx, |viewer, cx| {
                                viewer.apply_preset(index, preset, cx);
                            });
                        }));
                    }
                }
                menu
            })
    }

    /// Settings of the selected layer
    fn render_layer_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(index) = self.selected() else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let (muted, danger, info) = (theme.muted_foreground, theme.danger, theme.info);
        let layer = &self.layers[index];
        let display = &layer.display;
        let geometry = display.geometry();
        let overlaid = self.layout.overlaid();
        let bottom_visible = self.layers.iter().position(|layer| layer.visible);
        let id = |name: &'static str| (name, index as u64);
        let row = |label: &'static str| {
            h_flex()
                .gap_2()
                .child(div().w(px(64.0)).text_xs().text_color(muted).child(label))
        };

        let mut settings = v_flex()
            .gap_2()
            .child(section_title(format!("Layer {}", index + 1), muted))
            .when(overlaid && bottom_visible != Some(index), |settings| {
                settings.child(
                    row("Opacity")
                        .child(div().flex_1().child(Slider::new(&layer.opacity_slider)))
                        .child(
                            div()
                                .w(px(32.0))
                                .text_xs()
                                .child(format!("{:.0}%", layer.opacity * 100.0)),
                        ),
                )
            });

        settings =
            if display.label_mode() {
                settings
                    .child(
                        row("Labels")
                            .child(
                                div()
                                    .flex_1()
                                    .child(format!("{} labels", display.label_count())),
                            )
                            .child(
                                Button::new(id("statistics"))
                                    .xsmall()
                                    .label("Statistics\u{2026}")
                                    .on_click(cx.listener(move |viewer, _, window, cx| {
                                        viewer.show_statistics(index, window, cx);
                                    })),
                            ),
                    )
                    .child(
                        Checkbox::new(id("outline"))
                            .label("Outline")
                            .checked(display.outline)
                            .on_click(cx.listener(move |viewer, checked: &bool, _, cx| {
                                viewer.layers[index].display.outline = *checked;
                                cx.notify();
                            })),
                    )
            } else {
                let window = display.window;
                settings
                    .child(
                        row("Window")
                            .child(div().flex_1().text_xs().child(format!(
                                "W {}  L {}",
                                format_value(window.width),
                                format_value(window.level)
                            )))
                            .child(self.presets_button(index, cx))
                            .child(
                                Button::new(id("auto"))
                                    .ghost()
                                    .xsmall()
                                    .icon(Carbon::AutoWindow)
                                    .tooltip_with_action(
                                        "Fit to the current frame",
                                        &AutoContrast,
                                        Some(shortcuts::CONTEXT),
                                    )
                                    .on_click(cx.listener(move |viewer, _, _, cx| {
                                        viewer.auto_window(index, cx);
                                    })),
                            ),
                    )
                    .child(self.render_histogram(index, cx))
                    .child(row("Colormap").child(h_flex().flex_1().gap_1().children(
                        Colormap::ALL.into_iter().map(|colormap| {
                            let name = colormap.name();
                            div()
                                .id(SharedString::from(format!("colormap-{name}")))
                                .flex_1()
                                .p_0p5()
                                .rounded_sm()
                                .border_1()
                                .border_color(if display.colormap == colormap {
                                    info
                                } else {
                                    transparent_black()
                                })
                                .child(colormap_strip(colormap, false, 8))
                                .tooltip(move |window, cx| Tooltip::new(name).build(window, cx))
                                .on_click(cx.listener(move |viewer, _, _, cx| {
                                    viewer.layers[index].display.colormap = colormap;
                                    cx.notify();
                                }))
                        }),
                    )))
                    .child(
                        Checkbox::new(id("clip-low"))
                            .label("Transparent below window")
                            .checked(display.clip_low)
                            .on_click(cx.listener(move |viewer, checked: &bool, _, cx| {
                                viewer.layers[index].display.clip_low = *checked;
                                cx.notify();
                            })),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Checkbox::new(id("suv"))
                                    .label("SUV")
                                    .checked(display.scale() != 1.0)
                                    .disabled(layer.suv.is_none())
                                    .on_click(cx.listener(move |viewer, checked: &bool, _, cx| {
                                        viewer.set_suv(index, *checked, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(layer.suv.map_or("no sidecar", |(_, source)| source)),
                            )
                            .child(
                                Button::new(id("suv-edit"))
                                    .ghost()
                                    .xsmall()
                                    .label("Edit\u{2026}")
                                    .tooltip("Weight, dose and timing for SUV")
                                    .on_click(cx.listener(move |viewer, _, window, cx| {
                                        viewer.edit_suv(index, window, cx);
                                    })),
                            ),
                    )
                    .when(display.range().is_none(), |settings| {
                        settings.child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child("No finite values in this frame"),
                        )
                    })
            };

        settings
            .children(self.render_time_curve(index, cx))
            .child(
                Checkbox::new(id("labels"))
                    .label("Label map")
                    .checked(display.label_mode())
                    .on_click(cx.listener(move |viewer, checked: &bool, _, cx| {
                        let checked = *checked;
                        viewer.change(index, |display| display.set_label_mode(checked), cx);
                    })),
            )
            .when(display.can_be_anatomical(), |settings| {
                settings.child(
                    Checkbox::new(id("native"))
                        .label("Native axes")
                        .checked(display.options().native)
                        .on_click(cx.listener(move |viewer, checked: &bool, _, cx| {
                            let checked = *checked;
                            viewer.change(index, |display| display.set_native(checked), cx);
                        })),
                )
            })
            .when(display.units_unknown(), |settings| {
                settings.child(
                    Checkbox::new(id("assume-mm"))
                        .label("Assume mm")
                        .checked(display.options().assume_mm)
                        .on_click(cx.listener(move |viewer, checked: &bool, _, cx| {
                            let checked = *checked;
                            viewer.change(index, |display| display.set_assume_mm(checked), cx);
                        })),
                )
            })
            .when(
                !overlaid && self.sync && self.sync_mode == SyncMode::World && geometry.anatomical,
                |settings| {
                    settings.child(
                        Checkbox::new(id("link"))
                            .label("Link in world")
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
            .when(overlaid && !geometry.anatomical, |settings| {
                settings.child(
                    div()
                        .text_xs()
                        .text_color(danger)
                        .child("No orientation in header. Overlay position may be wrong"),
                )
            })
            .into_any_element()
    }

    /// Settings shared by all layers. Empty unless there is something to set
    fn render_view_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let sync_modes = !self.layout.overlaid() && self.sync;
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
            .when(sync_modes || self.frame_slider.is_some(), |settings| {
                settings.child(section_title("View", muted))
            })
            .when_some(self.frame_slider.as_ref(), |settings, slider| {
                settings.child(
                    h_flex()
                        .gap_2()
                        .child(div().w(px(64.0)).text_xs().text_color(muted).child("Frame"))
                        .child(
                            v_flex()
                                .flex_1()
                                .gap_0p5()
                                .child(Slider::new(&slider.state))
                                .child(frame_ticks(slider.last_frame, muted)),
                        )
                        .child(div().w(px(48.0)).text_xs().child(format!(
                            "{} / {}",
                            self.frame + 1,
                            slider.last_frame + 1
                        ))),
                )
            })
            .when(self.frame_slider.is_some(), |settings| {
                let playing = self.time_playing.is_some();
                let viewer = cx.entity().downgrade();
                settings.child(
                    h_flex()
                        .gap_2()
                        .child(div().w(px(64.0)).text_xs().text_color(muted).child("Movie"))
                        .child(
                            Button::new("play")
                                .xsmall()
                                .icon(if playing { Carbon::Pause } else { Carbon::Play })
                                .label(if playing { "Pause" } else { "Play" })
                                .selected(playing)
                                .tooltip_with_action(
                                    "Play the frames",
                                    &TogglePlay,
                                    Some(shortcuts::CONTEXT),
                                )
                                .on_click(cx.listener(|viewer, _, window, cx| {
                                    viewer.toggle_play(&TogglePlay, window, cx);
                                })),
                        )
                        .child(
                            Button::new("frame-rate")
                                .ghost()
                                .xsmall()
                                .label(format!("{} fps", self.time_fps))
                                .tooltip("Frames per second")
                                .dropdown_menu(move |mut menu, _, _| {
                                    for rate in TIMEPOINT_FRAME_RATES {
                                        let viewer = viewer.clone();
                                        menu = menu.item(
                                            PopupMenuItem::new(format!("{rate} fps")).on_click(
                                                move |_, window, cx| {
                                                    let _ = viewer.update(cx, |viewer, cx| {
                                                        viewer.set_frames_per_second(
                                                            rate, window, cx,
                                                        );
                                                    });
                                                },
                                            ),
                                        );
                                    }
                                    menu
                                }),
                        ),
                )
            })
            .when(sync_modes, |settings| {
                settings.child(
                    h_flex()
                        .gap_2()
                        .child(div().w(px(64.0)).text_xs().text_color(muted).child("Sync"))
                        .child(mode_button("sync-world", "World", SyncMode::World))
                        .child(mode_button(
                            "sync-proportional",
                            "Proportional",
                            SyncMode::Proportional,
                        )),
                )
            })
    }
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for image in self.retired.drain(..) {
            cx.drop_image(image, Some(window));
        }
        self.schedule_renders(window, cx);
        self.schedule_histogram(cx);
        self.schedule_curve(cx);

        let theme = cx.theme();
        let (background, foreground) = (theme.background, theme.foreground);
        v_flex()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .font_features(tabular_nums())
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
            .on_action(cx.listener(Self::toggle_play))
            .on_action(cx.listener(Self::center_cursor))
            .on_action(cx.listener(Self::auto_contrast))
            .on_action(cx.listener(Self::next_colormap))
            .on_action(cx.listener(Self::toggle_segmentation))
            .on_action(cx.listener(Self::toggle_outline))
            .on_action(cx.listener(Self::toggle_overlay))
            .on_action(cx.listener(Self::layout_ortho))
            .on_action(cx.listener(Self::layout_compare))
            .on_action(cx.listener(Self::layout_grid))
            .on_action(cx.listener(Self::layout_single))
            .on_action(cx.listener(Self::close_selected))
            .on_action(cx.listener(Self::reload_selected))
            .on_action(cx.listener(Self::move_selected_up))
            .on_action(cx.listener(Self::move_selected_down))
            .on_action(cx.listener(Self::toggle_layer))
            .on_action(cx.listener(Self::next_layer))
            .on_action(cx.listener(Self::previous_layer))
            .on_action(cx.listener(Self::toggle_sync))
            .on_action(cx.listener(Self::toggle_radiological))
            .on_action(cx.listener(Self::toggle_crosshair))
            .on_action(cx.listener(Self::toggle_info))
            .on_action(cx.listener(Self::show_help))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::reset_zoom))
            .on_drop(cx.listener(|viewer, paths: &ExternalPaths, _, _| {
                for path in paths.paths() {
                    viewer.loader.enqueue(path.clone());
                }
            }))
            .child(self.render_toolbar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_panels(window, cx))
                    .child(self.render_sidebar(cx)),
            )
            .child(self.render_status(cx))
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

/// Where the item at `index` ends up when the item at `from` is moved to `to`
fn moved(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        to
    } else if from < index && index <= to {
        index - 1
    } else if to <= index && index < from {
        index + 1
    } else {
        index
    }
}

fn view_area(window: &Window) -> Size<f32> {
    let viewport = window.viewport_size();
    size(
        (f32::from(viewport.width) - SIDEBAR_WIDTH).max(0.0),
        (f32::from(viewport.height) - TOOLBAR_HEIGHT - STATUS_HEIGHT).max(0.0),
    )
}

fn grid_columns(count: usize, area: Size<f32>) -> usize {
    let side = |columns: usize| {
        let rows = count.div_ceil(columns);
        (area.width / columns as f32).min(area.height / rows as f32)
    };
    (1..=count.max(1))
        .max_by(|&a, &b| side(a).total_cmp(&side(b)))
        .unwrap_or(1)
}

pub(crate) fn format_value(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e9 {
        format!("{value:.0}")
    } else {
        format!("{value:.3}")
    }
}

fn section_title(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(text.into())
}

fn frame_ticks(last_frame: usize, color: Hsla) -> impl IntoElement {
    div()
        .relative()
        .w_full()
        .h(px(4.0))
        .children((0..=last_frame).map(move |frame| {
            div()
                .absolute()
                .top_0()
                .h_full()
                .w(px(1.0))
                .left(relative(frame as f32 / last_frame as f32))
                .bg(color)
        }))
}

fn colormap_strip(colormap: Colormap, labels: bool, blocks: usize) -> Div {
    h_flex()
        .w_full()
        .h(px(10.0))
        .rounded_sm()
        .overflow_hidden()
        .children((0..blocks).map(|block| {
            let [r, g, b] = if labels {
                let [r, g, b, _] = label_color(block as i64 + 1);
                [r, g, b]
            } else {
                colormap.color((block as f64 + 0.5) / blocks as f64)
            };
            div()
                .flex_1()
                .h_full()
                .bg(rgb(u32::from_be_bytes([0, r, g, b])))
        }))
}

/// The colors along the histogram's value axis, as the views show them
fn transfer_strip(histogram: &Histogram, coloring: Coloring) -> Div {
    const BLOCKS: usize = 64;
    let Coloring::Intensity {
        level,
        width,
        colormap,
        clip_low,
    } = coloring
    else {
        return div();
    };
    h_flex()
        .w_full()
        .h(px(6.0))
        .rounded_sm()
        .overflow_hidden()
        .children((0..BLOCKS).map(|block| {
            let fraction = (block as f64 + 0.5) / BLOCKS as f64;
            let value = histogram.min + fraction * (histogram.max - histogram.min);
            let t = (value - level) / width + 0.5;
            let block = div().flex_1().h_full();
            if clip_low && t <= 0.0 {
                return block;
            }
            let [r, g, b] = colormap.color(t);
            block.bg(rgb(u32::from_be_bytes([0, r, g, b])))
        }))
}

/// A vertical color bar with the window's top and bottom value
fn colorbar(colormap: Colormap, low: f64, high: f64) -> Div {
    const BLOCKS: usize = 32;
    v_flex()
        .items_end()
        .gap_0p5()
        .child(format_value(high))
        .child(
            v_flex()
                .w(px(6.0))
                .h(px(80.0))
                .rounded_sm()
                .overflow_hidden()
                .children((0..BLOCKS).rev().map(|block| {
                    let [r, g, b] = colormap.color((block as f64 + 0.5) / BLOCKS as f64);
                    div()
                        .w_full()
                        .flex_1()
                        .bg(rgb(u32::from_be_bytes([0, r, g, b])))
                })),
        )
        .child(format_value(low))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn tabular_nums() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}

fn layer_details(display: &VolumeDisplay) -> Vec<(&'static str, String)> {
    let h = display.volume().meta();
    let geometry = display.geometry();
    vec![
        ("File", display.name()),
        (
            "Size",
            format!("{} x {} x {}", h.width(), h.height(), h.depth()),
        ),
        (
            "Voxel",
            format!(
                "{} x {} x {} {}",
                format_value(h.spacing[0]),
                format_value(h.spacing[1]),
                format_value(h.spacing[2]),
                geometry.unit_name
            ),
        ),
        ("Frames", h.timepoints().to_string()),
        ("Type", h.datatype.name().to_string()),
        ("Format", h.format.name().to_string()),
        ("Transform", geometry.transform_name.to_string()),
        ("Path", display.volume().path().display().to_string()),
    ]
}

fn opacity_slider(load_id: u64, cx: &mut Context<Viewer>) -> (Entity<SliderState>, Subscription) {
    let state = cx.new(|_| {
        SliderState::new()
            .min(0.0)
            .max(100.0)
            .step(5.0)
            .default_value(OVERLAY_OPACITY * 100.0)
    });
    let changes = cx.subscribe(&state, move |viewer, _, event: &SliderEvent, cx| {
        if let SliderEvent::Change(value) = event
            && let Some(column) = viewer.layers.iter_mut().find(|c| c.load_id == load_id)
        {
            column.opacity = value.start() / 100.0;
            cx.notify();
        }
    });
    (state, changes)
}

#[cfg(test)]
mod tests {
    use super::{Layout, PRESETS, Preset, ViewPlane, format_value, grid_columns, moved, size};

    #[test]
    fn grids_make_cells_as_large_as_possible() {
        let laptop = size(1120.0, 840.0);
        assert_eq!(grid_columns(1, laptop), 1);
        assert_eq!(grid_columns(3, laptop), 2);
        assert_eq!(grid_columns(4, laptop), 2);
        assert_eq!(grid_columns(3, size(1600.0, 500.0)), 3);
    }

    #[test]
    fn layouts_know_their_plane() {
        let grid = Layout::Grid(ViewPlane::Axial);
        assert_eq!(
            grid.with_axis(ViewPlane::Coronal),
            Layout::Grid(ViewPlane::Coronal)
        );
        assert!(grid.same_kind(Layout::Grid(ViewPlane::Sagittal)));
        assert!(!grid.same_kind(Layout::Ortho));
        assert_eq!(Layout::Ortho.with_axis(ViewPlane::Coronal), Layout::Ortho);
        let single = |layer| Layout::Single {
            axis: ViewPlane::Axial,
            layer,
        };
        assert!(single(None).overlaid());
        assert!(!single(Some(1)).overlaid());
    }

    #[test]
    fn values_are_short() {
        assert_eq!(format_value(-1024.0), "-1024");
        assert_eq!(format_value(3.81234), "3.812");
    }

    #[test]
    fn presets_go_from_low_to_high() {
        for (_, presets) in PRESETS {
            for (name, preset) in *presets {
                match *preset {
                    Preset::Limits(low, high) => assert!(low < high, "{name}"),
                    Preset::Percentiles(low, high) => {
                        assert!((0.0..high).contains(&low) && high <= 1.0, "{name}");
                    }
                }
            }
        }
    }

    #[test]
    fn moving_shifts_the_items_between() {
        let after = |from, to| {
            (0..4)
                .map(|index| moved(index, from, to))
                .collect::<Vec<_>>()
        };
        assert_eq!(after(0, 2), [2, 0, 1, 3]);
        assert_eq!(after(2, 0), [1, 2, 0, 3]);
        assert_eq!(after(1, 1), [0, 1, 2, 3]);
    }
}
