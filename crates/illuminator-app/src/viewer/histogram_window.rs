//! Histogram window.
//! In a window of its own beside the viewer.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, AppContext, Bounds, ClipboardItem, Context, Div, DragMoveEvent, Entity,
    FocusHandle, FontWeight, Hsla, InteractiveElement, IntoElement, MouseMoveEvent, ParentElement,
    PathBuilder, Pixels, Render, SharedString, StatefulInteractiveElement, Styled, Subscription,
    Task, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowOptions, canvas, div, fill,
    point, px, relative, size,
};
use illuminator_core::Error;
use illuminator_core::geometry::{GeometryOptions, VolumeGeometry};
use illuminator_core::histogram::{Counting, Histogram, LabelMask, to_csv};
use illuminator_core::meta::strip_extension;
use illuminator_core::progress::Progress;
use illuminator_core::stats::Source;
use illuminator_core::volume::Volume;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::shortcuts::{self, CloseWindow, ShowHistogram};
use crate::theme;
use crate::ui::{choice_items, format_value};

use super::{DragEdge, Viewer, WindowEdge};

const HISTOGRAM_WINDOW_SIZE: (f32, f32) = (760.0, 480.0);
const HISTOGRAM_NOTICE_TIME: Duration = Duration::from_secs(4);
const HISTOGRAM_AXIS_WIDTH: f32 = 56.0;
const HISTOGRAM_AXIS_HEIGHT: f32 = 20.0;
const HISTOGRAM_X_TICKS: usize = 8;
const HISTOGRAM_Y_TICKS: usize = 5;
const HISTOGRAM_MAX_TICKS: usize = 64;
const HISTOGRAM_MAX_TICK_DECIMALS: usize = 324;
const HISTOGRAM_BIN_CHOICES: [Option<usize>; 7] = [
    None,
    Some(32),
    Some(64),
    Some(128),
    Some(256),
    Some(512),
    Some(1024),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AxisY {
    Count,
    Probability,
    Cumulative,
}

impl AxisY {
    const ALL: [Self; 3] = [Self::Count, Self::Probability, Self::Cumulative];

    fn name(self) -> &'static str {
        match self {
            Self::Count => "Count",
            Self::Probability => "Probability",
            Self::Cumulative => "Cumulative",
        }
    }

    /// The height of each bin.
    /// Probabilities are in percent.
    fn heights(self, histogram: &Histogram) -> Vec<f64> {
        let percent = 100.0 / histogram.total().max(1) as f64;
        let counts = histogram.counts.iter();
        match self {
            Self::Count => counts.map(|&count| count as f64).collect(),
            Self::Probability => counts.map(|&count| count as f64 * percent).collect(),
            Self::Cumulative => counts
                .scan(0, |sum, &count| {
                    *sum += count;
                    Some(*sum as f64 * percent)
                })
                .collect(),
        }
    }

    fn format(self, height: f64) -> String {
        match self {
            Self::Count => format!("{height:.0}"),
            Self::Probability | Self::Cumulative => format!("{height:.2} %"),
        }
    }
}

/// Only count voxels on the labels of a label map layer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Region {
    load_id: u64,
    label: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Settings {
    counting: Counting,
    y_axis: AxisY,
    log: bool,
    region: Option<Region>,
}

/// What a curve is counted from.
#[derive(Clone, PartialEq)]
struct CurveKey {
    volume: u64,
    frame: usize,
    geometry: VolumeGeometry,
    counting: Counting,
    region: Option<RegionKey>,
}

#[derive(Clone, PartialEq)]
struct RegionKey {
    volume: u64,
    frame: usize,
    geometry: VolumeGeometry,
    label: Option<i64>,
}

struct Curve {
    key: CurveKey,
    histogram: Option<Histogram>,
}

/// An intensity layer as the window shows it.
struct Line {
    load_id: u64,
    number: usize,
    name: SharedString,
    color: Hsla,
    volume: Arc<Volume>,
    key: CurveKey,
    limits: (f64, f64),
    selected: bool,
}

/// A label map layer to count inside of.
struct LabelMap {
    load_id: u64,
    name: SharedString,
    labels: Vec<i64>,
    volume: Arc<Volume>,
    geometry: VolumeGeometry,
    frame: usize,
}

/// A layers curve in the plot.
struct Shown<'a> {
    line: &'a Line,
    histogram: &'a Histogram,
    heights: Vec<f64>,
}

/// A curve's outline as fractions of the plot.
struct Stroke {
    color: Hsla,
    width: f32,
    points: Vec<(f32, f32)>,
}

/// The viewers layers as the window needs them.
struct Snapshot {
    lines: Vec<Line>,
    label_maps: Vec<LabelMap>,
    playing: bool,
}

/// What the window shows of the viewer. It redraws only when this changes.
#[derive(PartialEq)]
struct Watched {
    layers: Vec<WatchedLayer>,
    selected: Option<usize>,
    playing: bool,
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
        });
        Self {
            layers: layers.collect(),
            selected: viewer.selected(),
            playing: viewer.time_playing.is_some(),
        }
    }
}

/// Where values and heights land in the plot as fractions from its left and bottom.
struct Axes {
    x: (f64, f64),
    /// In log10 of the height
    y: (f64, f64),
    log: bool,
}

impl Axes {
    fn x(&self, value: f64) -> f32 {
        let (low, high) = self.x;
        let span = high - low;
        if span.is_finite() {
            ((value - low) / span) as f32
        } else {
            ((value * 0.5 - low * 0.5) / (high * 0.5 - low * 0.5)) as f32
        }
    }

