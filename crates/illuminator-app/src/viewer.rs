//! The main window and its state.

mod actions;
mod background;
mod export;
mod histogram_window;
mod layer_settings;
mod layers;
mod layout;
mod mouse;
mod panels;
mod playback;
mod sidebar;
mod status_bar;
mod toolbar;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::component::button::{Button, ButtonVariants, DropdownButton};
use gpui_kit::component::chart::LineChart;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::hover_card::HoverCard;
use gpui_kit::component::input::{Input, InputEvent, InputState};
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
use illuminator_core::colormap::Colormap;
use illuminator_core::display::{SampleKey, SliceJob, VolumeDisplay};
use illuminator_core::geometry::{Point3D, SlicePlane, ViewPlane};
use illuminator_core::histogram::Histogram;
use illuminator_core::loader::{LoadState, VolumeLoader};
use illuminator_core::slicer::{Coloring, SampledSlice, SliceImage, fit_size};
use illuminator_core::suv::SuvInputs;
use illuminator_core::volume::Volume;
use image::{Frame, RgbaImage};
use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::assets::Carbon;
use crate::cli::{Cli, LayoutName};
use crate::label_statistics::{Snapshot, StatisticsView};
use crate::picture::{Picture, PictureImage, Rect};
use crate::shortcuts::{self, *};
use crate::suv_form::SuvForm;
use crate::ui::{
    self, AppStyle, colormap_strip, format_editable, format_value, parse_value, rgb_to_rgba,
    section_title,
};
use crate::zoom::{Visible, Zoom};

const SIDEBAR_WIDTH: f32 = 240.0;
const SCROLL_STEP: f32 = 20.0;
const OVERLAY_OPACITY: f32 = 0.5;
const TOOLBAR_HEIGHT: f32 = 36.0;
const TOOLBAR_LABELS_WIDTH: f32 = 1000.0;
const STATUS_HEIGHT: f32 = 24.0;
const POSITION_TEMPLATE: &str = "-000.0";
const VALUE_TEMPLATE: &str = "-0000.000";
const LABEL_TEMPLATE: &str = "00000";
const VOXEL_TEMPLATE: &str = "0000";
const CORNER_TEXT: f32 = 10.0;
const HISTOGRAM_BINS: usize = 128;
const HISTOGRAM_HEIGHT: f32 = 56.0;
const ZOOM_SPEED: f64 = 200.0;
const CHART_CURVE_HEIGHT: f32 = 64.0;
const TIMEPOINT_FRAME_RATES: [u32; 5] = [1, 2, 5, 10, 20];
const LIGHTBOX_GRIDS: [(usize, usize); 6] = [(3, 2), (3, 3), (4, 3), (4, 4), (5, 4), (6, 5)];
const LIGHTBOX_STEPS: [usize; 6] = [1, 2, 3, 4, 5, 10];
const LIGHTBOX_SLOTS: usize = 6;

/// Window presets under one heading in the presets menu
struct PresetGroup {
    title: &'static str,
    presets: &'static [(&'static str, Preset)],
}

