//! Plot windows.
//! Plot of the visible layers in a window of its own beside the viewer.

mod axes;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext, Bounds, ClipboardItem, Context, Div, Entity,
    FocusHandle, FontWeight, Hsla, InteractiveElement, IntoElement, MouseMoveEvent, ParentElement,
    PathBuilder, Pixels, Render, SharedString, Stateful, StatefulInteractiveElement, Styled,
    Subscription, Task, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowOptions, canvas,
    div, fill, point, px, relative, size,
};
use illuminator_core::Error;
use illuminator_core::display::VolumeDisplay;
use illuminator_core::geometry::{GeometryOptions, VolumeGeometry};
use illuminator_core::histogram::LabelMask;
use illuminator_core::meta::strip_extension;
use illuminator_core::progress::Progress;
use illuminator_core::stats::Source;
use illuminator_core::volume::Volume;
use std::any::TypeId;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::shortcuts::{self, CloseWindow};
use crate::theme;
use crate::ui::{choice_items, title_bar, uses_client_manager_decorations};

use super::{STALE_HISTOGRAM_OPACITY, Viewer};

pub(super) use self::axes::{Axes, ticks, value_at};

const PLOT_WINDOW_SIZE: (f32, f32) = (760.0, 480.0);
const PLOT_NOTICE_TIME: Duration = Duration::from_secs(4);
const PLOT_AXIS_WIDTH: f32 = 56.0;
const PLOT_AXIS_HEIGHT: f32 = 20.0;

/// Measures one layer on a background thread
pub(super) type Job<V> = Box<dyn FnOnce(&Progress) -> illuminator_core::Result<Option<V>> + Send>;

/// Points as fractions of the plot
pub(super) type Polyline = Vec<(f32, f32)>;

/// What a plot window measures of each layer
pub(super) trait Plot: Sized + 'static {
    /// What a layer's value is measured from
    type Key: Clone + PartialEq + 'static;
    type Value: Send + 'static;

    const TITLE: &'static str;
    const CSV_NAME: &'static str;
    const COPY_TOOLTIP: &'static str;
    const MEASURING: &'static str;
    const MEASURING_NOTE: &'static str;
    const NO_VALUES: &'static str;
    const ONLY_4D: bool = false;
    const WAITS_FOR_PAUSE: bool = false;

    /// What to measure of a layer, inside `region` when there is one
    fn key(&self, display: &VolumeDisplay, region: Option<&RegionKey>) -> Option<Self::Key>;

    fn job(
        key: &Self::Key,
        volume: Arc<Volume>,
        label_maps: &[LabelMap],
    ) -> Option<Job<Self::Value>>;

    fn shows_stale(_old: &Self::Key, _new: &Self::Key) -> bool {
        true
    }

    fn csv(shown: &[(&Line<Self::Key>, &Self::Value)]) -> String;

    fn controls(
        window: &PlotWindow<Self>,
        controls: Div,
        snapshot: Option<&Snapshot<Self::Key>>,
        cx: &mut Context<PlotWindow<Self>>,
    ) -> Div;

    fn chart(&self, plotted: &[Plotted<Self>], snapshot: &Snapshot<Self::Key>) -> Chart;

    fn snap(x: f64) -> f64 {
        x
    }

    /// Title of the readout at `x`
    fn x_name(x: f64) -> String;

    /// A layer's `value` at `x` in the readout
    fn readout(&self, value: &Self::Value, x: f64) -> String;

    /// Add plots own marks and mouse handling to `plot`
    fn decorate(
        &self,
        plot: Stateful<Div>,
        axes: &Axes,
        plotted: &[Plotted<Self>],
        snapshot: &Snapshot<Self::Key>,
        cx: &mut Context<PlotWindow<Self>>,
    ) -> Stateful<Div>;
}

