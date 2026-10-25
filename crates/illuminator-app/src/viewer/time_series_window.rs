//! Time series window.
//! The values of an area in every frame of the visible 4D layers.

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
use illuminator_core::stats::Summary;
use illuminator_core::time_series::{Area, time_series, to_csv};
use illuminator_core::volume::Volume;
use std::sync::Arc;

use crate::shortcuts::ShowTimeSeries;
use crate::ui::{choice_items, format_value};

use super::Viewer;
use super::plot_window::{
    Axes, Chart, Job, LabelMap, Line, Plot, PlotWindow, Plotted, Polyline, RegionKey, Snapshot,
    ticks,
};

const TIME_SERIES_X_TICKS: usize = 8;
const TIME_SERIES_Y_TICKS: usize = 5;
const CURSOR_CUBE_RADII: [usize; 4] = [0, 1, 2, 3];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Statistic {
    Mean,
    Min,
    Max,
}

impl Statistic {
    const ALL: [Self; 3] = [Self::Mean, Self::Min, Self::Max];

    fn name(self) -> &'static str {
        match self {
            Self::Mean => "Mean",
            Self::Min => "Min",
            Self::Max => "Max",
        }
    }

    fn of(self, summary: &Summary) -> f64 {
        match self {
            Self::Mean => summary.mean,
            Self::Min => summary.min,
            Self::Max => summary.max,
        }
    }
}

fn cursor_name(radius: usize) -> String {
    match radius {
        0 => "Cursor voxel".into(),
        _ => {
            let side = 2 * radius + 1;
            format!("{side}\u{d7}{side}\u{d7}{side} at cursor")
        }
    }
}

/// What a series is measured from.
#[derive(Clone, PartialEq)]
pub(super) struct SeriesKey {
    volume: u64,
    geometry: VolumeGeometry,
    area: AreaKey,
}

