//! Time series window.
//! The values of a region in every frame of the visible 4D layers.

mod series;

use gpui_kit::component::button::Button;
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::{ActiveTheme, Sizable};
use gpui_kit::{
    App, Context, Div, InteractiveElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    ParentElement, Pixels, Stateful, Styled, Window, div, px, relative,
};
use illuminator_core::display::VolumeDisplay;
use illuminator_core::geometry::VolumeGeometry;
use illuminator_core::progress::Progress;
use illuminator_core::statistics::ValueSummary;
use illuminator_core::time_series::{RegionOfInterest, time_series, to_csv};
use illuminator_core::volume::Volume;
use std::sync::Arc;

use crate::shortcuts::ShowTimeSeriesWindow;
use crate::ui::{add_choice_items, format_value};

use self::series::{curve_segments, padded_value_range};
use super::Viewer;
use super::plot_window::{
    Axes, Chart, LabelMap, MeasurementJob, Plot, PlotLayer, PlotStrings, PlotWindow, PlottedValue,
    RegionKey, ViewerSnapshot, tick_values,
};

const TIME_SERIES_X_TICKS: usize = 8;
const TIME_SERIES_Y_TICKS: usize = 5;
const CURSOR_CUBE_HALF_WIDTHS: [usize; 4] = [0, 1, 2, 3];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Statistic {
    Mean,
    Minimum,
    Maximum,
}

impl Statistic {
    const ALL: [Self; 3] = [Self::Mean, Self::Minimum, Self::Maximum];

    fn name(self) -> &'static str {
        match self {
            Self::Mean => "Mean",
            Self::Minimum => "Min",
            Self::Maximum => "Max",
        }
    }

    fn value_of(self, summary: &ValueSummary) -> f64 {
        match self {
            Self::Mean => summary.mean,
            Self::Minimum => summary.minimum,
            Self::Maximum => summary.maximum,
        }
    }
}

fn cursor_region_name(half_width: usize) -> String {
    match half_width {
        0 => "Cursor voxel".into(),
        _ => {
            let side = 2 * half_width + 1;
            format!("{side}\u{d7}{side}\u{d7}{side} at cursor")
        }
    }
}

/// What a series is measured from.
#[derive(Clone, PartialEq)]
pub(super) struct TimeSeriesKey {
    volume: u64,
    geometry: VolumeGeometry,
    region: RegionOfInterestKey,
}