    fn y(&self, height: f64) -> f32 {
        let height = match self.log {
            true if height > 0.0 => height.log10(),
            true => self.y.0,
            false => height,
        };
        (((height - self.y.0) / (self.y.1 - self.y.0)) as f32).clamp(0.0, 1.0)
    }
}

pub struct HistogramWindow {
    viewer: WeakEntity<Viewer>,
    settings: Settings,
    /// By layer load id
    curves: HashMap<u64, Curve>,
    /// Counts running in the background by layer
    counting: HashMap<u64, (CurveKey, Arc<AtomicBool>)>,
    hidden: HashSet<u64>,
    /// The value under the mouse
    hover: Option<f64>,
    plot_bounds: Rc<Cell<Bounds<Pixels>>>,
    notice: Option<SharedString>,
    notice_timer: Task<()>,
    watched: Option<Watched>,
    focus_handle: FocusHandle,
    _viewer_changes: Subscription,
    _viewer_closed: Subscription,
}

impl HistogramWindow {
    fn new(viewer: &Entity<Viewer>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self {
            viewer: viewer.downgrade(),
            settings: Settings {
                counting: Counting {
                    bins: None,
                    ignore_zeros: true,
                },
                y_axis: AxisY::Count,
                log: false,
                region: None,
            },
            curves: HashMap::new(),
            counting: HashMap::new(),
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

    fn snapshot(&self, cx: &App) -> Option<Snapshot> {
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
        let region = self.settings.region.and_then(|region| {
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
            .filter(|(_, layer)| layer.visible && !layer.display.label_mode())
            .map(|(index, layer)| {
                let display = &layer.display;
                Line {
                    load_id: layer.load_id,
                    number: index + 1,
                    name: display.name().into(),
                    color: theme::curve_color(index, cx),
                    volume: display.shared_volume(),
                    key: CurveKey {
                        volume: display.volume().id(),
                        frame: display.frame(),
                        geometry: display.geometry().clone(),
                        counting: self.settings.counting,
                        region: region.clone(),
                    },
                    limits: display.window_limits(),
                    selected: viewer.selected() == Some(index),
                }
            })
            .collect();
        Some(Snapshot {
            lines,
            label_maps,
            playing: viewer.time_playing.is_some(),
        })
    }

    /// Counts layers whose curves are missing or out of date in the background.
    fn schedule(&mut self, snapshot: &Snapshot, cx: &mut Context<Self>) {
        self.curves
            .retain(|load_id, _| snapshot.lines.iter().any(|line| line.load_id == *load_id));
        // layers hidden in the legend are counted when they are shown again
        let wanted: Vec<&Line> = snapshot
            .lines
            .iter()
            .filter(|line| !self.hidden.contains(&line.load_id))
            .collect();
        // stop counts that are out of date or no longer wanted, so their layer is free sooner
        for (load_id, (key, cancelled)) in &self.counting {
            if !wanted
                .iter()
                .any(|line| line.load_id == *load_id && line.key == *key)
            {
                cancelled.store(true, Ordering::Relaxed);
            }
        }
        // Only count when the "movie" isn't playing
        if snapshot.playing {
            return;
        }
        for line in wanted {
            if current_curve(&self.curves, line).is_some()
                || self.counting.contains_key(&line.load_id)
            {
                continue;
            }
            self.count(line, snapshot, cx);
        }
    }

    fn count(&mut self, line: &Line, snapshot: &Snapshot, cx: &mut Context<Self>) {
        let load_id = line.load_id;
        let key = line.key.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        self.counting
            .insert(load_id, (key.clone(), Arc::clone(&cancelled)));
        let (volume, geometry, frame) = (Arc::clone(&line.volume), key.geometry.clone(), key.frame);
        let counting = key.counting;
        let labels = self.settings.region.and_then(|region| {
            let map = snapshot
                .label_maps
                .iter()
                .find(|map| map.load_id == region.load_id)?;
            Some((
                Arc::clone(&map.volume),
                map.geometry.clone(),
                map.frame,
                region.label,
            ))
        });
        cx.spawn(async move |this, cx| {
            let histogram = cx
                .background_spawn(async move {
                    let values = Source {
                        volume: &volume,
                        geometry: &geometry,
                        frame,
                    };
                    let mask = labels
                        .as_ref()
                        .map(|(volume, geometry, frame, label)| LabelMask {
                            labels: Source {
                                volume,
                                geometry,
                                frame: *frame,
                            },
                            label: *label,
                        });
                    let progress = Progress::cancellable(cancelled);
                    Histogram::counted(values, counting, mask, &progress)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.counting.remove(&load_id);
                let current = this.snapshot(cx).is_some_and(|snapshot| {
                    snapshot
                        .lines
                        .iter()
                        .any(|line| line.load_id == load_id && line.key == key)
                });
                match histogram {
                    Err(Error::Cancelled) => {}
                    histogram if current => {
                        let histogram = histogram.unwrap_or_else(|error| {
                            eprintln!("Illuminator: Histogram: {error}");
                            None
                        });
                        this.curves.insert(load_id, Curve { key, histogram });
                    }
                    _ => {}
                }
                // Even obsolete work must wake rendering so the current key gets scheduled
                cx.notify();
            });
        })
        .detach();
    }

    /// The counted curves of the layers not hidden in the legend in layer order.
    fn shown<'a>(&'a self, snapshot: &'a Snapshot) -> Vec<(&'a Line, &'a Histogram)> {
        snapshot
            .lines
            .iter()
            .filter(|line| !self.hidden.contains(&line.load_id))
            .filter_map(|line| Some((line, current_curve(&self.curves, line)?.histogram.as_ref()?)))
            .collect()
    }

    fn csv(&self, snapshot: &Snapshot) -> Option<String> {
        if !export_ready(&snapshot.lines, &self.curves, &self.hidden) {
            return None;
        }
        let shown = self.shown(snapshot);
        let curves = shown
            .iter()
            .map(|&(line, histogram)| (line.number, line.name.as_ref(), histogram));
        Some(to_csv(curves))
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

    /// Shows `notice` beside the export buttons for a few seconds
    fn show_notice(&mut self, notice: SharedString, cx: &mut Context<Self>) {
        self.notice = Some(notice);
        // replacing the timer drops the one of an earlier notice
        self.notice_timer = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HISTOGRAM_NOTICE_TIME).await;
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
        // next to the selected layers file, or the first one shown
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
        let name = format!("{}_histogram.csv", strip_extension(&line.name));
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

    fn render_controls(&self, snapshot: Option<&Snapshot>, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let can_export = snapshot
            .is_some_and(|snapshot| export_ready(&snapshot.lines, &self.curves, &self.hidden));
        let this = cx.entity().downgrade();
        let settings = self.settings;
        let bins = match settings.counting.bins {
            None => "Auto".to_string(),
            Some(bins) => bins.to_string(),
        };
        let maps: Vec<(SharedString, u64, Vec<i64>)> = snapshot
            .map(|snapshot| {
                let maps = snapshot.label_maps.iter();
                maps.map(|map| (map.name.clone(), map.load_id, map.labels.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let region = match settings.region {
            None => "Whole volume".to_string(),
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

        h_flex()
            .gap_3()
            .flex_wrap()
            .child(
                Button::new("bins")
                    .xsmall()
                    .outline()
                    .label(format!("Bins: {bins}"))
                    .dropdown_caret(true)
                    .dropdown_menu({
                        let this = this.clone();
                        move |menu, _, _| {
                            let choices = HISTOGRAM_BIN_CHOICES.map(|bins| {
                                let name = bins.map_or("Auto".into(), |bins| bins.to_string());
                                (name, bins)
                            });
                            choice_items(menu, &this, choices, |this, bins, _, cx| {
                                this.settings.counting.bins = bins;
                                cx.notify();
                            })
                        }
                    }),
            )
            .child(
                Button::new("y-axis")
                    .xsmall()
                    .outline()
                    .label(settings.y_axis.name())
                    .dropdown_caret(true)
                    .dropdown_menu({
                        let this = this.clone();
                        move |menu, _, _| {
                            let choices = AxisY::ALL.map(|y_axis| (y_axis.name(), y_axis));
                            choice_items(menu, &this, choices, |this, y_axis, _, cx| {
                                this.settings.y_axis = y_axis;
                                cx.notify();
                            })
                        }
                    }),
            )
            .child(
                Checkbox::new("log")
                    .label("Log scale")
                    .checked(settings.log)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.settings.log = *checked;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("ignore-zeros")
                    .label("Ignore zeros")
                    .checked(settings.counting.ignore_zeros)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.settings.counting.ignore_zeros = *checked;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("region")
                    .xsmall()
                    .outline()
                    .label(format!("Region: {region}"))
                    .dropdown_caret(true)
                    .dropdown_menu(move |menu, _, _| {
                        let mut menu = choice_items(
                            menu.max_h(px(360.0)).scrollable(true),
                            &this,
                            [("Whole volume", None)],
                            |this: &mut Self, region, _, cx| {
                                this.settings.region = region;
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
                                .map(|(text, label)| (text, Some(Region { load_id, label })));
                            menu = choice_items(
                                menu.separator().label(name.clone()),
                                &this,
                                choices,
                                |this, region, _, cx| {
                                    this.settings.region = region;
                                    cx.notify();
                                },
                            );
                        }
                        menu
                    }),
            )
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
                    .tooltip("Copy shown histograms as CSV")
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

    fn render_plot(&self, snapshot: Option<&Snapshot>, cx: &mut Context<Self>) -> AnyElement {
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
            return message("No visible intensity layers");
        }
        let settings = self.settings;
        // selected layer last so it is drawn on top
        let mut shown: Vec<Shown> = self
            .shown(snapshot)
            .into_iter()
            .map(|(line, histogram)| Shown {
                line,
                histogram,
                heights: settings.y_axis.heights(histogram),
            })
            .collect();
        shown.sort_by_key(|shown| shown.line.selected);
        if shown.is_empty() {
            return message(
                if snapshot
                    .lines
                    .iter()
                    .all(|line| self.hidden.contains(&line.load_id))
                {
                    "All layers are hidden"
                } else if snapshot.lines.iter().any(|line| {
                    !self.hidden.contains(&line.load_id)
                        && current_curve(&self.curves, line).is_none()
                }) {
                    if snapshot.playing {
                        "Pause playback to count values"
                    } else {
                        "Counting values\u{2026}"
                    }
                } else {
                    "No values to count"
                },
            );
        }

        let axes = axes(&shown, settings);
        let x_ticks = ticks(axes.x.0, axes.x.1, HISTOGRAM_X_TICKS);
        let y_ticks: Vec<(f32, String)> = if settings.log {
            let decades = axes.y.0.ceil() as i32..=axes.y.1.floor() as i32;
            decades
                .map(|decade| {
                    let decimals = (-decade).max(0) as usize;
                    let label = format!("{:.*}", decimals, 10f64.powi(decade));
                    (axes.y(10f64.powi(decade)), label)
                })
                .collect()
        } else {
            let (heights, decimals) = ticks(axes.y.0, axes.y.1, HISTOGRAM_Y_TICKS);
            heights
                .into_iter()
                .map(|height| (axes.y(height), format!("{height:.decimals$}")))
                .collect()
        };
        let grid: Vec<f32> = y_ticks.iter().map(|&(at, _)| at).collect();
        let cumulative = settings.y_axis == AxisY::Cumulative;
        let strokes: Vec<Stroke> = shown
            .iter()
            .map(|shown| Stroke {
                color: shown.line.color,
                width: if shown.line.selected { 2.0 } else { 1.25 },
                points: steps(&axes, shown.histogram, &shown.heights, cumulative),
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
                let mut path = PathBuilder::stroke(px(stroke.width));
                for (index, &step) in stroke.points.iter().enumerate() {
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
        };
        let bounds = Rc::clone(&self.plot_bounds);
        let mut plot = div()
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

        // the selected layers window
        if let Some(line) = shown
            .iter()
            .map(|shown| shown.line)
            .find(|line| line.selected)
        {
            let (low, high) = line.limits;
            let (left, right) = (axes.x(low).clamp(0.0, 1.0), axes.x(high).clamp(0.0, 1.0));
            let load_id = line.load_id;
            let edge = |edge: WindowEdge, value: f64| {
                let id = match edge {
                    WindowEdge::Low => "window-low",
                    WindowEdge::High => "window-high",
                };
                div()
                    .id(id)
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(axes.x(value).clamp(0.0, 1.0)))
                    .ml(px(-4.0))
                    .w(px(8.0))
                    .flex()
                    .justify_center()
                    .cursor_ew_resize()
                    .child(div().w(px(2.0)).h_full().bg(line.color))
                    .on_drag(DragEdge { load_id, edge }, |drag, _, _, cx| {
                        cx.new(|_| *drag)
                    })
            };
            plot = plot
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(relative(left))
                        .w(relative(right - left))
                        .bg(line.color.opacity(0.1)),
                )
                .child(edge(WindowEdge::Low, low))
                .child(edge(WindowEdge::High, high));
        }

        if let Some(value) = self.hover {
            let at = axes.x(value);
            let rows = shown.iter().rev().map(|shown| {
                let height = height_at(shown.histogram, &shown.heights, value, cumulative);
                let line = shown.line;
                h_flex()
                    .gap_1()
                    .child(div().w(px(8.0)).h(px(8.0)).bg(line.color))
                    .child(format!(
                        "{} {}",
                        line.number,
                        settings.y_axis.format(height)
                    ))
            });
            let readout = v_flex()
                .absolute()
                .top_1()
                .p_1()
                .gap_0p5()
                .bg(background.opacity(0.9))
                .text_xs()
                .child(format_value(value))
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
                let bounds = this.plot_bounds.get();
                let width = f32::from(bounds.size.width);
                if width > 0.0 {
                    let fraction = f32::from(event.position.x - bounds.left()) / width;
                    this.hover = Some(value_at(x, fraction));
                    cx.notify();
                }
            }))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !hovered {
                    this.hover = None;
                    cx.notify();
                }
            }))
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<DragEdge>, _, cx| {
                    let drag = *event.drag(cx);
                    let width = f32::from(event.bounds.size.width);
                    let fraction = f32::from(event.event.position.x - event.bounds.left()) / width;
                    let value = value_at(x, fraction);
                    if let Some(viewer) = this.viewer.upgrade() {
                        viewer.update(cx, |viewer, cx| viewer.move_window_edge(drag, value, cx));
                    }
                }),
            );

        let y_labels = div()
            .relative()
            .w(px(HISTOGRAM_AXIS_WIDTH))
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
        let (values, decimals) = x_ticks;
        let x_labels = div()
            .relative()
            .h(px(HISTOGRAM_AXIS_HEIGHT))
            .ml(px(HISTOGRAM_AXIS_WIDTH))
            .children(values.into_iter().map(|value| {
                div()
                    .absolute()
                    .top_0p5()
                    .left(relative(axes.x(value)))
                    .ml(px(-40.0))
                    .w(px(80.0))
                    .flex()
                    .justify_center()
                    .text_xs()
                    .text_color(muted)
                    .child(format!("{value:.decimals$}"))
            }));
        v_flex()
            .flex_1()
            .min_h_0()
            .child(h_flex().flex_1().min_h_0().child(y_labels).child(plot))
            .child(x_labels)
            .into_any_element()
    }

    fn render_legend(&self, snapshot: Option<&Snapshot>, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let lines = snapshot.map_or(&[][..], |snapshot| &snapshot.lines);
        h_flex()
            .gap_4()
            .flex_wrap()
            .text_xs()
            .children(lines.iter().map(|line| {
                let load_id = line.load_id;
                let hidden = self.hidden.contains(&load_id);
                let note = match current_curve(&self.curves, line) {
                    // hidden layers aren't counted until they are shown
                    None if hidden => "",
                    None if snapshot.is_some_and(|snapshot| snapshot.playing) => {
                        " (pause to count)"
                    }
                    None => " (counting\u{2026})",
                    Some(curve) if curve.histogram.is_none() => " (no values)",
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

impl Render for HistogramWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.snapshot(cx);
        if let Some(snapshot) = &snapshot {
            // the region's label map was closed or isn't a label map anymore
            if let Some(region) = self.settings.region
                && !snapshot
                    .label_maps
                    .iter()
                    .any(|map| map.load_id == region.load_id)
            {
                self.settings.region = None;
                cx.notify();
            }
            self.schedule(snapshot, cx);
        }
        let theme = cx.theme();
        let (background, foreground) = (theme.background, theme.foreground);
        v_flex()
            .size_full()
            .p_3()
            .gap_2()
            .bg(background)
            .text_color(foreground)
            .text_sm()
            .track_focus(&self.focus_handle)
            .key_context(shortcuts::HISTOGRAM_CONTEXT)
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .child(self.render_controls(snapshot.as_ref(), cx))
            .child(self.render_plot(snapshot.as_ref(), cx))
            .child(self.render_legend(snapshot.as_ref(), cx))
    }
}

impl Viewer {
    /// Opens histogram window or brings it to the front.
    pub(super) fn show_histogram(
        &mut self,
        _: &ShowHistogram,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(handle) = self.histogram_window
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let viewer = cx.entity();
        // Opened after this update so the new window can read the viewer when it draws
        cx.defer(move |cx| {
            let (width, height) = HISTOGRAM_WINDOW_SIZE;
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(width), px(height)),
                    cx,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Histogram".into()),
                    ..Default::default()
                }),
                app_id: Some("illuminator".into()),
                ..Default::default()
            };
            let opened = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| HistogramWindow::new(&viewer, window, cx))
            });
            match opened {
                Ok((handle, _)) => {
                    viewer.update(cx, |viewer, _| viewer.histogram_window = Some(handle));
                }
                Err(error) => eprintln!("Illuminator: cannot open the histogram window: {error}"),
            }
        });
    }
}