/// What a plot shows of the plotted layers.
pub(super) struct Chart {
    pub(super) axes: Axes,
    pub(super) x_ticks: Vec<(f32, String)>,
    pub(super) y_ticks: Vec<(f32, String)>,
    pub(super) curves: Vec<Vec<Polyline>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LabelRegion {
    load_id: u64,
    label: Option<i64>,
}

#[derive(Clone, PartialEq)]
pub(super) struct RegionKey {
    pub(super) volume: u64,
    pub(super) frame: usize,
    pub(super) geometry: VolumeGeometry,
    pub(super) label: Option<i64>,
}

impl RegionKey {
    pub(super) fn job(&self, label_maps: &[LabelMap]) -> Option<RegionJob> {
        let map = label_maps
            .iter()
            .find(|map| map.volume.id() == self.volume)?;
        Some(RegionJob {
            volume: Arc::clone(&map.volume),
            geometry: self.geometry.clone(),
            frame: self.frame,
            label: self.label,
        })
    }
}

/// A label map region to measure inside of on a background thread.
pub(super) struct RegionJob {
    volume: Arc<Volume>,
    geometry: VolumeGeometry,
    frame: usize,
    label: Option<i64>,
}

impl RegionJob {
    pub(super) fn mask(&self) -> LabelMask<'_> {
        LabelMask {
            labels: Source {
                volume: &self.volume,
                geometry: &self.geometry,
                frame: self.frame,
            },
            label: self.label,
        }
    }
}

/// A layers value and the key it was measured for
pub(super) struct Measured<K, V> {
    pub(super) key: K,
    pub(super) value: Option<V>,
}

/// An intensity layer as a plot window shows it
pub(super) struct Line<K> {
    pub(super) load_id: u64,
    pub(super) number: usize,
    pub(super) name: SharedString,
    pub(super) color: Hsla,
    pub(super) volume: Arc<Volume>,
    pub(super) key: Option<K>,
    pub(super) limits: (f64, f64),
    pub(super) selected: bool,
}

/// A layers value in the plot
pub(super) struct Plotted<'a, P: Plot> {
    pub(super) line: &'a Line<P::Key>,
    pub(super) value: &'a P::Value,
    pub(super) stale: bool,
}

pub(super) struct LabelMap {
    load_id: u64,
    name: SharedString,
    labels: Vec<i64>,
    volume: Arc<Volume>,
    geometry: VolumeGeometry,
    frame: usize,
}

/// A layers curve as fractions of the plot
struct Stroke {
    color: Hsla,
    width: f32,
    polylines: Vec<Polyline>,
}

/// The viewers layers as the window needs them
pub(super) struct Snapshot<K> {
    pub(super) lines: Vec<Line<K>>,
    label_maps: Vec<LabelMap>,
    pub(super) playing: bool,
    pub(super) frame: usize,
}

/// What the window shows of the viewer. Only redraws when this changes
#[derive(PartialEq)]
struct Watched {
    layers: Vec<WatchedLayer>,
    selected: Option<usize>,
    playing: bool,
    frame: usize,
}

#[derive(PartialEq)]
struct WatchedLayer {
    load_id: u64,
    visible: bool,
    label_mode: bool,
    volume: u64,
    frame: usize,
    options: GeometryOptions,
    limits: (f64, f64),
    cursor: Option<[usize; 3]>,
}

impl Watched {
    fn of(viewer: &Viewer) -> Self {
        let layers = viewer.layers.iter().map(|layer| WatchedLayer {
            load_id: layer.load_id,
            visible: layer.visible,
            label_mode: layer.display.label_mode(),
            volume: layer.display.volume().id(),
            frame: layer.display.frame(),
            options: layer.display.options(),
            limits: layer.display.window_limits(),
            cursor: layer.display.cursor_sample().map(|(voxel, _)| voxel),
        });
        Self {
            layers: layers.collect(),
            selected: viewer.selected(),
            playing: viewer.time_playing.is_some(),
            frame: viewer.frame,
        }
    }
}

