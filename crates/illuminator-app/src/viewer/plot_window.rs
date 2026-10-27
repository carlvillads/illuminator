//! Plot windows.
//! A plot of the visible layers in a window of its own beside the viewer.
//! The histogram and time series windows are kinds of [`Plot`].
//!
//! Each visible intensity layer is a [`PlotLayer`]. Its value is measured in the background
//! for a key, and a [`Chart`] draws each [`PlottedValue`] as a [`Curve`].
//!
//! - `measuring`: measuring layers and keeping track of which values are current
//! - `regions`: measuring inside a label map instead of the plot's own region
//! - `export`: copying and saving the values as CSV
//! - `render`: the controls, the plot and the legend
//! - `axes`: where values land in the plot

mod axes;
mod export;
mod measuring;
mod regions;
mod render;
#[cfg(test)]
pub(super) mod test_fixtures;

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

pub(super) use self::axes::{Axes, tick_values, value_at};
pub(super) use self::measuring::MeasurementJob;
pub(super) use self::regions::{LabelMap, RegionKey, ResolvedLabelRegion};

const PLOT_WINDOW_SIZE: (f32, f32) = (760.0, 480.0);

/// Points as fractions of the plot from bottom-left
pub(super) type Polyline = Vec<(f32, f32)>;

/// One layer's curve, in pieces
pub(super) type Curve = Vec<Polyline>;

/// The words a plot window uses
pub(super) struct PlotStrings {
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

    const STRINGS: PlotStrings;
    const REQUIRES_4D: bool = false;
    const WAITS_FOR_PAUSE: bool = false;

    /// What to measure of a layer, inside `region` when there is one.
    /// None if there is nothing to measure.
    fn key(&self, display: &VolumeDisplay, region: Option<&RegionKey>) -> Option<Self::Key>;

    /// None if `key` can't be measured
    fn measurement_job(
        key: &Self::Key,
        volume: Arc<Volume>,
        label_maps: &[LabelMap],
    ) -> Option<MeasurementJob<Self::Value>>;

    fn shows_stale(_old: &Self::Key, _new: &Self::Key) -> bool {
        true
    }

    fn csv(exported: &[(&PlotLayer<Self::Key>, &Self::Value)]) -> String;

    /// Adds the plots settings to `controls`
    fn controls(
        window: &PlotWindow<Self>,
        controls: Div,
        snapshot: &ViewerSnapshot<Self::Key>,
        cx: &mut Context<PlotWindow<Self>>,
    ) -> Div;

    fn chart(&self, plotted: &[PlottedValue<Self>], snapshot: &ViewerSnapshot<Self::Key>) -> Chart;

    /// Adds the plots marks and mouse handling on top of curves
    fn overlay(
        &self,
        plot: Stateful<Div>,
        axes: &Axes,
        plotted: &[PlottedValue<Self>],
        snapshot: &ViewerSnapshot<Self::Key>,
        cx: &mut Context<PlotWindow<Self>>,
    ) -> Stateful<Div>;

    fn quantize_x(x: f64) -> f64 {
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
pub(super) struct PlottedValue<'a, P: Plot> {
    pub(super) layer: &'a PlotLayer<P::Key>,
    pub(super) value: &'a P::Value,
    /// Measured for an out of date key
    pub(super) stale: bool,
}

/// The viewers layers as the window needs them
pub(super) struct ViewerSnapshot<K> {
    pub(super) layers: Vec<PlotLayer<K>>,
    label_maps: Vec<LabelMap>,
    pub(super) playback_active: bool,
    pub(super) frame: usize,
}

impl<K: PartialEq> ViewerSnapshot<K> {
    /// Whether the layer `load_id` still wants a value for `key`
    fn requires_value(&self, load_id: u64, key: &K) -> bool {
        self.layers
            .iter()
            .any(|layer| layer.load_id == load_id && layer.key.as_ref() == Some(key))
    }
}

/// What the window shows of the viewer. Only redraws when this changes
#[derive(PartialEq)]
struct ObservedViewerState {
    layers: Vec<ObservedLayerState>,
    selected: Option<usize>,
    playback_active: bool,
    frame: usize,
}

#[derive(PartialEq)]
struct ObservedLayerState {
    load_id: u64,
    visible: bool,
    label_mode: bool,
    volume: u64,
    frame: usize,
    options: GeometryOptions,
    limits: (f64, f64),
    cursor: Option<[usize; 3]>,
}

impl ObservedViewerState {
    fn of(viewer: &Viewer) -> Self {
        let layers = viewer.layers.iter().map(|layer| ObservedLayerState {
            load_id: layer.load_id,
            visible: layer.visible,
            label_mode: layer.display.label_mode(),
            volume: layer.display.volume().id(),
            frame: layer.display.frame(),
            options: layer.display.options(),
            limits: layer.display.window_limits(),
            cursor: layer.display.value_at_cursor().map(|(voxel, _)| voxel),
        });
        Self {
            layers: layers.collect(),
            selected: viewer.selected_layer(),
            playback_active: viewer.playback_task.is_some(),
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
    hovered_x: Option<f64>,
    plot_bounds: Rc<Cell<Bounds<Pixels>>>,
    notice: Option<SharedString>,
    notice_timer: Task<()>,
    observed_state: Option<ObservedViewerState>,
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
            hovered_x: None,
            plot_bounds: Rc::default(),
            notice: None,
            notice_timer: Task::ready(()),
            observed_state: None,
            focus_handle,
            _viewer_changes: cx.observe(viewer, |this, viewer, cx| {
                let observed_state = ObservedViewerState::of(viewer.read(cx));
                if this.observed_state.as_ref() != Some(&observed_state) {
                    this.observed_state = Some(observed_state);
                    cx.notify();
                }
            }),
            _viewer_closed: cx.observe_release_in(viewer, window, |_, _, window, _| {
                window.remove_window();
            }),
        }
    }

    fn snapshot(&self, cx: &App) -> Option<ViewerSnapshot<P::Key>> {
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
                    && (!P::REQUIRES_4D || display.volume().metadata().timepoints() > 1)
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
                    selected: viewer.selected_layer() == Some(index),
                }
            })
            .collect();
        Some(ViewerSnapshot {
            layers,
            label_maps,
            playback_active: viewer.playback_task.is_some(),
            frame: viewer.frame,
        })
    }

    /// Layers not hidden in the legend
    fn legend_visible_layers<'s>(
        &self,
        snapshot: &'s ViewerSnapshot<P::Key>,
    ) -> impl Iterator<Item = &'s PlotLayer<P::Key>> {
        snapshot
            .layers
            .iter()
            .filter(|layer| !self.hidden.contains(&layer.load_id))
    }

    /// Whether measuring waits for the movie to pause
    fn is_waiting_for_pause(&self, snapshot: &ViewerSnapshot<P::Key>) -> bool {
        P::WAITS_FOR_PAUSE && snapshot.playback_active
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
    pub(super) fn value_at_position(&self, x: Pixels, (low, high): (f64, f64)) -> Option<f64> {
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
        let plot_type = TypeId::of::<P>();
        if let Some(handle) = self.plot_windows.get(&plot_type).copied()
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
                    title: Some(P::STRINGS.title.into()),
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
                    viewer.plot_windows.insert(plot_type, handle);
                }),
                Err(error) => eprintln!(
                    "Illuminator: cannot open the {} window: {error}",
                    P::STRINGS.title.to_lowercase()
                ),
            }
        });
    }
}