/// Only curves counted for the desired source and settings may be shown or exported.
fn current_curve<'a>(curves: &'a HashMap<u64, Curve>, line: &Line) -> Option<&'a Curve> {
    curves
        .get(&line.load_id)
        .filter(|curve| curve.key == line.key)
}

fn export_ready(lines: &[Line], curves: &HashMap<u64, Curve>, hidden: &HashSet<u64>) -> bool {
    let mut has_values = false;
    for line in lines.iter().filter(|line| !hidden.contains(&line.load_id)) {
        let Some(curve) = current_curve(curves, line) else {
            return false;
        };
        has_values |= curve.histogram.is_some();
    }
    has_values
}

/// Ranges of the x and y axes that fit every shown curve
fn axes(shown: &[Shown], settings: Settings) -> Axes {
    let low = shown.iter().map(|shown| shown.histogram.min);
    let high = shown.iter().map(|shown| shown.histogram.max);
    let (low, high) = (
        low.fold(f64::INFINITY, f64::min),
        high.fold(f64::NEG_INFINITY, f64::max),
    );
    let heights = || shown.iter().flat_map(|shown| shown.heights.iter().copied());
    let top = heights().fold(0.0, f64::max);
    let y = if settings.log {
        let lowest = heights().filter(|&height| height > 0.0).fold(top, f64::min);
        if top > 0.0 {
            // Make sure single voxels still show
            (lowest.log10().floor() - 0.3, top.log10() + 0.1)
        } else {
            (0.0, 1.0)
        }
    } else {
        (0.0, if top > 0.0 { top * 1.05 } else { 1.0 })
    };
    Axes {
        x: if high > low {
            (low, high)
        } else {
            (low - 0.5, low + 0.5)
        },
        y,
        log: settings.log,
    }
}