pub(super) struct PlotWindow<P: Plot> {
    viewer: WeakEntity<Viewer>,
    viewer_window: AnyWindowHandle,
    pub(super) plot: P,
    region: Option<LabelRegion>,
    /// By layer load id
    results: HashMap<u64, Measured<P::Key, P::Value>>,
    running: HashMap<u64, (P::Key, Arc<AtomicBool>)>,
    hidden: HashSet<u64>,
    hover: Option<f64>,
    plot_bounds: Rc<Cell<Bounds<Pixels>>>,
    notice: Option<SharedString>,
    notice_timer: Task<()>,
    watched: Option<Watched>,
    focus_handle: FocusHandle,
    _viewer_changes: Subscription,
    _viewer_closed: Subscription,
}

impl<P: Plot> PlotWindow<P> {
    fn new(
        plot: P,
        viewer: &Entity<Viewer>,
        viewer_window: AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self {
            viewer: viewer.downgrade(),
            viewer_window,
            plot,
            region: None,
            results: HashMap::new(),
            running: HashMap::new(),
            hidden: HashSet::new(),
            hover: None,
            plot_bounds: Rc::default(),
            notice: None,
            notice_timer: Task::ready(()),
            watched: None,
            focus_handle,
            _viewer_changes: cx.observe(viewer, |this, viewer, cx| {
                let watched = Watched::of(viewer.read(cx));
                if this.watched.as_ref() != Some(&watched) {
                    this.watched = Some(watched);
                    cx.notify();
                }
            }),
            _viewer_closed: cx.observe_release_in(viewer, window, |_, _, window, _| {
                window.remove_window();
            }),
        }
    }

