//! Histogram window.
//! The values of the visible layers counted into bins

mod bars;

use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::{
    AppContext, Context, Div, DragMoveEvent, InteractiveElement, ParentElement, Stateful,
    StatefulInteractiveElement, Styled, Window, div, px, relative,
};
use illuminator_core::display::VolumeDisplay;
use illuminator_core::geometry::VolumeGeometry;
use illuminator_core::histogram::{Counting, Histogram, to_csv};
use illuminator_core::progress::Progress;
use illuminator_core::stats::Source;
use illuminator_core::volume::Volume;
use std::sync::Arc;

use crate::shortcuts::ShowHistogram;
use crate::ui::{choice_items, format_value};

use self::bars::{height_at, steps};
use super::plot_window::{
    Axes, Chart, Job, LabelArea, LabelMap, Plot, PlotLayer, PlotWindow, Plotted, RegionKey,
    Snapshot, Texts, ticks, value_at,
};
use super::{DragEdge, Viewer, WindowEdge};

const HISTOGRAM_X_TICKS: usize = 8;
const HISTOGRAM_Y_TICKS: usize = 5;
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

/// What a curve is counted from.
#[derive(Clone, PartialEq)]
pub(super) struct CurveKey {
    volume: u64,
    frame: usize,
    geometry: VolumeGeometry,
    counting: Counting,
    region: Option<RegionKey>,
}

impl CurveKey {
    /// Whether this key counts the same values as `key` in another frame.
    fn other_frame_of(&self, key: &CurveKey) -> bool {
        self.frame != key.frame
            && self.volume == key.volume
            && self.geometry == key.geometry
            && self.counting == key.counting
            && self.region == key.region
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct HistogramPlot {
    counting: Counting,
    y_axis: AxisY,
    log: bool,
}

impl Plot for HistogramPlot {
    type Key = CurveKey;
    type Value = Histogram;

    const TEXTS: Texts = Texts {
        title: "Histogram",
        csv_name: "histogram",
        copy_tooltip: "Copy shown histograms as CSV",
        no_layers: "No visible intensity layers",
        measuring: "Counting values\u{2026}",
        measuring_note: " (counting\u{2026})",
        waiting: "Pause playback to count values",
        waiting_note: " (pause to count)",
        nothing_to_measure: "Nothing to count",
        nothing_to_measure_note: " (nothing to count)",
        no_values: "No values to count",
    };
    const WAITS_FOR_PAUSE: bool = true;

    fn key(&self, display: &VolumeDisplay, region: Option<&RegionKey>) -> Option<CurveKey> {
        Some(CurveKey {
            volume: display.volume().id(),
            frame: display.frame(),
            geometry: display.geometry().clone(),
            counting: self.counting,
            region: region.cloned(),
        })
    }

    fn job(key: &CurveKey, volume: Arc<Volume>, label_maps: &[LabelMap]) -> Option<Job<Histogram>> {
        let labels = match &key.region {
            Some(region) => Some(region.resolve(label_maps)?),
            None => None,
        };
        let (geometry, frame, counting) = (key.geometry.clone(), key.frame, key.counting);
        Some(Box::new(move |progress: &Progress| {
            let values = Source {
                volume: &volume,
                geometry: &geometry,
                frame,
            };
            Histogram::counted(
                values,
                counting,
                labels.as_ref().map(LabelArea::mask),
                progress,
            )
        }))
    }

    fn shows_stale(old: &CurveKey, new: &CurveKey) -> bool {
        old.other_frame_of(new)
    }

    fn csv(exported: &[(&PlotLayer<CurveKey>, &Histogram)]) -> String {
        to_csv(
            exported
                .iter()
                .map(|&(layer, histogram)| (layer.number, layer.name.as_ref(), histogram)),
        )
    }

    fn controls(
        window: &PlotWindow<Self>,
        controls: Div,
        snapshot: &Snapshot<CurveKey>,
        cx: &mut Context<PlotWindow<Self>>,
    ) -> Div {
        let this = cx.entity().downgrade();
        let plot = window.plot;
        let bins = match plot.counting.bins {
            None => "Auto".to_string(),
            Some(bins) => bins.to_string(),
        };
        controls
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
                                this.plot.counting.bins = bins;
                                cx.notify();
                            })
                        }
                    }),
            )
            .child(
                Button::new("y-axis")
                    .xsmall()
                    .outline()
                    .label(plot.y_axis.name())
                    .dropdown_caret(true)
                    .dropdown_menu(move |menu, _, _| {
                        let choices = AxisY::ALL.map(|y_axis| (y_axis.name(), y_axis));
                        choice_items(menu, &this, choices, |this, y_axis, _, cx| {
                            this.plot.y_axis = y_axis;
                            cx.notify();
                        })
                    }),
            )
            .child(
                Checkbox::new("log")
                    .label("Log scale")
                    .checked(plot.log)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.plot.log = *checked;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("ignore-zeros")
                    .label("Ignore zeros")
                    .checked(plot.counting.ignore_zeros)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.plot.counting.ignore_zeros = *checked;
                        cx.notify();
                    })),
            )
            .child(window.region_button(
                "Region",
                "Whole volume".into(),
                vec![("Whole volume".into(), ())],
                |_, ()| {},
                snapshot,
                cx,
            ))
    }

    fn chart(&self, plotted: &[Plotted<Self>], _: &Snapshot<CurveKey>) -> Chart {
        let heights: Vec<Vec<f64>> = plotted
            .iter()
            .map(|plotted| self.y_axis.heights(plotted.value))
            .collect();
        let axes = self.axes(plotted, &heights);
        let (values, decimals) = ticks(axes.x.0, axes.x.1, HISTOGRAM_X_TICKS);
        let x_ticks = values
            .into_iter()
            .map(|value| (axes.x(value), format!("{value:.decimals$}")))
            .collect();
        let y_ticks = if self.log {
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
        let cumulative = self.y_axis == AxisY::Cumulative;
        let curves = plotted
            .iter()
            .zip(&heights)
            .map(|(plotted, heights)| vec![steps(&axes, plotted.value, heights, cumulative)])
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
        mut plot: Stateful<Div>,
        axes: &Axes,
        plotted: &[Plotted<Self>],
        _: &Snapshot<CurveKey>,
        cx: &mut Context<PlotWindow<Self>>,
    ) -> Stateful<Div> {
        // the selected layers window
        if let Some(layer) = plotted
            .iter()
            .map(|plotted| plotted.layer)
            .find(|layer| layer.selected)
        {
            let (low, high) = layer.limits;
            let (left, right) = (axes.x(low).clamp(0.0, 1.0), axes.x(high).clamp(0.0, 1.0));
            let load_id = layer.load_id;
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
                    .child(div().w(px(2.0)).h_full().bg(layer.color))
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
                        .bg(layer.color.opacity(0.1)),
                )
                .child(edge(WindowEdge::Low, low))
                .child(edge(WindowEdge::High, high));
        }
        let x = axes.x;
        plot.on_drag_move(
            cx.listener(move |this, event: &DragMoveEvent<DragEdge>, _, cx| {
                let drag = *event.drag(cx);
                let width = f32::from(event.bounds.size.width);
                let fraction = f32::from(event.event.position.x - event.bounds.left()) / width;
                let value = value_at(x, fraction);
                this.update_viewer(cx, |viewer, _, cx| viewer.move_window_edge(drag, value, cx));
            }),
        )
    }

    fn readout_title(x: f64) -> String {
        format_value(x)
    }

    fn readout(&self, histogram: &Histogram, x: f64) -> String {
        let heights = self.y_axis.heights(histogram);
        let cumulative = self.y_axis == AxisY::Cumulative;
        self.y_axis
            .format(height_at(histogram, &heights, x, cumulative))
    }
}