/// Value `fraction` of the way along the x axis from `low` to `high`.
fn value_at((low, high): (f64, f64), fraction: f32) -> f64 {
    let fraction = f64::from(fraction);
    let span = high - low;
    if span.is_finite() {
        low + fraction * span
    } else {
        (1.0 - fraction) * low + fraction * high
    }
}

/// Bin readout including the cumulative distributions tails outside its range.
fn height_at(histogram: &Histogram, heights: &[f64], value: f64, cumulative: bool) -> f64 {
    if value.is_nan() || value < histogram.min {
        return 0.0;
    }
    if value >= histogram.max {
        return if cumulative {
            heights.last().copied().unwrap_or(0.0)
        } else {
            0.0
        };
    }
    let bin = ((value - histogram.min) / histogram.bin_width()) as usize;
    heights
        .get(bin.min(heights.len().saturating_sub(1)))
        .copied()
        .unwrap_or(0.0)
}

/// Outline of a histograms bars as fractions of the plot.
fn steps(axes: &Axes, histogram: &Histogram, heights: &[f64], cumulative: bool) -> Vec<(f32, f32)> {
    let width = histogram.bin_width();
    let mut points = Vec::with_capacity(2 * heights.len() + 4);
    if cumulative {
        points.push((0.0, axes.y(0.0)));
    }
    points.push((axes.x(histogram.min), axes.y(0.0)));
    for (bin, &height) in heights.iter().enumerate() {
        let left = histogram.min + bin as f64 * width;
        let right = if bin + 1 == heights.len() {
            histogram.max
        } else {
            histogram.min + (bin + 1) as f64 * width
        };
        let y = axes.y(height);
        points.push((axes.x(left), y));
        points.push((axes.x(right), y));
    }
    if cumulative {
        points.push((1.0, axes.y(heights.last().copied().unwrap_or(0.0))));
    } else {
        points.push((axes.x(histogram.max), axes.y(0.0)));
    }
    points
}