    fn snapshot(&self, cx: &App) -> Option<Snapshot<P::Key>> {
        let viewer = self.viewer.upgrade()?;
        let viewer = viewer.read(cx);
        let label_maps: Vec<LabelMap> = viewer
            .layers
            .iter()
            .filter(|layer| layer.display.label_mode())
            .map(|layer| LabelMap {
                load_id: layer.load_id,
                name: layer.display.name().into(),
                labels: layer.display.labels().to_vec(),
                volume: layer.display.shared_volume(),
                geometry: layer.display.geometry().clone(),
                frame: layer.display.frame(),
            })
            .collect();
        let region = self.region.and_then(|region| {
            let map = label_maps
                .iter()
                .find(|map| map.load_id == region.load_id)?;
            Some(RegionKey {
                volume: map.volume.id(),
                frame: map.frame,
                geometry: map.geometry.clone(),
                label: region.label,
            })
        });
        let lines = viewer
            .layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| {
                let display = &layer.display;
                layer.visible
                    && !display.label_mode()
                    && (!P::ONLY_4D || display.volume().meta().timepoints() > 1)
            })
            .map(|(index, layer)| {
                let display = &layer.display;
                Line {
                    load_id: layer.load_id,
                    number: index + 1,
                    name: display.name().into(),
                    color: theme::curve_color(index, cx),
                    volume: display.shared_volume(),
                    key: self.plot.key(display, region.as_ref()),
                    limits: display.window_limits(),
                    selected: viewer.selected() == Some(index),
                }
            })
            .collect();
        Some(Snapshot {
            lines,
            label_maps,
            playing: viewer.time_playing.is_some(),
            frame: viewer.frame,
        })
    }

    /// Whether measuring waits for the movie to pause
    fn waiting(&self, snapshot: &Snapshot<P::Key>) -> bool {
        P::WAITS_FOR_PAUSE && snapshot.playing
    }

    /// Measures layers whose values are missing or out of date in the background
    fn schedule(&mut self, snapshot: &Snapshot<P::Key>, cx: &mut Context<Self>) {
        self.results
            .retain(|load_id, _| snapshot.lines.iter().any(|line| line.load_id == *load_id));
        // layers hidden in the legend are measured when they are shown again
        let wanted: Vec<&Line<P::Key>> = snapshot
            .lines
            .iter()
            .filter(|line| line.key.is_some() && !self.hidden.contains(&line.load_id))
            .collect();
        // stop measurements that are out of date or no longer wanted
        for (load_id, (key, cancelled)) in &self.running {
            if !wanted
                .iter()
                .any(|line| line.load_id == *load_id && line.key.as_ref() == Some(key))
            {
                cancelled.store(true, Ordering::Relaxed);
            }
        }
        if self.waiting(snapshot) {
            return;
        }
        for line in wanted {
            if current(&self.results, line).is_some() || self.running.contains_key(&line.load_id) {
                continue;
            }
            self.measure(line, snapshot, cx);
        }
    }

    fn measure(
        &mut self,
        line: &Line<P::Key>,
        snapshot: &Snapshot<P::Key>,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = line.key.clone() else {
            return;
        };
        let Some(job) = P::job(&key, Arc::clone(&line.volume), &snapshot.label_maps) else {
            return;
        };
        let load_id = line.load_id;
        let cancelled = Arc::new(AtomicBool::new(false));
        self.running
            .insert(load_id, (key.clone(), Arc::clone(&cancelled)));
        cx.spawn(async move |this, cx| {
            let value = cx
                .background_spawn(async move { job(&Progress::cancellable(cancelled)) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.running.remove(&load_id);
                let current = this.snapshot(cx).is_some_and(|snapshot| {
                    snapshot
                        .lines
                        .iter()
                        .any(|line| line.load_id == load_id && line.key.as_ref() == Some(&key))
                });
                match value {
                    Err(Error::Cancelled) => {}
                    value if current => {
                        let value = value.unwrap_or_else(|error| {
                            eprintln!("Illuminator: {}: {error}", P::TITLE);
                            None
                        });
                        this.results.insert(load_id, Measured { key, value });
                    }
                    _ => {}
                }
                // Even obsolete work must wake rendering!
                cx.notify();
            });
        })
        .detach();
    }

    /// The current values of the layers not hidden in the legend in layer order
    fn shown<'a>(
        &'a self,
        snapshot: &'a Snapshot<P::Key>,
    ) -> Vec<(&'a Line<P::Key>, &'a P::Value)> {
        snapshot
            .lines
            .iter()
            .filter(|line| !self.hidden.contains(&line.load_id))
            .filter_map(|line| Some((line, current(&self.results, line)?.value.as_ref()?)))
            .collect()
    }

    /// The values to plot of the layers not hidden in the legend in layer order
    fn plotted<'a>(&'a self, snapshot: &'a Snapshot<P::Key>) -> Vec<Plotted<'a, P>> {
        snapshot
            .lines
            .iter()
            .filter(|line| !self.hidden.contains(&line.load_id))
            .filter_map(|line| {
                let key = line.key.as_ref()?;
                let measured = self.results.get(&line.load_id)?;
                let stale = measured.key != *key;
                if stale && !P::shows_stale(&measured.key, key) {
                    return None;
                }
                Some(Plotted {
                    line,
                    value: measured.value.as_ref()?,
                    stale,
                })
            })
            .collect()
    }

    fn csv(&self, snapshot: &Snapshot<P::Key>) -> Option<String> {
        if !export_ready(&snapshot.lines, &self.results, &self.hidden) {
            return None;
        }
        Some(P::csv(&self.shown(snapshot)))
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot(cx) else {
            return;
        };
        if let Some(csv) = self.csv(&snapshot) {
            cx.write_to_clipboard(ClipboardItem::new_string(csv));
            self.show_notice("Copied.".into(), cx);
        }
    }

    fn show_notice(&mut self, notice: SharedString, cx: &mut Context<Self>) {
        self.notice = Some(notice);
        // replacing the timer drops the one of an earlier notice
        self.notice_timer = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PLOT_NOTICE_TIME).await;
            let _ = this.update(cx, |this, cx| {
                this.notice = None;
                cx.notify();
            });
        });
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot(cx) else {
            return;
        };
        let Some(csv) = self.csv(&snapshot) else {
            return;
        };
        let shown = self.shown(&snapshot);
        let Some((line, _)) = shown
            .iter()
            .find(|(line, _)| line.selected)
            .or(shown.first())
        else {
            return;
        };
        let directory = line
            .volume
            .path()
            .parent()
            .map(PathBuf::from)
            .unwrap_or_default();
        let name = format!("{}_{}.csv", strip_extension(&line.name), P::CSV_NAME);
        let chosen = cx.prompt_for_new_path(&directory, Some(&name));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = chosen.await else {
                return; // cancelled
            };
            let file = path.clone();
            let saved = cx
                .background_spawn(async move { std::fs::write(&file, csv) })
                .await;
            let notice = match saved {
                Ok(()) => format!("Saved to {}", path.display()),
                Err(error) => {
                    eprintln!("Illuminator: saving {}: {error}", path.display());
                    format!("Cannot save: {error}")
                }
            };
            let _ = this.update(cx, |this, cx| this.show_notice(notice.into(), cx));
        })
        .detach();
    }

    pub(super) fn update_viewer(
        &self,
        cx: &mut App,
        update: impl FnOnce(&mut Viewer, &mut Window, &mut Context<Viewer>),
    ) {
        let Some(viewer) = self.viewer.upgrade() else {
            return;
        };
        let _ = self.viewer_window.update(cx, |_, window, cx| {
            viewer.update(cx, |viewer, cx| update(viewer, window, cx));
        });
    }

    /// The value at `x` on the plot along an axis from `low` to `high`
    pub(super) fn x_at(&self, x: Pixels, (low, high): (f64, f64)) -> Option<f64> {
        let bounds = self.plot_bounds.get();
        let width = f32::from(bounds.size.width);
        if width <= 0.0 {
            return None;
        }
        let fraction = (f32::from(x - bounds.left()) / width).clamp(0.0, 1.0);
        Some(value_at((low, high), fraction))
    }

    pub(super) fn region_button<T: Copy + 'static>(
        &self,
        caption: &str,
        own: String,
        areas: Vec<(String, T)>,
        choose: impl Fn(&mut P, T) + 'static,
        snapshot: Option<&Snapshot<P::Key>>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let maps: Vec<(SharedString, u64, Vec<i64>)> = snapshot
            .map(|snapshot| {
                let maps = snapshot.label_maps.iter();
                maps.map(|map| (map.name.clone(), map.load_id, map.labels.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let chosen = match self.region {
            None => own,
            Some(region) => {
                let name = maps
                    .iter()
                    .find(|(_, load_id, _)| *load_id == region.load_id)
                    .map_or_else(SharedString::default, |(name, _, _)| name.clone());
                match region.label {
                    None => format!("{name}, all labels"),
                    Some(label) => format!("{name}, label {label}"),
                }
            }
        };
        let choose = Rc::new(choose);
        Button::new("region")
            .xsmall()
            .outline()
            .label(format!("{caption}: {chosen}"))
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                let choose = Rc::clone(&choose);
                let mut menu = choice_items(
                    menu.max_h(px(360.0)).scrollable(true),
                    &this,
                    areas.clone(),
                    move |this: &mut Self, area, _, cx| {
                        choose(&mut this.plot, area);
                        this.region = None;
                        cx.notify();
                    },
                );
                for (name, load_id, labels) in &maps {
                    let load_id = *load_id;
                    let all = std::iter::once(("All labels".to_string(), None));
                    let each = labels
                        .iter()
                        .map(|&label| (format!("Label {label}"), Some(label)));
                    let choices = all
                        .chain(each)
                        .map(|(text, label)| (text, LabelRegion { load_id, label }));
                    menu = choice_items(
                        menu.separator().label(name.clone()),
                        &this,
                        choices,
                        |this, region, _, cx| {
                            this.region = Some(region);
                            cx.notify();
                        },
                    );
                }
                menu
            })
    }

    fn render_controls(&self, snapshot: Option<&Snapshot<P::Key>>, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let can_export = snapshot
            .is_some_and(|snapshot| export_ready(&snapshot.lines, &self.results, &self.hidden));
        P::controls(self, h_flex().gap_3().flex_wrap(), snapshot, cx)
            .child(div().flex_1())
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .children(self.notice.clone()),
            )
            .child(
                Button::new("copy")
                    .xsmall()
                    .ghost()
                    .label("Copy")
                    .tooltip(P::COPY_TOOLTIP)
                    .disabled(!can_export)
                    .on_click(cx.listener(|this, _, _, cx| this.copy(cx))),
            )
            .child(
                Button::new("save")
                    .xsmall()
                    .label("Save CSV\u{2026}")
                    .disabled(!can_export)
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            )
    }

    fn render_plot(
        &self,
        snapshot: Option<&Snapshot<P::Key>>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let (muted, border, plot_background, background) = (
            theme.muted_foreground,
            theme.border,
            theme.secondary,
            theme.background,
        );
        let message = |text: &'static str| {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(muted)
                .child(text)
                .into_any_element()
        };
        let Some(snapshot) = snapshot else {
            return message("The viewer is closed");
        };
        if snapshot.lines.is_empty() {
            return message(if P::ONLY_4D {
                "No visible 4D intensity layers"
            } else {
                "No visible intensity layers"
            });
        }
        let measuring = if self.waiting(snapshot) {
            "Pause playback to count values"
        } else {
            P::MEASURING
        };
        // selected layer last so it is drawn on top
        let mut plotted = self.plotted(snapshot);
        plotted.sort_by_key(|plotted| plotted.line.selected);
        if plotted.is_empty() {
            let shown: Vec<&Line<P::Key>> = snapshot
                .lines
                .iter()
                .filter(|line| !self.hidden.contains(&line.load_id))
                .collect();
            return message(if shown.is_empty() {
                "All layers are hidden"
            } else if shown.iter().all(|line| line.key.is_none()) {
                "The cursor is outside the layers"
            } else if shown
                .iter()
                .any(|line| line.key.is_some() && current(&self.results, line).is_none())
            {
                measuring
            } else {
                P::NO_VALUES
            });
        }

        let Chart {
            axes,
            x_ticks,
            y_ticks,
            curves,
        } = self.plot.chart(&plotted, snapshot);
        let grid: Vec<f32> = y_ticks.iter().map(|&(at, _)| at).collect();
        let strokes: Vec<Stroke> = plotted
            .iter()
            .zip(curves)
            .map(|(plotted, polylines)| Stroke {
                color: if plotted.stale {
                    plotted.line.color.opacity(STALE_HISTOGRAM_OPACITY)
                } else {
                    plotted.line.color
                },
                width: if plotted.line.selected { 2.0 } else { 1.25 },
                polylines,
            })
            .collect();
        let paint = move |bounds: Bounds<Pixels>, _, window: &mut Window, _: &mut App| {
            let at = |(x, y): (f32, f32)| {
                point(
                    bounds.left() + bounds.size.width * x,
                    bounds.bottom() - bounds.size.height * y,
                )
            };
            for &y in &grid {
                let origin = at((0.0, y));
                window.paint_quad(fill(
                    Bounds::new(origin, size(bounds.size.width, px(1.0))),
                    border,
                ));
            }
            for stroke in strokes {
                for polyline in stroke.polylines {
                    let mut path = PathBuilder::stroke(px(stroke.width));
                    for (index, &step) in polyline.iter().enumerate() {
                        if index == 0 {
                            path.move_to(at(step));
                        } else {
                            path.line_to(at(step));
                        }
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, stroke.color);
                    }
                }
            }
        };
        let bounds = Rc::clone(&self.plot_bounds);
        let plot = div()
            .id("plot")
            .relative()
            .flex_1()
            .h_full()
            .overflow_hidden()
            .border_1()
            .border_color(border)
            .bg(plot_background)
            .child(
                canvas(move |area, _, _| bounds.set(area), paint)
                    .absolute()
                    .size_full(),
            );
        let mut plot = self.plot.decorate(plot, &axes, &plotted, snapshot, cx);
        // plot out of date values while measuring the current ones
        if plotted.iter().any(|plotted| plotted.stale) {
            plot = plot.child(
                div()
                    .absolute()
                    .top_1()
                    .left_0()
                    .right_0()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .px_1()
                            .rounded_sm()
                            .border_1()
                            .border_color(border)
                            .bg(plot_background)
                            .text_xs()
                            .text_color(muted)
                            .child(measuring),
                    ),
            );
        }

        if let Some(x) = self.hover {
            let at = axes.x(x);
            let rows = plotted
                .iter()
                .rev()
                .filter(|plotted| !plotted.stale)
                .map(|plotted| {
                    let line = plotted.line;
                    h_flex()
                        .gap_1()
                        .child(div().w(px(8.0)).h(px(8.0)).bg(line.color))
                        .child(format!(
                            "{} {}",
                            line.number,
                            self.plot.readout(plotted.value, x)
                        ))
                });
            let readout = v_flex()
                .absolute()
                .top_1()
                .p_1()
                .gap_0p5()
                .bg(background.opacity(0.9))
                .text_xs()
                .child(P::x_name(x))
                .children(rows);
            plot = plot
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(relative(at))
                        .w(px(1.0))
                        .bg(muted),
                )
                .child(if at < 0.5 {
                    readout.right_1()
                } else {
                    readout.left_1()
                });
        }

        let x = axes.x;
        let plot = plot
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                let Some(value) = this.x_at(event.position.x, x) else {
                    return;
                };
                let value = P::snap(value);
                if this.hover != Some(value) {
                    this.hover = Some(value);
                    cx.notify();
                }
            }))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !hovered {
                    this.hover = None;
                    cx.notify();
                }
            }));

        let y_labels =
            div()
                .relative()
                .w(px(PLOT_AXIS_WIDTH))
                .h_full()
                .children(y_ticks.into_iter().map(|(at, label)| {
                    div()
                        .absolute()
                        .right(px(6.0))
                        .top(relative(1.0 - at))
                        .mt(px(-7.0))
                        .text_xs()
                        .text_color(muted)
                        .child(label)
                }));
        let x_labels = div()
            .relative()
            .h(px(PLOT_AXIS_HEIGHT))
            .ml(px(PLOT_AXIS_WIDTH))
            .children(x_ticks.into_iter().map(|(at, label)| {
                div()
                    .absolute()
                    .top_0p5()
                    .left(relative(at))
                    .ml(px(-40.0))
                    .w(px(80.0))
                    .flex()
                    .justify_center()
                    .text_xs()
                    .text_color(muted)
                    .child(label)
            }));
        v_flex()
            .flex_1()
            .min_h_0()
            .child(h_flex().flex_1().min_h_0().child(y_labels).child(plot))
            .child(x_labels)
            .into_any_element()
    }

    fn render_legend(&self, snapshot: Option<&Snapshot<P::Key>>, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let waiting = snapshot.is_some_and(|snapshot| self.waiting(snapshot));
        let lines = snapshot.map_or(&[][..], |snapshot| &snapshot.lines);
        h_flex()
            .gap_4()
            .flex_wrap()
            .text_xs()
            .children(lines.iter().map(|line| {
                let load_id = line.load_id;
                let hidden = self.hidden.contains(&load_id);
                let note = match current(&self.results, line) {
                    // hidden layers aren't measured until they are shown
                    None if hidden => "",
                    None if line.key.is_none() => " (cursor outside)",
                    None if waiting => " (pause to count)",
                    None => P::MEASURING_NOTE,
                    Some(measured) if measured.value.is_none() => " (no values)",
                    Some(_) => "",
                };
                h_flex()
                    .id(("legend", load_id))
                    .gap_1()
                    .cursor_pointer()
                    .when(hidden, |entry| entry.opacity(0.4))
                    .when(line.selected, |entry| {
                        entry.font_weight(FontWeight::SEMIBOLD)
                    })
                    .child(div().w(px(14.0)).h(px(3.0)).bg(line.color))
                    .child(format!("{} {}", line.number, line.name))
                    .child(div().text_color(muted).child(note))
                    .tooltip(|window, cx| Tooltip::new("Show or hide").build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.hidden.remove(&load_id) {
                            this.hidden.insert(load_id);
                        }
                        cx.notify();
                    }))
            }))
    }
}

