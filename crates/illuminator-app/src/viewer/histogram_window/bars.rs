//! Histogram bars.

use illuminator_core::histogram::Histogram;

use super::super::plot_window::Axes;

/// Bin readout including the cumulative distributions tails outside its range.
pub(super) fn height_at(
    histogram: &Histogram,
    heights: &[f64],
    value: f64,
    cumulative: bool,
) -> f64 {
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
pub(super) fn steps(
    axes: &Axes,
    histogram: &Histogram,
    heights: &[f64],
    cumulative: bool,
) -> Vec<(f32, f32)> {
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

#[cfg(test)]
mod tests {
    use super::super::AxisY;
    use super::{Axes, height_at, steps};
    use illuminator_core::histogram::Histogram;

    fn histogram() -> Histogram {
        Histogram {
            min: 2.0,
            max: 6.0,
            counts: vec![1, 3],
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
    fn wide_histograms_remain_finite() {
        let axes = Axes {
            x: (-f64::MAX, f64::MAX),
            y: (0.0, 1.0),
            log: false,
        };
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
}