/// Round values from `low` to `high` about `count` and needed decimals.
fn ticks(low: f64, high: f64, count: usize) -> (Vec<f64>, usize) {
    if !(low.is_finite() && high.is_finite() && high > low) {
        return (
            [low, high]
                .into_iter()
                .filter(|value| value.is_finite())
                .take(1)
                .collect(),
            0,
        );
    }
    let span = high - low;
    let fallback = || {
        let mut values = vec![low, value_at((low, high), 0.5), high];
        values.dedup();
        let decimals = if span.is_finite() {
            (-span.log10().floor() + 1.0).clamp(0.0, HISTOGRAM_MAX_TICK_DECIMALS as f64) as usize
        } else {
            0
        };
        (values, decimals)
    };
    if !span.is_finite() {
        return fallback();
    }
    let rough = span / count.clamp(1, HISTOGRAM_MAX_TICKS - 1) as f64;
    if !(rough.is_finite() && rough > 0.0) {
        return fallback();
    }
    let base = 10f64.powf(rough.log10().floor());
    if !(base.is_finite() && base > 0.0) {
        return fallback();
    }
    let Some(step) = [1.0, 2.0, 5.0, 10.0]
        .map(|factor| factor * base)
        .into_iter()
        .find(|&step| step.is_finite() && step > 0.0 && step >= rough)
    else {
        return fallback();
    };
    let decimals = (-step.log10().floor()).clamp(0.0, HISTOGRAM_MAX_TICK_DECIMALS as f64) as usize;
    let (first, last) = ((low / step).ceil(), (high / step).floor());
    let intervals = last - first;
    if !(0.0..HISTOGRAM_MAX_TICKS as f64).contains(&intervals) {
        return fallback();
    }
    let mut values: Vec<f64> = (0..=intervals as usize)
        .map(|offset| (first + offset as f64) * step)
        .filter(|value| value.is_finite() && *value >= low && *value <= high)
        .collect();
    values.dedup();
    if values.is_empty() {
        return fallback();
    }
    (values, decimals)
}