impl<P: Plot> Render for PlotWindow<P> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.snapshot(cx);
        if let Some(snapshot) = &snapshot {
            // the region's label map was closed or isn't a label map anymore
            if let Some(region) = self.region
                && !snapshot
                    .label_maps
                    .iter()
                    .any(|map| map.load_id == region.load_id)
            {
                self.region = None;
                cx.notify();
            }
            self.schedule(snapshot, cx);
        }
        let theme = cx.theme();
        let (background, foreground) = (theme.background, theme.foreground);
        v_flex()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .text_sm()
            .track_focus(&self.focus_handle)
            .key_context(shortcuts::PLOT_CONTEXT)
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .when(uses_client_manager_decorations(window), |root| {
                root.child(title_bar(P::TITLE))
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .p_3()
                    .gap_2()
                    .child(self.render_controls(snapshot.as_ref(), cx))
                    .child(self.render_plot(snapshot.as_ref(), cx))
                    .child(self.render_legend(snapshot.as_ref(), cx)),
            )
    }
}

impl Viewer {
    /// Opens a plot window of the kind of `plot` or brings a open window to the front
    pub(super) fn show_plot_window<P: Plot>(
        &mut self,
        plot: P,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let kind = TypeId::of::<P>();
        if let Some(handle) = self.plot_windows.get(&kind).copied()
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let viewer = cx.entity();
        let viewer_window = window.window_handle();
        cx.defer(move |cx| {
            let (width, height) = PLOT_WINDOW_SIZE;
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(width), px(height)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some(P::TITLE.into()),
                    ..Default::default()
                }),
                app_id: Some("illuminator".into()),
                ..Default::default()
            };
            let opened = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| PlotWindow::new(plot, &viewer, viewer_window, window, cx))
            });
            match opened {
                Ok((handle, _)) => viewer.update(cx, |viewer, _| {
                    viewer.plot_windows.insert(kind, handle);
                }),
                Err(error) => eprintln!(
                    "Illuminator: cannot open the {} window: {error}",
                    P::TITLE.to_lowercase()
                ),
            }
        });
    }
}

pub(super) fn current<'a, K: PartialEq, V>(
    results: &'a HashMap<u64, Measured<K, V>>,
    line: &Line<K>,
) -> Option<&'a Measured<K, V>> {
    results
        .get(&line.load_id)
        .filter(|measured| line.key.as_ref() == Some(&measured.key))
}

pub(super) fn export_ready<K: PartialEq, V>(
    lines: &[Line<K>],
    results: &HashMap<u64, Measured<K, V>>,
    hidden: &HashSet<u64>,
) -> bool {
    let mut has_values = false;
    for line in lines
        .iter()
        .filter(|line| line.key.is_some() && !hidden.contains(&line.load_id))
    {
        let Some(measured) = current(results, line) else {
            return false;
        };
        has_values |= measured.value.is_some();
    }
    has_values
}