impl HistogramPlot {
    /// Ranges of the x and y axes that fit every plotted curve
    fn axes(&self, plotted: &[Plotted<Self>], heights: &[Vec<f64>]) -> Axes {
        let low = plotted.iter().map(|plotted| plotted.value.min);
        let high = plotted.iter().map(|plotted| plotted.value.max);
        let (low, high) = (
            low.fold(f64::INFINITY, f64::min),
            high.fold(f64::NEG_INFINITY, f64::max),
        );
        let heights = || heights.iter().flatten().copied();
        let top = heights().fold(0.0, f64::max);
        let y = if self.log {
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
            log: self.log,
        }
    }
}

impl Viewer {
    /// Opens histogram window or brings it to the front
    pub(super) fn show_histogram(
        &mut self,
        _: &ShowHistogram,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let plot = HistogramPlot {
            counting: Counting {
                bins: None,
                ignore_zeros: true,
            },
            y_axis: AxisY::Count,
            log: false,
        };
        self.show_plot_window(plot, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::super::plot_window::RegionKey;
    use super::super::plot_window::test_util::loaded_display;
    use super::CurveKey;
    use illuminator_core::histogram::Counting;

    #[test]
    fn curve_keys_change_with_their_sources_and_settings() {
        let (_directory, display) = loaded_display();
        let key = CurveKey {
            volume: display.volume().id(),
            frame: display.frame(),
            geometry: display.geometry().clone(),
            counting: Counting {
                bins: Some(32),
                ignore_zeros: true,
            },
            region: Some(RegionKey {
                volume: 42,
                frame: 0,
                geometry: display.geometry().clone(),
                label: Some(1),
            }),
        };
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
            let mut changed = key.clone();
            change(&mut changed);
            assert!(changed != key, "{name}");
            assert_eq!(changed.other_frame_of(&key), name == "frame", "{name}");
        }
    }
}