const PRESETS: &[PresetGroup] = &[
    PresetGroup {
        title: "CT (HU)",
        presets: &[
            ("Brain  W 80 L 40", Preset::Limits(0.0, 80.0)),
            ("Subdural  W 200 L 75", Preset::Limits(-25.0, 175.0)),
            ("Stroke  W 40 L 40", Preset::Limits(20.0, 60.0)),
            ("Soft tissue  W 400 L 50", Preset::Limits(-150.0, 250.0)),
            ("Lung  W 1500 L -600", Preset::Limits(-1350.0, 150.0)),
            ("Bone  W 1800 L 400", Preset::Limits(-500.0, 1300.0)),
        ],
    },
    PresetGroup {
        title: "PET (SUV)",
        presets: &[
            ("0 to 5", Preset::Limits(0.0, 5.0)),
            ("0 to 10", Preset::Limits(0.0, 10.0)),
        ],
    },
    PresetGroup {
        title: "Histogram",
        presets: &[
            ("Full range", Preset::Percentiles(0.0, 1.0)),
            ("1 to 99 %", Preset::Percentiles(0.01, 0.99)),
            ("5 to 95 %", Preset::Percentiles(0.05, 0.95)),
        ],
    },
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

/// The samples behind a panel's image. A new coloring can reuse them.
struct PanelSamples {
    key: SampleKey,
    samples: Arc<SampledSlice>,
}

#[derive(Default)]
struct Panel {
    image: Option<PanelImage>,
    samples: Option<PanelSamples>,
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

/// Text fields for the low and high limit of the window of the selected layer.
struct LimitInputs {
    low: Entity<InputState>,
    high: Entity<InputState>,
    /// The layer whose limits the fields show
    load_id: Option<u64>,
    _events: [Subscription; 2],
}

/// The numbers for showing a layer in SUV and where they came from
#[derive(Clone, Copy)]
struct Suv {
    inputs: SuvInputs,
    source: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HistogramKey {
    volume: u64,
    frame: usize,
}

#[derive(Default)]
struct FrameHistogram {
    key: Option<HistogramKey>,
    pending: Option<HistogramKey>,
    histogram: Option<Histogram>,
    error: Option<String>,
}

impl FrameHistogram {
    fn current(&self, key: HistogramKey) -> Option<&Histogram> {
        self.histogram.as_ref().filter(|_| self.key == Some(key))
    }

    fn needs_count(&self, key: HistogramKey) -> bool {
        self.pending.is_none() && self.key != Some(key)
    }

    /// Only the owning request may release pending work even after a reload.
    fn complete(
        &mut self,
        request: HistogramKey,
        current: HistogramKey,
        histogram: Result<Option<Histogram>, String>,
    ) -> bool {
        if self.pending != Some(request) {
            return false;
        }
        self.pending = None;
        if request == current {
            self.key = Some(request);
            (self.histogram, self.error) = match histogram {
                Ok(histogram) => (histogram, None),
                Err(error) => (None, Some(error)),
            };
        }
        true
    }
}

struct Layer {
    display: VolumeDisplay,
    panels: Vec<Panel>,
    histogram: FrameHistogram,
    curve: Option<TimeCurve>,
    curve_pending: bool,
    suv: Option<Suv>,
    load_id: u64,
    visible: bool,
    opacity: f32,
    opacity_slider: Entity<SliderState>,
    _opacity_changes: Subscription,
}

impl Layer {
    fn new(load_id: u64, display: VolumeDisplay, cx: &mut Context<Viewer>) -> Self {
        let suv = match SuvInputs::from_sidecar(display.volume().path()) {
            Some(Ok(inputs)) => Some(Suv {
                inputs,
                source: "from sidecar",
            }),
            Some(Err(error)) => {
                eprintln!("Illuminator: {error}");
                None
            }
            None => None,
        };
        let (opacity_slider, opacity_changes) = opacity_slider(load_id, cx);
        Self {
            display,
            panels: Vec::new(),
            histogram: FrameHistogram::default(),
            curve: None,
            curve_pending: false,
            suv,
            load_id,
            visible: true,
            opacity: OVERLAY_OPACITY,
            opacity_slider,
            _opacity_changes: opacity_changes,
        }
    }

    fn histogram_key(&self) -> HistogramKey {
        HistogramKey {
            volume: self.display.volume().id(),
            frame: self.display.frame(),
        }
    }

    fn current_histogram(&self) -> Option<&Histogram> {
        self.histogram.current(self.histogram_key())
    }

    fn panel(&self, slot: usize) -> Option<&Panel> {
        self.panels.get(slot)
    }

    fn panel_mut(&mut self, slot: usize) -> &mut Panel {
        if self.panels.len() <= slot {
            self.panels.resize_with(slot + 1, Panel::default);
        }
        &mut self.panels[slot]
    }
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
    mip: bool,
    slot: usize,
    plane: SlicePlane,
    slice: Option<usize>,
    current: bool,
}

impl View {
    /// Where the image starts inside of the view
    fn image_offset(&self) -> Point<f32> {
        point(
            (self.size.width - self.image_size.width) / 2.0,
            (self.size.height - self.image_size.height) / 2.0,
        )
    }
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
    mip: bool,
    s: f64,
    t: f64,
    in_view: (f64, f64),
    image_size: Size<f32>,
    fit: (f64, f64),
    plane: SlicePlane,
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
    Lightbox(ViewPlane),
}

impl Layout {
    fn overlaid(self) -> bool {
        matches!(
            self,
            Self::Ortho | Self::Single { layer: None, .. } | Self::Lightbox(_)
        )
    }

    fn axis(self) -> Option<ViewPlane> {
        match self {
            Self::Grid(axis) | Self::Single { axis, .. } | Self::Lightbox(axis) => Some(axis),
            Self::Ortho | Self::Compare => None,
        }
    }

    fn with_axis(self, axis: ViewPlane) -> Self {
        match self {
            Self::Grid(_) => Self::Grid(axis),
            Self::Single { layer, .. } => Self::Single { axis, layer },
            Self::Lightbox(_) => Self::Lightbox(axis),
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
    show_mip: bool,
    frame: usize,
    frame_slider: Option<FrameSlider>,
    time_playing: Option<Task<()>>,
    time_fps: u32,
    time_curve_bounds: Rc<Cell<Bounds<Pixels>>>,
    lightbox_grid: (usize, usize),
    lightbox_step: usize,
    contrast_drag: Option<(usize, Point<Pixels>)>,
    limit_inputs: LimitInputs,
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
    label_loads: HashSet<u64>,
    load_colormaps: HashMap<u64, Colormap>,
    retired: Vec<Arc<RenderImage>>,
    histogram_window: Option<AnyWindowHandle>,
    _loader_events: Task<()>,
    _open_requests: Task<()>,
}

impl Viewer {
    pub fn new(
        cache: Arc<VolumeCache>,
        cli: Cli,
        mut opened: mpsc::UnboundedReceiver<Vec<PathBuf>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (sender, mut changes) = mpsc::unbounded::<()>();
        let loader = VolumeLoader::new(Arc::clone(&cache), move || {
            let _ = sender.unbounded_send(());
        });
        let mut load_colormaps = HashMap::new();
        for (file, colormap) in cli.files.into_iter().zip(cli.colormaps) {
            let load_id = loader.enqueue(file);
            if let Some(colormap) = colormap {
                load_colormaps.insert(load_id, colormap);
            }
        }
        let label_loads = cli
            .labels
            .into_iter()
            .map(|file| loader.enqueue(file))
            .collect();
        let axis = cli.plane.map_or(ViewPlane::Axial, ViewPlane::from);
        let layout = match cli.layout.unwrap_or(LayoutName::Ortho) {
            LayoutName::Ortho => Layout::Ortho,
            LayoutName::Compare => Layout::Compare,
            LayoutName::Grid => Layout::Grid(axis),
            LayoutName::Single => Layout::Single { axis, layer: None },
            LayoutName::Lightbox => Layout::Lightbox(axis),
        };
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
        let limit_inputs = LimitInputs::new(window, cx);
        Self {
            cache,
            loader,
            layers: Vec::new(),
            sync: true,
            sync_mode: SyncMode::World,
            radiological: false,
            show_crosshair: true,
            show_mip: cli.mip,
            frame: 0,
            frame_slider: None,
            time_playing: None,
            time_fps: 5,
            time_curve_bounds: Rc::default(),
            lightbox_grid: (4, 3),
            lightbox_step: 1,
            contrast_drag: None,
            limit_inputs,
            zoom: Default::default(),
            pan_drag: None,
            scroll_remainder: 0.0,
            active_panel: (0, axis),
            focus_handle,
            layout,
            previous_layout: Layout::Ortho,
            hover: None,
            selected: 0,
            info: InfoLevel::Minimal,
            reloads: HashMap::new(),
            label_loads,
            load_colormaps,
            retired: Vec::new(),
            histogram_window: None,
            _loader_events: loader_events,
            _open_requests: open_requests,
        }
    }

    fn take_loaded(&mut self, cx: &mut Context<Self>) {
        let mut added = false;
        for loaded in self.loader.take_completed() {
            let load_id = loaded.load_id;
            let reload_of = self.reloads.remove(&load_id);
            match VolumeDisplay::new(loaded) {
                Ok(display) => {
                    match reload_of {
                        Some(old_id) => self.replace_layer(old_id, display),
                        None => self.add_layer(load_id, display, cx),
                    }
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
        let (index, axis) = self.active_panel;
        (index < self.layers.len()).then_some((index, axis))
    }

    fn selected(&self) -> Option<usize> {
        (self.selected < self.layers.len()).then_some(self.selected)
    }

    fn layer_by_id(&mut self, load_id: u64) -> Option<&mut Layer> {
        self.layers
            .iter_mut()
            .find(|layer| layer.load_id == load_id)
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.layers.len() {
            self.selected = index;
            if !self.layout.overlaid() {
                self.active_panel.0 = index;
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

    fn toggle_play(&mut self, _: &TogglePlay, window: &mut Window, cx: &mut Context<Self>) {
        if self.time_playing.take().is_none() {
            self.play(window, cx);
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

    fn change(
        &mut self,
        index: usize,
        change: impl FnOnce(&mut VolumeDisplay) -> illuminator_core::Result<()>,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = change(&mut self.layers[index].display) {
            eprintln!("Illuminator: layer {}: {error}", index + 1);
        }
        self.sync_from(index);
        cx.notify();
    }

    fn sync_from(&mut self, active: usize) {
        if self.layout.overlaid() {
            // make sure overlaid layers share world pos
            let cursor = self.layers[active].display.cursor();
            for layer in &mut self.layers {
                layer.display.set_cursor(cursor);
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
                for (index, layer) in self.layers.iter_mut().enumerate() {
                    if index != active && layer.display.world_linked() {
                        layer.display.set_cursor(cursor);
                    }
                }
            }
            SyncMode::Proportional => {
                let fraction = source.fractional_cursor();
                for (index, layer) in self.layers.iter_mut().enumerate() {
                    if index != active {
                        layer.display.set_fractional_cursor(fraction);
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
        let leader = self.layers.iter().position(|layer| {
            self.sync_mode == SyncMode::Proportional || layer.display.world_linked()
        });
        if let Some(leader) = leader {
            self.sync_from(leader);
        }
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
        self.show_limits(window, cx);

        let theme = cx.theme();
        let (background, foreground) = (theme.background, theme.foreground);
        v_flex()
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
            .on_action(cx.listener(Self::layout_lightbox))
            .on_action(cx.listener(Self::copy_image))
            .on_action(cx.listener(Self::save_image))
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
            .on_action(cx.listener(Self::toggle_mip))
            .on_action(cx.listener(Self::show_help))
            .on_action(cx.listener(Self::show_histogram))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::reset_zoom))
            .on_drop(cx.listener(|viewer, paths: &ExternalPaths, _, _| {
                for path in paths.paths() {
                    viewer.loader.enqueue(path.clone());
                }
            }))
            .child(self.render_toolbar(window, cx))
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

fn view_area(window: &Window) -> Size<f32> {
    let viewport = window.viewport_size();
    size(
        (f32::from(viewport.width) - SIDEBAR_WIDTH).max(0.0),
        (f32::from(viewport.height) - TOOLBAR_HEIGHT - STATUS_HEIGHT).max(0.0),
    )
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
            && let Some(layer) = viewer.layer_by_id(load_id)
        {
            layer.opacity = value.start() / 100.0;
            cx.notify();
        }
    });
    (state, changes)
}

#[cfg(test)]
mod tests {
    use super::{FrameHistogram, Histogram, HistogramKey, Layout, PRESETS, Preset, ViewPlane};

    fn histogram(min: f64) -> Histogram {
        Histogram {
            min,
            max: min + 2.0,
            counts: vec![1, 1],
        }
    }

    #[test]
    fn histogram_cache_requires_current_volume_and_frame() {
        let key = HistogramKey {
            volume: 1,
            frame: 0,
        };
        let mut cache = FrameHistogram {
            pending: Some(key),
            ..Default::default()
        };
        assert!(cache.complete(key, key, Ok(Some(histogram(0.0)))));
        assert!(cache.current(key).is_some());
        assert!(!cache.needs_count(key));
        for current in [
            HistogramKey { volume: 2, ..key },
            HistogramKey { frame: 1, ..key },
        ] {
            assert!(cache.current(current).is_none());
            assert!(cache.needs_count(current));
        }
    }

    #[test]
    fn obsolete_histogram_completion_wakes_replacement_work() {
        let old = HistogramKey {
            volume: 1,
            frame: 0,
        };
        for current in [
            HistogramKey { volume: 2, ..old },
            HistogramKey { frame: 1, ..old },
        ] {
            let mut cache = FrameHistogram {
                pending: Some(old),
                ..Default::default()
            };
            assert!(!cache.needs_count(current));
            assert!(cache.complete(old, current, Ok(Some(histogram(0.0)))));
            assert!(cache.pending.is_none());
            assert!(cache.current(current).is_none());
            assert!(cache.needs_count(current));
        }
    }

    #[test]
    fn pre_reload_completion_cannot_release_or_replace_newer_work() {
        let old = HistogramKey {
            volume: 1,
            frame: 0,
        };
        let current = HistogramKey {
            volume: 2,
            frame: 0,
        };
        let mut cache = FrameHistogram {
            pending: Some(current),
            ..Default::default()
        };
        assert!(!cache.complete(old, current, Ok(Some(histogram(0.0)))));
        assert_eq!(cache.pending, Some(current));
        assert!(!cache.needs_count(current));
        assert!(cache.complete(current, current, Ok(Some(histogram(100.0)))));
        assert!(!cache.complete(old, current, Ok(Some(histogram(0.0)))));
        assert_eq!(cache.current(current).unwrap().min, 100.0);
    }

    #[test]
    fn empty_histogram_result_is_not_recounted_on_every_redraw() {
        let key = HistogramKey {
            volume: 1,
            frame: 0,
        };
        let mut cache = FrameHistogram {
            pending: Some(key),
            ..Default::default()
        };
        assert!(cache.complete(key, key, Ok(None)));
        assert!(cache.current(key).is_none());
        assert!(!cache.needs_count(key));
    }

    #[test]
    fn failed_count_keeps_its_error_for_the_frame() {
        let key = HistogramKey {
            volume: 1,
            frame: 0,
        };
        let mut cache = FrameHistogram {
            pending: Some(key),
            ..Default::default()
        };
        assert!(cache.complete(key, key, Err("too narrow".into())));
        assert!(cache.current(key).is_none());
        assert_eq!(cache.error.as_deref(), Some("too narrow"));
        assert!(!cache.needs_count(key));
        cache.pending = Some(key);
        assert!(cache.complete(key, key, Ok(Some(histogram(0.0)))));
        assert!(cache.error.is_none());
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
        let lightbox = Layout::Lightbox(ViewPlane::Axial);
        assert!(lightbox.overlaid());
        assert_eq!(
            lightbox.with_axis(ViewPlane::Sagittal),
            Layout::Lightbox(ViewPlane::Sagittal)
        );
    }

    #[test]
    fn presets_go_from_low_to_high() {
        for group in PRESETS {
            for (name, preset) in group.presets {
                match *preset {
                    Preset::Limits(low, high) => assert!(low < high, "{name}"),
                    Preset::Percentiles(low, high) => {
                        assert!((0.0..high).contains(&low) && high <= 1.0, "{name}");
                    }
                }
            }
        }
    }
}
