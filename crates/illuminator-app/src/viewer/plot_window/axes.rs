//! Where values land in a plot.

const MAX_TICKS: usize = 64;
const MAX_TICK_DECIMALS: usize = 324;

/// Where values and heights land in the plot as fractions from its left and bottom.
pub(in crate::viewer) struct Axes {
    pub(in crate::viewer) x: (f64, f64),
    /// In log10 of the height
    pub(in crate::viewer) y: (f64, f64),
    pub(in crate::viewer) log: bool,
}

impl Axes {
    pub(in crate::viewer) fn x(&self, value: f64) -> f32 {
        let (low, high) = self.x;
        let span = high - low;
        if span.is_finite() {
            ((value - low) / span) as f32
        } else {
            ((value * 0.5 - low * 0.5) / (high * 0.5 - low * 0.5)) as f32
        }
    }

    pub(in crate::viewer) fn y(&self, height: f64) -> f32 {
        let height = match self.log {
            true if height > 0.0 => height.log10(),
            true => self.y.0,
            false => height,
        };
        (((height - self.y.0) / (self.y.1 - self.y.0)) as f32).clamp(0.0, 1.0)
    }
}

/// Value `fraction` of the way along the x axis from `low` to `high`
pub(in crate::viewer) fn value_at((low, high): (f64, f64), fraction: f32) -> f64 {
    let fraction = f64::from(fraction);
    let span = high - low;
    if span.is_finite() {
        low + fraction * span
    } else {
        (1.0 - fraction) * low + fraction * high
    }
}

/// Round values from `low` to `high` about `count` and needed decimals
pub(in crate::viewer) fn ticks(low: f64, high: f64, count: usize) -> (Vec<f64>, usize) {
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
            (-span.log10().floor() + 1.0).clamp(0.0, MAX_TICK_DECIMALS as f64) as usize
        } else {
            0
        };
        (values, decimals)
    };
    if !span.is_finite() {
        return fallback();
    }
    let rough = span / count.clamp(1, MAX_TICKS - 1) as f64;
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
    let decimals = (-step.log10().floor()).clamp(0.0, MAX_TICK_DECIMALS as f64) as usize;
    let (first, last) = ((low / step).ceil(), (high / step).floor());
    let intervals = last - first;
    if !(0.0..MAX_TICKS as f64).contains(&intervals) {
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
    use super::{Axes, MAX_TICK_DECIMALS, MAX_TICKS, ticks, value_at};

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
            assert!(values.len() <= MAX_TICKS);
            assert!(decimals <= MAX_TICK_DECIMALS);
            assert!(
                values
                    .iter()
                    .all(|value| value.is_finite() && (low..=high).contains(value))
            );
            assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
            for value in values {
                let label = format!("{value:.decimals$}");
                assert!(label.len() <= MAX_TICK_DECIMALS + 3);
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
            assert!(values.len() <= MAX_TICKS);
            assert!(decimals <= MAX_TICK_DECIMALS);
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
