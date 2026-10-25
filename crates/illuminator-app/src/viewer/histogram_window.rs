//! Histogram window.
//! The values of the visible layers counted into bins

mod plot;

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

use self::plot::{height_at, steps};
use super::plot_window::{
    Axes, Chart, Job, LabelMap, Line, Plot, PlotWindow, Plotted, RegionJob, RegionKey, Snapshot,
    ticks, value_at,
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

    const TITLE: &'static str = "Histogram";
    const CSV_NAME: &'static str = "histogram";
    const COPY_TOOLTIP: &'static str = "Copy shown histograms as CSV";
    const MEASURING: &'static str = "Counting values\u{2026}";
    const MEASURING_NOTE: &'static str = " (counting\u{2026})";
    const NO_VALUES: &'static str = "No values to count";
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
        let region = match &key.region {
            Some(region) => Some(region.job(label_maps)?),
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
                region.as_ref().map(RegionJob::mask),
                progress,
            )
        }))
    }

    fn shows_stale(old: &CurveKey, new: &CurveKey) -> bool {
        old.other_frame_of(new)
    }

    fn csv(shown: &[(&Line<CurveKey>, &Histogram)]) -> String {
        to_csv(
            shown
                .iter()
                .map(|&(line, histogram)| (line.number, line.name.as_ref(), histogram)),
        )
    }

    fn controls(
        window: &PlotWindow<Self>,
        controls: Div,
        snapshot: Option<&Snapshot<CurveKey>>,
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

    fn x_name(x: f64) -> String {
        format_value(x)
    }

    fn readout(&self, histogram: &Histogram, x: f64) -> String {
        let heights = self.y_axis.heights(histogram);
        let cumulative = self.y_axis == AxisY::Cumulative;
        self.y_axis
            .format(height_at(histogram, &heights, x, cumulative))
    }

    fn decorate(
        &self,
        mut plot: Stateful<Div>,
        axes: &Axes,
        plotted: &[Plotted<Self>],
        _: &Snapshot<CurveKey>,
        cx: &mut Context<PlotWindow<Self>>,
    ) -> Stateful<Div> {
        // the selected layers window
        if let Some(line) = plotted
            .iter()
            .map(|plotted| plotted.line)
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
    use super::super::plot_window::{Line, Measured, RegionKey, current, export_ready};
    use super::CurveKey;
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

    fn line_fixture() -> (TestDirectory, Line<CurveKey>) {
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
            key: Some(key),
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
        let mut key = line.key.clone().unwrap();
        key.region = Some(RegionKey {
            volume: 42,
            frame: 0,
            geometry: key.geometry.clone(),
            label: Some(1),
        });
        line.key = Some(key.clone());
        let mut results = HashMap::from([(
            line.load_id,
            Measured {
                key: key.clone(),
                value: Some(histogram()),
            },
        )]);
        assert!(current(&results, &line).is_some());
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
            line.key = Some(changed);
            assert!(current(&results, &line).is_none(), "{name}");
        }
        results.get_mut(&line.load_id).unwrap().value = None;
        assert!(current(&results, &line).is_none());
        line.key = None;
        assert!(current(&results, &line).is_none());
        line.key = Some(key);
        assert!(current(&results, &line).unwrap().value.is_none());
        line.load_id += 1;
        assert!(current(&results, &line).is_none());
    }

    #[test]
    fn export_waits_for_every_nonhidden_desired_curve() {
        let (_directory, first) = line_fixture();
        let key = first.key.clone().unwrap();
        let second = Line {
            load_id: first.load_id + 1,
            number: 2,
            name: "second".into(),
            color: first.color,
            volume: Arc::clone(&first.volume),
            key: Some(key.clone()),
            limits: first.limits,
            selected: true,
        };
        let lines = [first, second];
        let mut results = HashMap::from([(
            lines[0].load_id,
            Measured {
                key: key.clone(),
                value: Some(histogram()),
            },
        )]);
        let mut hidden = HashSet::new();
        assert!(!export_ready(&lines, &results, &hidden));
        hidden.insert(lines[1].load_id);
        assert!(export_ready(&lines, &results, &hidden));
        hidden.clear();
        let mut stale_key = key.clone();
        stale_key.frame += 1;
        results.insert(
            lines[1].load_id,
            Measured {
                key: stale_key,
                value: None,
            },
        );
        assert!(!export_ready(&lines, &results, &hidden));
        results.get_mut(&lines[1].load_id).unwrap().value = Some(histogram());
        assert!(!export_ready(&lines, &results, &hidden));
        let second = results.get_mut(&lines[1].load_id).unwrap();
        second.key = key;
        second.value = None;
        assert!(export_ready(&lines, &results, &hidden));
        results.get_mut(&lines[0].load_id).unwrap().value = None;
        assert!(!export_ready(&lines, &results, &hidden));
        hidden.extend(lines.iter().map(|line| line.load_id));
        assert!(!export_ready(&lines, &results, &hidden));
        assert!(!export_ready::<CurveKey, Histogram>(
            &[],
            &results,
            &HashSet::new()
        ));
    }
}