#[cfg(test)]
mod tests {
    use super::{
        Axes, AxisY, Curve, CurveKey, HISTOGRAM_MAX_TICK_DECIMALS, HISTOGRAM_MAX_TICKS, Line,
        RegionKey, current_curve, export_ready, height_at, steps, ticks, value_at,
    };
    use gpui_kit::rgb;
    use illuminator_core::cache::VolumeCache;
    use illuminator_core::display::VolumeDisplay;
    use illuminator_core::histogram::{Counting, Histogram};
    use illuminator_core::loader::VolumeLoader;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    struct TestDirectory(PathBuf);

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn line_fixture() -> (TestDirectory, Line) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = TestDirectory(std::env::temp_dir().join(format!(
            "illuminator-histogram-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        std::fs::create_dir(&directory.0).unwrap();
        let path = directory.0.join("values.nrrd");
        let mut data = b"NRRD0004\ntype: uchar\ndimension: 4\nsizes: 2 1 1 2\nencoding: raw\nspace: RAS\nspace directions: (1,0,0) (0,1,0) (0,0,1) none\n\n".to_vec();
        data.extend_from_slice(&[1, 2, 3, 4]);
        std::fs::write(&path, data).unwrap();
        let (sender, receiver) = mpsc::channel();
        // a cache of its own, not the one the environment points the app at
        let cache = VolumeCache::new(directory.0.join("cache"), 1 << 20).unwrap();
        let loader = VolumeLoader::new(Arc::new(cache), move || {
            let _ = sender.send(());
        });
        let load_id = loader.enqueue(path);
        let deadline = Instant::now() + Duration::from_secs(10);
        let loaded = loop {
            if let Some(loaded) = loader.take_completed().pop() {
                break loaded;
            }
            receiver
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("test volume did not load");
        };
        let display = VolumeDisplay::new(loaded).unwrap();
        let key = CurveKey {
            volume: display.volume().id(),
            frame: display.frame(),
            geometry: display.geometry().clone(),
            counting: Counting {
                bins: Some(32),
                ignore_zeros: true,
            },
            region: None,
        };
        let line = Line {
            load_id,
            number: 1,
            name: "values".into(),
            color: rgb(crate::theme::HISTOGRAM_CURVE_COLORS_DARK[0]).into(),
            volume: display.shared_volume(),
            key,
            limits: display.window_limits(),
            selected: true,
        };
        (directory, line)
    }

    fn histogram() -> Histogram {
        Histogram {
            min: 2.0,
            max: 6.0,
            counts: vec![1, 3],
        }
    }

    #[test]
    fn curve_keys_reject_stale_sources_and_settings() {
        let (_directory, mut line) = line_fixture();
        line.key.region = Some(RegionKey {
            volume: 42,
            frame: 0,
            geometry: line.key.geometry.clone(),
            label: Some(1),
        });
        let key = line.key.clone();
        let mut curves = HashMap::from([(
            line.load_id,
            Curve {
                key: key.clone(),
                histogram: Some(histogram()),
            },
        )]);
        assert!(current_curve(&curves, &line).is_some());
        type KeyChange = (&'static str, fn(&mut CurveKey));
        let changes: [KeyChange; 10] = [
            ("volume", |key| key.volume += 1),
            ("frame", |key| key.frame += 1),
            ("geometry", |key| key.geometry.anatomical ^= true),
            ("bins", |key| key.counting.bins = Some(64)),
            ("zeros", |key| key.counting.ignore_zeros = false),
            ("region removed", |key| key.region = None),
            ("region volume", |key| {
                key.region.as_mut().unwrap().volume += 1
            }),
            ("region frame", |key| {
                key.region.as_mut().unwrap().frame += 1
            }),
            ("region geometry", |key| {
                key.region.as_mut().unwrap().geometry.anatomical ^= true;
            }),
            ("region label", |key| {
                key.region.as_mut().unwrap().label = Some(2);
            }),
        ];
        for (name, change) in changes {
            line.key = key.clone();
            change(&mut line.key);
            assert!(current_curve(&curves, &line).is_none(), "{name}");
        }
        // A cached empty result must also match the current key before claiming no values.
        curves.get_mut(&line.load_id).unwrap().histogram = None;
        assert!(current_curve(&curves, &line).is_none());
        line.key = key;
        assert!(current_curve(&curves, &line).unwrap().histogram.is_none());
        line.load_id += 1;
        assert!(current_curve(&curves, &line).is_none());
    }

    #[test]
    fn export_waits_for_every_nonhidden_desired_curve() {
        let (_directory, first) = line_fixture();
        let second = Line {
            load_id: first.load_id + 1,
            number: 2,
            name: "second".into(),
            color: first.color,
            volume: Arc::clone(&first.volume),
            key: first.key.clone(),
            limits: first.limits,
            selected: true,
        };
        let lines = [first, second];
        let mut curves = HashMap::from([(
            lines[0].load_id,
            Curve {
                key: lines[0].key.clone(),
                histogram: Some(histogram()),
            },
        )]);
        let mut hidden = HashSet::new();
        assert!(!export_ready(&lines, &curves, &hidden));
        hidden.insert(lines[1].load_id);
        assert!(export_ready(&lines, &curves, &hidden));
        hidden.clear();
        let mut stale_key = lines[1].key.clone();
        stale_key.frame += 1;
        curves.insert(
            lines[1].load_id,
            Curve {
                key: stale_key,
                histogram: None,
            },
        );
        assert!(!export_ready(&lines, &curves, &hidden));
        curves.get_mut(&lines[1].load_id).unwrap().histogram = Some(histogram());
        assert!(!export_ready(&lines, &curves, &hidden));
        let second = curves.get_mut(&lines[1].load_id).unwrap();
        second.key = lines[1].key.clone();
        second.histogram = None;
        assert!(export_ready(&lines, &curves, &hidden));
        curves.get_mut(&lines[0].load_id).unwrap().histogram = None;
        assert!(!export_ready(&lines, &curves, &hidden));
        hidden.extend(lines.iter().map(|line| line.load_id));
        assert!(!export_ready(&lines, &curves, &hidden));
        assert!(!export_ready(&[], &curves, &HashSet::new()));
    }

    #[test]
    fn cumulative_lookup_keeps_both_tails() {
        let histogram = histogram();
        let heights = AxisY::Cumulative.heights(&histogram);
        for (value, height) in [
            (f64::NEG_INFINITY, 0.0),
            (1.0, 0.0),
            (2.0, 25.0),
            (3.0, 25.0),
            (4.0, 100.0),
            (f64::from_bits(histogram.max.to_bits() - 1), 100.0),
            (6.0, 100.0),
            (9.0, 100.0),
            (f64::INFINITY, 100.0),
        ] {
            assert_eq!(height_at(&histogram, &heights, value, true), height);
        }
        assert_eq!(height_at(&histogram, &[25.0, 99.0], 6.0, true), 99.0);
        assert_eq!(height_at(&histogram, &[], 6.0, true), 0.0);
        assert_eq!(height_at(&histogram, &heights, f64::NAN, true), 0.0);
    }

    #[test]
    fn ordinary_lookup_is_zero_outside_the_histogram() {
        let histogram = histogram();
        for axis in [AxisY::Count, AxisY::Probability] {
            let heights = axis.heights(&histogram);
            for value in [1.0, 6.0, 9.0] {
                assert_eq!(height_at(&histogram, &heights, value, false), 0.0);
            }
            assert_eq!(height_at(&histogram, &heights, 2.0, false), heights[0]);
            assert_eq!(height_at(&histogram, &heights, 4.0, false), heights[1]);
        }
    }

    #[test]
    fn cumulative_path_extends_across_common_axes() {
        let histogram = histogram();
        let axes = Axes {
            x: (0.0, 10.0),
            y: (0.0, 100.0),
            log: false,
        };
        let heights = AxisY::Cumulative.heights(&histogram);
        assert_eq!(
            steps(&axes, &histogram, &heights, true),
            vec![
                (0.0, 0.0),
                (0.2, 0.0),
                (0.2, 0.25),
                (0.4, 0.25),
                (0.4, 1.0),
                (0.6, 1.0),
                (1.0, 1.0),
            ]
        );
        let log_axes = Axes {
            log: true,
            y: (-1.0, 3.0),
            ..axes
        };
        let points = steps(&log_axes, &histogram, &heights, true);
        assert_eq!(points[0], (0.0, log_axes.y(0.0)));
        assert_eq!(points[1], (0.2, log_axes.y(0.0)));
        assert_eq!(points.last(), Some(&(1.0, log_axes.y(100.0))));
        assert!(points.iter().all(|(x, y)| x.is_finite() && y.is_finite()));
    }

    #[test]
    fn ordinary_path_returns_to_baseline_without_cumulative_tails() {
        let histogram = histogram();
        let axes = Axes {
            x: (0.0, 10.0),
            y: (0.0, 4.0),
            log: false,
        };
        let heights = AxisY::Count.heights(&histogram);
        assert_eq!(
            steps(&axes, &histogram, &heights, false),
            vec![
                (0.2, 0.0),
                (0.2, 0.25),
                (0.4, 0.25),
                (0.4, 0.75),
                (0.6, 0.75),
                (0.6, 0.0),
            ]
        );
    }

    #[test]
    fn wide_axes_and_interpolation_remain_finite() {
        for range in [(-f64::MAX, f64::MAX), (-f64::MAX, f64::MAX * 0.75)] {
            assert!(!(range.1 - range.0).is_finite());
            let axes = Axes {
                x: range,
                y: (0.0, 1.0),
                log: false,
            };
            for fraction in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let value = value_at(range, fraction);
                assert!(value.is_finite());
                assert!((range.0..=range.1).contains(&value));
                let mapped = axes.x(value);
                assert!(mapped.is_finite());
                assert!((mapped - fraction).abs() <= f32::EPSILON);
            }
            assert_eq!(axes.x(range.0), 0.0);
            assert_eq!(axes.x(range.1), 1.0);
            assert_eq!(value_at(range, 0.0), range.0);
            assert_eq!(value_at(range, 1.0), range.1);
        }
        let axes = Axes {
            x: (-f64::MAX, f64::MAX),
            y: (0.0, 1.0),
            log: false,
        };
        assert_eq!(axes.x(0.0), 0.5);
        assert_eq!(value_at(axes.x, 0.5), 0.0);
        for (min, max) in [(-f64::MAX, -f64::MAX * 0.5), (f64::MAX * 0.5, f64::MAX)] {
            let histogram = Histogram {
                min,
                max,
                counts: vec![1],
            };
            assert!((max - min).is_finite());
            assert!(
                steps(&axes, &histogram, &[1.0], false)
                    .iter()
                    .all(|(x, y)| x.is_finite() && y.is_finite())
            );
        }
    }

    #[test]
    fn subnormal_ticks_and_mapping_are_bounded() {
        let tiny = f64::from_bits(1);
        let range = (0.0, 2.0 * tiny);
        let axes = Axes {
            x: range,
            y: (0.0, 1.0),
            log: false,
        };
        assert_eq!(axes.x(tiny), 0.5);
        assert_eq!(value_at(range, 0.5), tiny);
        for (low, high, count) in [
            (range.0, range.1, 8), // rough underflows
            (0.0, tiny, 1),        // decimal base underflows
            (-tiny, tiny, 8),
            (0.0, 100.0 * tiny, 8),
        ] {
            let (values, decimals) = ticks(low, high, count);
            assert!(!values.is_empty());
            assert!(values.len() <= HISTOGRAM_MAX_TICKS);
            assert!(decimals <= HISTOGRAM_MAX_TICK_DECIMALS);
            assert!(
                values
                    .iter()
                    .all(|value| value.is_finite() && (low..=high).contains(value))
            );
            assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
            for value in values {
                let label = format!("{value:.decimals$}");
                assert!(label.len() <= HISTOGRAM_MAX_TICK_DECIMALS + 3);
                let parsed = label.parse::<f64>().unwrap();
                assert!(parsed.is_finite());
                if value != 0.0 {
                    assert_ne!(parsed, 0.0);
                }
            }
        }
        let (values, decimals) = ticks(range.0, range.1, 8);
        assert_eq!(values, vec![0.0, tiny, 2.0 * tiny]);
        for value in values {
            assert_eq!(format!("{value:.decimals$}").parse::<f64>().unwrap(), value);
        }
    }

    #[test]
    fn extreme_tick_requests_remain_bounded_and_finite() {
        for (low, high, count) in [
            (-f64::MAX, f64::MAX, 8),
            (0.0, f64::MAX, 1), // no finite nice step large enough
            (0.0, 100.0, 0),
            (0.0, 100.0, usize::MAX),
            (1e300, f64::from_bits(1e300_f64.to_bits() + 1), 8),
        ] {
            let (values, decimals) = ticks(low, high, count);
            assert!(!values.is_empty());
            assert!(values.len() <= HISTOGRAM_MAX_TICKS);
            assert!(decimals <= HISTOGRAM_MAX_TICK_DECIMALS);
            assert!(
                values
                    .iter()
                    .all(|value| value.is_finite() && (low..=high).contains(value))
            );
            for value in values {
                assert!(format!("{value:.decimals$}").len() <= 635);
            }
        }
        let axes = Axes {
            x: (10.0, 30.0),
            y: (0.0, 1.0),
            log: false,
        };
        assert_eq!(axes.x(15.0), 0.25);
        assert_eq!(value_at(axes.x, 0.25), 15.0);
        assert_eq!(value_at(axes.x, 1.5), 40.0);
    }

    #[test]
    fn ticks_are_round() {
        assert_eq!(
            ticks(0.0, 100.0, 5),
            (vec![0.0, 20.0, 40.0, 60.0, 80.0, 100.0], 0)
        );
        assert_eq!(ticks(-1040.5, 3080.5, 8).0[0], -1000.0);
        let (small, decimals) = ticks(0.0, 0.05, 5);
        assert_eq!((small.len(), decimals), (6, 2));
        assert_eq!(ticks(3.0, 3.0, 5), (vec![3.0], 0));
    }
}
