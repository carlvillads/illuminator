//! Plot windows.
//! A plot of the visible layers in a window of its own beside the viewer.
//! The histogram and time series windows are kinds of [`Plot`].
//!
//! Each visible intensity layer is a [`PlotLayer`]. Its value is measured in the background
//! for a key, and a [`Chart`] draws each [`Plotted`] value as a [`Curve`].
//!
//! - `measuring`: measuring layers and keeping track of which values are current
//! - `regions`: measuring inside a label map instead of the plots own area
//! - `export`: copying and saving the values as CSV
//! - `render`: the controls, the plot and the legend
//! - `axes`: where values land in the plot

mod axes;
mod export;
mod measuring;
mod regions;
mod render;
#[cfg(test)]
pub(super) mod test_util;

use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Context, Div, Entity, FocusHandle, Hsla, Pixels,
    SharedString, Stateful, Subscription, Task, TitlebarOptions, WeakEntity, Window, WindowBounds,
    WindowOptions, px, size,
};
use illuminator_core::display::VolumeDisplay;
use illuminator_core::geometry::GeometryOptions;
use illuminator_core::volume::Volume;
use std::any::TypeId;
use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use crate::theme;

use self::measuring::Measurements;
use self::regions::LabelRegion;
use super::Viewer;

pub(super) use self::axes::{Axes, ticks, value_at};
pub(super) use self::measuring::Job;
pub(super) use self::regions::{LabelArea, LabelMap, RegionKey};

const PLOT_WINDOW_SIZE: (f32, f32) = (760.0, 480.0);

/// Points as fractions of the plot from bottom-left
pub(super) type Polyline = Vec<(f32, f32)>;

/// One layer's curve, in pieces
pub(super) type Curve = Vec<Polyline>;

/// The words a plot window uses
pub(super) struct Texts {
    pub(super) title: &'static str,
    /// Ends the names of saved CSV files
    pub(super) csv_name: &'static str,
    pub(super) copy_tooltip: &'static str,
    pub(super) no_layers: &'static str,
    /// In the plot, and after a layer's name in the legend, while it is measured
    pub(super) measuring: &'static str,
    pub(super) measuring_note: &'static str,
    /// The same while measuring waits for the movie to pause
    pub(super) waiting: &'static str,
    pub(super) waiting_note: &'static str,
    /// The same for layers with nothing to measure
    pub(super) nothing_to_measure: &'static str,
    pub(super) nothing_to_measure_note: &'static str,
    pub(super) no_values: &'static str,
}

/// What a plot window measures of each layer and how to draw it.
pub(super) trait Plot: Sized + 'static {
    /// What a layer's value is measured from
    type Key: Clone + PartialEq + 'static;
    type Value: Send + 'static;

    const TEXTS: Texts;
    const ONLY_4D: bool = false;
    const WAITS_FOR_PAUSE: bool = false;

    /// What to measure of a layer, inside `region` when there is one.
    /// None if there is nothing to measure.
    fn key(&self, display: &VolumeDisplay, region: Option<&RegionKey>) -> Option<Self::Key>;

    /// None if `key` can't be measured
    fn job(
        key: &Self::Key,
        volume: Arc<Volume>,
        label_maps: &[LabelMap],
    ) -> Option<Job<Self::Value>>;

    fn shows_stale(_old: &Self::Key, _new: &Self::Key) -> bool {
        true
    }

    fn csv(exported: &[(&PlotLayer<Self::Key>, &Self::Value)]) -> String;

    /// Adds the plots settings to `controls`
    fn controls(
        window: &PlotWindow<Self>,
        controls: Div,
        snapshot: &Snapshot<Self::Key>,
        cx: &mut Context<PlotWindow<Self>>,
    ) -> Div;

    fn chart(&self, plotted: &[Plotted<Self>], snapshot: &Snapshot<Self::Key>) -> Chart;

    /// Adds the plots marks and mouse handling on top of curves
    fn overlay(
        &self,
        plot: Stateful<Div>,
        axes: &Axes,
        plotted: &[Plotted<Self>],
        snapshot: &Snapshot<Self::Key>,
        cx: &mut Context<PlotWindow<Self>>,
    ) -> Stateful<Div>;

    fn snap_x(x: f64) -> f64 {
        x
    }

    fn readout_title(x: f64) -> String;

    /// Layers `value` at `x`
    fn readout(&self, value: &Self::Value, x: f64) -> String;
}

/// What a plot shows of the plotted layers
pub(super) struct Chart {
    pub(super) axes: Axes,
    pub(super) x_ticks: Vec<(f32, String)>,
    pub(super) y_ticks: Vec<(f32, String)>,
    pub(super) curves: Vec<Curve>,
}

/// A visible intensity layer as a plot window shows it
pub(super) struct PlotLayer<K> {
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
    pub(super) layer: &'a PlotLayer<P::Key>,
    pub(super) value: &'a P::Value,
    /// Measured for an out of date key
    pub(super) stale: bool,
}

/// The viewers layers as the window needs them
pub(super) struct Snapshot<K> {
    pub(super) layers: Vec<PlotLayer<K>>,
    label_maps: Vec<LabelMap>,
    pub(super) playing: bool,
    pub(super) frame: usize,
}

impl<K: PartialEq> Snapshot<K> {
    /// Whether the layer `load_id` still wants a value for `key`
    fn wants(&self, load_id: u64, key: &K) -> bool {
        self.layers
            .iter()
            .any(|layer| layer.load_id == load_id && layer.key.as_ref() == Some(key))
    }
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
    measurements: Measurements<P::Key, P::Value>,
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
            measurements: Measurements::default(),
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
            .map(LabelMap::of)
            .collect();
        let region = self.region_key(&label_maps);
        let layers = viewer
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
                PlotLayer {
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
            layers,
            label_maps,
            playing: viewer.time_playing.is_some(),
            frame: viewer.frame,
        })
    }

    /// Layers not hidden in the legend
    fn unhidden<'s>(
        &self,
        snapshot: &'s Snapshot<P::Key>,
    ) -> impl Iterator<Item = &'s PlotLayer<P::Key>> {
        snapshot
            .layers
            .iter()
            .filter(|layer| !self.hidden.contains(&layer.load_id))
    }

    /// Whether measuring waits for the movie to pause
    fn waiting(&self, snapshot: &Snapshot<P::Key>) -> bool {
        P::WAITS_FOR_PAUSE && snapshot.playing
    }

    /// Changes the viewer in its own window
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
                    title: Some(P::TEXTS.title.into()),
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
                    P::TEXTS.title.to_lowercase()
                ),
            }
        });
    }
}
