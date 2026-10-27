//! Time series lines.

use super::super::plot_window::{Axes, Curve};

/// The y axis range that fits `values` with some room. None if there are none
pub(super) fn padded_value_range<'a>(
    values: impl Iterator<Item = &'a Option<f64>>,
) -> Option<(f64, f64)> {
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

/// A value per frame as a line
pub(super) fn curve_segments(axes: &Axes, values: &[Option<f64>]) -> Curve {
    let mut segments = vec![Vec::new()];
    for (frame, value) in values.iter().enumerate() {
        match value {
            Some(value) => segments
                .last_mut()
                .unwrap()
                .push((axes.x_fraction(frame as f64), axes.y_fraction(*value))),
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
    use super::{Axes, curve_segments, padded_value_range};

    #[test]
    fn series_break_at_frames_without_values() {
        let axes = Axes {
            x: (0.0, 4.0),
            y: (0.0, 10.0),
            logarithmic: false,
        };
        let values = [Some(0.0), Some(5.0), None, None, Some(10.0)];
        assert_eq!(
            curve_segments(&axes, &values),
            vec![vec![(0.0, 0.0), (0.25, 0.5)], vec![(1.0, 1.0)]]
        );
        assert!(curve_segments(&axes, &[None, None]).is_empty());
    }

    #[test]
    fn value_range_has_room_around_the_values() {
        assert_eq!(
            padded_value_range([Some(0.0), None, Some(10.0)].iter()),
            Some((-0.5, 10.5))
        );
        assert_eq!(padded_value_range([Some(3.0)].iter()), Some((2.5, 3.5)));
        assert_eq!(padded_value_range([None].iter()), None);
    }
}
