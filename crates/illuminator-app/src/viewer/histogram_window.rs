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
use illuminator_core::histogram::{Histogram, HistogramOptions, to_csv};
use illuminator_core::progress::Progress;
use illuminator_core::statistics::VolumeFrame;
use illuminator_core::volume::Volume;
use std::sync::Arc;

use crate::shortcuts::ShowHistogramWindow;
use crate::ui::{add_choice_items, format_value};

use self::bars::{bin_height_at, step_outline};
use super::plot_window::{
    Axes, Chart, LabelMap, MeasurementJob, Plot, PlotLayer, PlotStrings, PlotWindow, PlottedValue,
    RegionKey, ResolvedLabelRegion, ViewerSnapshot, tick_values, value_at,
};
use super::{Viewer, WindowEdge, WindowEdgeDrag};

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
enum YAxisQuantity {
    Count,
    Probability,
    Cumulative,
}

impl YAxisQuantity {
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
    fn bin_heights(self, histogram: &Histogram) -> Vec<f64> {
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

    fn format_height(self, height: f64) -> String {
        match self {
            Self::Count => format!("{height:.0}"),
            Self::Probability | Self::Cumulative => format!("{height:.2} %"),
        }
    }
}

/// What a curve is counted from.
#[derive(Clone, PartialEq)]
pub(super) struct HistogramCurveKey {
    volume: u64,
    frame: usize,
    geometry: VolumeGeometry,
    options: HistogramOptions,
    region: Option<RegionKey>,
}

impl HistogramCurveKey {
    /// Whether this key counts the same values as `key` in another frame.
    fn is_other_frame_of(&self, key: &HistogramCurveKey) -> bool {
        self.frame != key.frame
            && self.volume == key.volume
            && self.geometry == key.geometry
            && self.options == key.options
            && self.region == key.region
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct HistogramPlot {
    options: HistogramOptions,
    y_axis: YAxisQuantity,
    logarithmic: bool,
}

impl Plot for HistogramPlot {
    type Key = HistogramCurveKey;
    type Value = Histogram;

    const STRINGS: PlotStrings = PlotStrings {
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

    fn key(
        &self,
        display: &VolumeDisplay,
        region: Option<&RegionKey>,
    ) -> Option<HistogramCurveKey> {
        Some(HistogramCurveKey {
            volume: display.volume().id(),
            frame: display.frame(),
            geometry: display.geometry().clone(),
            options: self.options,
            region: region.cloned(),
        })
    }

    fn measurement_job(
        key: &HistogramCurveKey,
        volume: Arc<Volume>,
        label_maps: &[LabelMap],
    ) -> Option<MeasurementJob<Histogram>> {
        let labels = match &key.region {
            Some(region) => Some(region.resolve(label_maps)?),
            None => None,
        };
        let (geometry, frame, counting) = (key.geometry.clone(), key.frame, key.options);
        Some(Box::new(move |progress: &Progress| {
            let values = VolumeFrame {
                volume: &volume,
                geometry: &geometry,
                frame,
            };
            Histogram::of_selection(
                values,
                counting,
                labels.as_ref().map(ResolvedLabelRegion::mask),
                progress,
            )
        }))
    }

    fn shows_stale(old: &HistogramCurveKey, new: &HistogramCurveKey) -> bool {
        old.is_other_frame_of(new)
    }

    fn csv(exported: &[(&PlotLayer<HistogramCurveKey>, &Histogram)]) -> String {
        to_csv(
            exported
                .iter()
                .map(|&(layer, histogram)| (layer.number, layer.name.as_ref(), histogram)),
        )
    }

    fn controls(
        window: &PlotWindow<Self>,
        controls: Div,
        snapshot: &ViewerSnapshot<HistogramCurveKey>,
        cx: &mut Context<PlotWindow<Self>>,
    ) -> Div {
        let this = cx.entity().downgrade();
        let plot = window.plot;
        let bins = match plot.options.bins {
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
                            add_choice_items(menu, &this, choices, |this, bins, _, cx| {
                                this.plot.options.bins = bins;
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
                        let choices = YAxisQuantity::ALL.map(|y_axis| (y_axis.name(), y_axis));
                        add_choice_items(menu, &this, choices, |this, y_axis, _, cx| {
                            this.plot.y_axis = y_axis;
                            cx.notify();
                        })
                    }),
            )
            .child(
                Checkbox::new("log")
                    .label("Log scale")
                    .checked(plot.logarithmic)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.plot.logarithmic = *checked;
                        cx.notify();
                    })),
            )
            .child(
                Checkbox::new("ignore-zeros")
                    .label("Ignore zeros")
                    .checked(plot.options.ignore_zeros)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.plot.options.ignore_zeros = *checked;
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

    fn chart(
        &self,
        plotted: &[PlottedValue<Self>],
        _: &ViewerSnapshot<HistogramCurveKey>,
    ) -> Chart {
        let heights: Vec<Vec<f64>> = plotted
            .iter()
            .map(|plotted| self.y_axis.bin_heights(plotted.value))
            .collect();
        let axes = self.fit_axes(plotted, &heights);
        let (values, decimals) = tick_values(axes.x.0, axes.x.1, HISTOGRAM_X_TICKS);
        let x_ticks = values
            .into_iter()
            .map(|value| (axes.x_fraction(value), format!("{value:.decimals$}")))
            .collect();
        let y_ticks = if self.logarithmic {
            let decades = axes.y.0.ceil() as i32..=axes.y.1.floor() as i32;
            decades
                .map(|decade| {
                    let decimals = (-decade).max(0) as usize;
                    let label = format!("{:.*}", decimals, 10f64.powi(decade));
                    (axes.y_fraction(10f64.powi(decade)), label)
                })
                .collect()
        } else {
            let (heights, decimals) = tick_values(axes.y.0, axes.y.1, HISTOGRAM_Y_TICKS);
            heights
                .into_iter()
                .map(|height| (axes.y_fraction(height), format!("{height:.decimals$}")))
                .collect()
        };
        let cumulative = self.y_axis == YAxisQuantity::Cumulative;
        let curves = plotted
            .iter()
            .zip(&heights)
            .map(|(plotted, heights)| vec![step_outline(&axes, plotted.value, heights, cumulative)])
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
        plotted: &[PlottedValue<Self>],
        _: &ViewerSnapshot<HistogramCurveKey>,
        cx: &mut Context<PlotWindow<Self>>,
    ) -> Stateful<Div> {
        // the selected layers window
        if let Some(layer) = plotted
            .iter()
            .map(|plotted| plotted.layer)
            .find(|layer| layer.selected)
        {
            let (low, high) = layer.limits;
            let (left, right) = (
                axes.x_fraction(low).clamp(0.0, 1.0),
                axes.x_fraction(high).clamp(0.0, 1.0),
            );
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
                    .left(relative(axes.x_fraction(value).clamp(0.0, 1.0)))
                    .ml(px(-4.0))
                    .w(px(8.0))
                    .flex()
                    .justify_center()
                    .cursor_ew_resize()
                    .child(div().w(px(2.0)).h_full().bg(layer.color))
                    .on_drag(WindowEdgeDrag { load_id, edge }, |drag, _, _, cx| {
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
            cx.listener(move |this, event: &DragMoveEvent<WindowEdgeDrag>, _, cx| {
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
        let heights = self.y_axis.bin_heights(histogram);
        let cumulative = self.y_axis == YAxisQuantity::Cumulative;
        self.y_axis
            .format_height(bin_height_at(histogram, &heights, x, cumulative))
    }
}

impl HistogramPlot {
    /// Ranges of the x and y axes that fit every plotted curve
    fn fit_axes(&self, plotted: &[PlottedValue<Self>], heights: &[Vec<f64>]) -> Axes {
        let low = plotted.iter().map(|plotted| plotted.value.minimum);
        let high = plotted.iter().map(|plotted| plotted.value.maximum);
        let (low, high) = (
            low.fold(f64::INFINITY, f64::min),
            high.fold(f64::NEG_INFINITY, f64::max),
        );
        let heights = || heights.iter().flatten().copied();
        let maximum_height = heights().fold(0.0, f64::max);
        let y = if self.logarithmic {
            let lowest = heights()
                .filter(|&height| height > 0.0)
                .fold(maximum_height, f64::min);
            if maximum_height > 0.0 {
                // Make sure single voxels still show
                (lowest.log10().floor() - 0.3, maximum_height.log10() + 0.1)
            } else {
                (0.0, 1.0)
            }
        } else {
            (
                0.0,
                if maximum_height > 0.0 {
                    maximum_height * 1.05
                } else {
                    1.0
                },
            )
        };
        Axes {
            x: if high > low {
                (low, high)
            } else {
                (low - 0.5, low + 0.5)
            },
            y,
            logarithmic: self.logarithmic,
        }
    }
}

impl Viewer {
    /// Opens histogram window or brings it to the front
    pub(super) fn show_histogram(
        &mut self,
        _: &ShowHistogramWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let plot = HistogramPlot {
            options: HistogramOptions {
                bins: None,
                ignore_zeros: true,
            },
            y_axis: YAxisQuantity::Count,
            logarithmic: false,
        };
        self.show_plot_window(plot, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::super::plot_window::RegionKey;
    use super::super::plot_window::test_fixtures::loaded_display;
    use super::HistogramCurveKey;
    use illuminator_core::histogram::HistogramOptions;

    #[test]
    fn curve_keys_change_with_their_sources_and_settings() {
        let (_directory, display) = loaded_display();
        let key = HistogramCurveKey {
            volume: display.volume().id(),
            frame: display.frame(),
            geometry: display.geometry().clone(),
            options: HistogramOptions {
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
        type KeyChange = (&'static str, fn(&mut HistogramCurveKey));
        let changes: [KeyChange; 10] = [
            ("volume", |key| key.volume += 1),
            ("frame", |key| key.frame += 1),
            ("geometry", |key| key.geometry.anatomical ^= true),
            ("bins", |key| key.options.bins = Some(64)),
            ("zeros", |key| key.options.ignore_zeros = false),
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
            assert_eq!(changed.is_other_frame_of(&key), name == "frame", "{name}");
        }
    }
}