#[derive(Clone, PartialEq)]
enum AreaKey {
    Cube { center: [usize; 3], radius: usize },
    Labels(Box<RegionKey>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct TimeSeriesPlot {
    radius: usize, // Of the cube around the cursor
    statistic: Statistic,
}

impl Plot for TimeSeriesPlot {
    type Key = SeriesKey;
    type Value = Vec<Option<Summary>>;

    const TITLE: &'static str = "Time series";
    const CSV_NAME: &'static str = "time_series";
    const COPY_TOOLTIP: &'static str = "Copy shown series as CSV";
    const MEASURING: &'static str = "Measuring\u{2026}";
    const MEASURING_NOTE: &'static str = " (measuring\u{2026})";
    const NO_VALUES: &'static str = "No values in this area";
    const ONLY_4D: bool = true;

    fn key(&self, display: &VolumeDisplay, region: Option<&RegionKey>) -> Option<SeriesKey> {
        let area = match region {
            Some(region) => AreaKey::Labels(Box::new(region.clone())),
            None => {
                let (center, _) = display.cursor_sample()?;
                AreaKey::Cube {
                    center,
                    radius: self.radius,
                }
            }
        };
        Some(SeriesKey {
            volume: display.volume().id(),
            geometry: display.geometry().clone(),
            area,
        })
    }

    fn job(
        key: &SeriesKey,
        volume: Arc<Volume>,
        label_maps: &[LabelMap],
    ) -> Option<Job<Self::Value>> {
        let geometry = key.geometry.clone();
        let job: Job<Self::Value> = match &key.area {
            &AreaKey::Cube { center, radius } => Box::new(move |progress: &Progress| {
                let area = Area::Cube { center, radius };
                time_series(&volume, &geometry, area, progress).map(with_values)
            }),
            AreaKey::Labels(region) => {
                let region = region.job(label_maps)?;
                Box::new(move |progress: &Progress| {
                    let area = Area::Labels(region.mask());
                    time_series(&volume, &geometry, area, progress).map(with_values)
                })
            }
        };
        Some(job)
    }

    fn csv(shown: &[(&Line<SeriesKey>, &Self::Value)]) -> String {
        to_csv(
            shown
                .iter()
                .map(|&(line, frames)| (line.number, line.name.as_ref(), &frames[..])),
        )
    }

    fn controls(
        window: &PlotWindow<Self>,
        controls: Div,
        snapshot: Option<&Snapshot<SeriesKey>>,
        cx: &mut Context<PlotWindow<Self>>,
    ) -> Div {
        let this = cx.entity().downgrade();
        let plot = window.plot;
        let cubes = CURSOR_CUBE_RADII.map(|radius| (cursor_name(radius), radius));
        controls
            .child(window.region_button(
                "Area",
                cursor_name(plot.radius),
                cubes.to_vec(),
                |plot: &mut Self, radius| plot.radius = radius,
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
                        choice_items(menu, &this, choices, |this, statistic, _, cx| {
                            this.plot.statistic = statistic;
                            cx.notify();
                        })
                    }),
            )
    }

    fn chart(&self, plotted: &[Plotted<Self>], snapshot: &Snapshot<SeriesKey>) -> Chart {
        let values: Vec<Vec<Option<f64>>> = plotted
            .iter()
            .map(|plotted| self.values(plotted.value))
            .collect();
        let (low, high) = value_range(values.iter().flatten()).unwrap_or((0.0, 1.0));
        let frames = frames(snapshot);
        let axes = Axes {
            x: (0.0, (frames - 1) as f64),
            y: (low, high),
            log: false,
        };
        // whole frame numbers only
        let (numbers, _) = ticks(1.0, frames as f64, TIME_SERIES_X_TICKS);
        let x_ticks = numbers
            .into_iter()
            .filter(|number| number.fract() == 0.0)
            .map(|number| (axes.x(number - 1.0), format!("{number:.0}")))
            .collect();
        let (heights, decimals) = ticks(low, high, TIME_SERIES_Y_TICKS);
        let y_ticks = heights
            .into_iter()
            .map(|height| (axes.y(height), format!("{height:.decimals$}")))
            .collect();
        let curves = values
            .iter()
            .map(|values| segments(&axes, values))
            .collect();
        Chart {
            axes,
            x_ticks,
            y_ticks,
            curves,
        }
    }

    fn snap(x: f64) -> f64 {
        x.round()
    }

    fn x_name(x: f64) -> String {
        format!("Frame {}", x as usize + 1)
    }

    fn readout(&self, frames: &Self::Value, x: f64) -> String {
        frames
            .get(x as usize)
            .and_then(Option::as_ref)
            .map_or("\u{2013}".into(), |summary| {
                format_value(self.statistic.of(summary))
            })
    }

    fn decorate(
        &self,
        plot: Stateful<Div>,
        axes: &Axes,
        _: &[Plotted<Self>],
        snapshot: &Snapshot<SeriesKey>,
        cx: &mut Context<PlotWindow<Self>>,
    ) -> Stateful<Div> {
        let info = cx.theme().info;
        let x = axes.x;
        let frame_at = move |this: &PlotWindow<Self>, position: Pixels| {
            Some(this.x_at(position, x)?.round() as usize)
        };
        plot.cursor_pointer()
            // the viewer's current frame
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(axes.x(snapshot.frame as f64)))
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
}

impl TimeSeriesPlot {
    /// The chosen statistic of each frame
    fn values(&self, frames: &[Option<Summary>]) -> Vec<Option<f64>> {
        frames
            .iter()
            .map(|frame| frame.as_ref().map(|summary| self.statistic.of(summary)))
            .collect()
    }
}

impl Viewer {
    pub(super) fn show_time_series(
        &mut self,
        _: &ShowTimeSeries,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let plot = TimeSeriesPlot {
            radius: 0,
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

/// The most frames of any layer (min. 2)
fn frames(snapshot: &Snapshot<SeriesKey>) -> usize {
    snapshot
        .lines
        .iter()
        .map(|line| line.volume.meta().timepoints())
        .fold(2, usize::max)
}

fn with_values(frames: Vec<Option<Summary>>) -> Option<Vec<Option<Summary>>> {
    frames.iter().any(Option::is_some).then_some(frames)
}

fn value_range<'a>(values: impl Iterator<Item = &'a Option<f64>>) -> Option<(f64, f64)> {
    let (low, high) = values
        .flatten()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), &value| {
            (low.min(value), high.max(value))
        });
    if low > high {
        return None;
    }
    let padding = if high > low { (high - low) * 0.05 } else { 0.5 };
    Some((low - padding, high + padding))
}

fn segments(axes: &Axes, values: &[Option<f64>]) -> Vec<Polyline> {
    let mut segments = vec![Vec::new()];
    for (frame, value) in values.iter().enumerate() {
        match value {
            Some(value) => segments
                .last_mut()
                .unwrap()
                .push((axes.x(frame as f64), axes.y(*value))),
            None if segments.last().is_some_and(|last| !last.is_empty()) => {
                segments.push(Vec::new());
            }
            None => {}
        }
    }
    segments.retain(|segment| !segment.is_empty());
    segments
}

#[cfg(test)]
mod tests {
    use super::{Axes, segments, value_range};

    #[test]
    fn series_break_at_frames_without_values() {
        let axes = Axes {
            x: (0.0, 4.0),
            y: (0.0, 10.0),
            log: false,
        };
        let values = [Some(0.0), Some(5.0), None, None, Some(10.0)];
        assert_eq!(
            segments(&axes, &values),
            vec![vec![(0.0, 0.0), (0.25, 0.5)], vec![(1.0, 1.0)]]
        );
        assert!(segments(&axes, &[None, None]).is_empty());
    }

    #[test]
    fn value_range_has_room_around_the_values() {
        assert_eq!(
            value_range([Some(0.0), None, Some(10.0)].iter()),
            Some((-0.5, 10.5))
        );
        assert_eq!(value_range([Some(3.0)].iter()), Some((2.5, 3.5)));
        assert_eq!(value_range([None].iter()), None);
    }
}