#[derive(Clone, PartialEq)]
enum RegionOfInterestKey {
    Cube { center: [usize; 3], radius: usize },
    LabelMask(Box<RegionKey>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct TimeSeriesPlot {
    half_width: usize, // Of the cube around the cursor
    statistic: Statistic,
}

impl Plot for TimeSeriesPlot {
    type Key = TimeSeriesKey;
    type Value = Vec<Option<ValueSummary>>;

    const STRINGS: PlotStrings = PlotStrings {
        title: "Time series",
        csv_name: "time_series",
        copy_tooltip: "Copy shown series as CSV",
        no_layers: "No visible 4D intensity layers",
        measuring: "Measuring\u{2026}",
        measuring_note: " (measuring\u{2026})",
        waiting: "Pause playback to measure",
        waiting_note: " (pause to measure)",
        nothing_to_measure: "The cursor is outside the layers",
        nothing_to_measure_note: " (cursor outside)",
        no_values: "No values in this region",
    };
    const REQUIRES_4D: bool = true;

    fn key(&self, display: &VolumeDisplay, region: Option<&RegionKey>) -> Option<TimeSeriesKey> {
        let region = match region {
            Some(region) => RegionOfInterestKey::LabelMask(Box::new(region.clone())),
            None => {
                let (center, _) = display.value_at_cursor()?;
                RegionOfInterestKey::Cube {
                    center,
                    radius: self.half_width,
                }
            }
        };
        Some(TimeSeriesKey {
            volume: display.volume().id(),
            geometry: display.geometry().clone(),
            region,
        })
    }

    fn measurement_job(
        key: &TimeSeriesKey,
        volume: Arc<Volume>,
        label_maps: &[LabelMap],
    ) -> Option<MeasurementJob<Self::Value>> {
        let geometry = key.geometry.clone();
        let job: MeasurementJob<Self::Value> = match &key.region {
            &RegionOfInterestKey::Cube { center, radius } => {
                Box::new(move |progress: &Progress| {
                    let region = RegionOfInterest::Cube {
                        center,
                        half_width: radius,
                    };
                    time_series(&volume, &geometry, region, progress).map(series_if_any_values)
                })
            }
            RegionOfInterestKey::LabelMask(region) => {
                let labels = region.resolve(label_maps)?;
                Box::new(move |progress: &Progress| {
                    let region = RegionOfInterest::LabelMask(labels.mask());
                    time_series(&volume, &geometry, region, progress).map(series_if_any_values)
                })
            }
        };
        Some(job)
    }

    fn csv(exported: &[(&PlotLayer<TimeSeriesKey>, &Self::Value)]) -> String {
        to_csv(
            exported
                .iter()
                .map(|&(layer, frames)| (layer.number, layer.name.as_ref(), &frames[..])),
        )
    }

    fn controls(
        window: &PlotWindow<Self>,
        controls: Div,
        snapshot: &ViewerSnapshot<TimeSeriesKey>,
        cx: &mut Context<PlotWindow<Self>>,
    ) -> Div {
        let this = cx.entity().downgrade();
        let plot = window.plot;
        let cubes = CURSOR_CUBE_HALF_WIDTHS.map(|radius| (cursor_region_name(radius), radius));
        controls
            .child(window.region_button(
                "Region",
                cursor_region_name(plot.half_width),
                cubes.to_vec(),
                |plot: &mut Self, radius| plot.half_width = radius,
                snapshot,
                cx,
            ))
            .child(
                Button::new("statistic")
                    .xsmall()
                    .outline()
                    .label(plot.statistic.name())
                    .dropdown_caret(true)
                    .dropdown_menu(move |menu, _, _| {
                        let choices = Statistic::ALL.map(|statistic| (statistic.name(), statistic));
                        add_choice_items(menu, &this, choices, |this, statistic, _, cx| {
                            this.plot.statistic = statistic;
                            cx.notify();
                        })
                    }),
            )
    }

    fn chart(
        &self,
        plotted: &[PlottedValue<Self>],
        snapshot: &ViewerSnapshot<TimeSeriesKey>,
    ) -> Chart {
        let values: Vec<Vec<Option<f64>>> = plotted
            .iter()
            .map(|plotted| self.values(plotted.value))
            .collect();
        let (low, high) = padded_value_range(values.iter().flatten()).unwrap_or((0.0, 1.0));
        let frames = maximum_frame_count(snapshot);
        let axes = Axes {
            x: (0.0, (frames - 1) as f64),
            y: (low, high),
            logarithmic: false,
        };
        // whole frame numbers only
        let (numbers, _) = tick_values(1.0, frames as f64, TIME_SERIES_X_TICKS);
        let x_ticks = numbers
            .into_iter()
            .filter(|number| number.fract() == 0.0)
            .map(|number| (axes.x_fraction(number - 1.0), format!("{number:.0}")))
            .collect();
        let (heights, decimals) = tick_values(low, high, TIME_SERIES_Y_TICKS);
        let y_ticks = heights
            .into_iter()
            .map(|height| (axes.y_fraction(height), format!("{height:.decimals$}")))
            .collect();
        let curves = values
            .iter()
            .map(|values| curve_segments(&axes, values))
            .collect();
        Chart {
            axes,
            x_ticks,
            y_ticks,
            curves,
        }
    }

    fn overlay(
        &self,
        plot: Stateful<Div>,
        axes: &Axes,
        _: &[PlottedValue<Self>],
        snapshot: &ViewerSnapshot<TimeSeriesKey>,
        cx: &mut Context<PlotWindow<Self>>,
    ) -> Stateful<Div> {
        let info = cx.theme().info;
        let x = axes.x;
        let frame_at = move |this: &PlotWindow<Self>, position: Pixels| {
            Some(this.value_at_position(position, x)?.round() as usize)
        };
        plot.cursor_pointer()
            // the viewer's current frame
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(axes.x_fraction(snapshot.frame as f64)))
                    .w(px(1.0))
                    .bg(info),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    if let Some(frame) = frame_at(this, event.position.x) {
                        go_to_frame(this, frame, cx);
                    }
                }),
            )
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button == Some(MouseButton::Left)
                    && let Some(frame) = frame_at(this, event.position.x)
                {
                    go_to_frame(this, frame, cx);
                }
            }))
    }

    fn quantize_x(x: f64) -> f64 {
        x.round()
    }

    fn readout_title(x: f64) -> String {
        format!("Frame {}", x as usize + 1)
    }

    fn readout(&self, frames: &Self::Value, x: f64) -> String {
        frames
            .get(x as usize)
            .and_then(Option::as_ref)
            .map_or("\u{2013}".into(), |summary| {
                format_value(self.statistic.value_of(summary))
            })
    }
}

impl TimeSeriesPlot {
    /// The chosen statistic of each frame
    fn values(&self, frames: &[Option<ValueSummary>]) -> Vec<Option<f64>> {
        frames
            .iter()
            .map(|frame| {
                frame
                    .as_ref()
                    .map(|summary| self.statistic.value_of(summary))
            })
            .collect()
    }
}

impl Viewer {
    pub(super) fn show_time_series(
        &mut self,
        _: &ShowTimeSeriesWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let plot = TimeSeriesPlot {
            half_width: 0,
            statistic: Statistic::Mean,
        };
        self.show_plot_window(plot, window, cx);
    }
}

fn go_to_frame(window: &PlotWindow<TimeSeriesPlot>, frame: usize, cx: &mut App) {
    window.update_viewer(cx, |viewer, window, cx| {
        viewer.go_to_frame(frame, window, cx)
    });
}

/// The most frames of any layer (at least 2)
fn maximum_frame_count(snapshot: &ViewerSnapshot<TimeSeriesKey>) -> usize {
    snapshot
        .layers
        .iter()
        .map(|layer| layer.volume.metadata().timepoints())
        .fold(2, usize::max)
}

fn series_if_any_values(frames: Vec<Option<ValueSummary>>) -> Option<Vec<Option<ValueSummary>>> {
    frames.iter().any(Option::is_some).then_some(frames)
}
