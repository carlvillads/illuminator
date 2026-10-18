//! Histograms of one frames values, for the sidebar, histogram window, and the percentile presets.

use rayon::prelude::*;
use std::borrow::Cow;
use std::fmt::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::{Error, Result};
use crate::geometry::{Point3D, VolumeGeometry};
use crate::meta::VolumeMeta;
use crate::progress::Progress;
use crate::stats::Source;
use crate::volume::Volume;
use crate::voxel;

const HISTOGRAM_MAX_BINS: usize = 10000;

#[derive(Debug, Clone, PartialEq)]
pub struct Histogram {
    pub min: f64,
    pub max: f64,
    pub counts: Vec<u64>,
}

impl Histogram {
    /// Histogram of the values in frame t. None if there are none
    pub fn of(volume: &Volume, t: usize, bins: usize, progress: &Progress) -> Result<Option<Self>> {
        let Some(range) = volume.frame_range(t, &progress.part(0.0, 0.5))? else {
            return Ok(None);
        };
        let meta = volume.meta();
        let mut bins = bins.clamp(1, HISTOGRAM_MAX_BINS);
        if range.min != range.max {
            while let Err(error) = validate_bin_edges(range.min, range.max, bins) {
                if bins == 1 {
                    return Err(error);
                }
                bins /= 2;
            }
        }
        let last = bins - 1;
        let span = range.max - range.min;
        let scaling = meta.scaling;
        let counts = voxel::Values::new(volume.frame(t), meta.datatype, meta.endian).fold(
            &progress.part(0.5, 0.5),
            "Counting values",
            || vec![0u64; bins],
            |mut counts, raw| {
                let value = scaling.apply(raw);
                if value.is_finite() {
                    let position = if span > 0.0 {
                        (value - range.min) / span * bins as f64
                    } else {
                        0.0
                    };
                    let bin = position.clamp(0.0, last as f64) as usize;
                    counts[bin] += 1;
                }
                counts
            },
            add_counts,
        )?;
        progress.report(1.0, "Counted values")?;
        Ok(Some(Self {
            min: range.min,
            max: range.max,
            counts,
        }))
    }

    /// Histogram of the values in `values` that `counting` keeps and that lie inside `mask`. None if no values inside mask.
    pub fn counted(
        values: Source,
        counting: Counting,
        mask: Option<LabelMask>,
        progress: &Progress,
    ) -> Result<Option<Self>> {
        let inside = mask
            .map(|mask| mask_bits(values, mask, &progress.part(0.0, 0.4)))
            .transpose()?;
        let passes = match inside {
            Some(_) => progress.part(0.4, 0.6),
            None => progress.clone(),
        };
        let spread = fold_kept(
            values,
            counting,
            inside.as_deref(),
            &passes.part(0.0, 0.5),
            "Measuring values",
            Spread::default,
            Spread::add,
            Spread::merge,
        )?;
        if spread.count == 0 {
            progress.report(1.0, "No values to count")?;
            return Ok(None);
        }
        let (min, max, bins) = binning(spread, counting.bins)?;
        let width = (max - min) / bins as f64;
        let last = bins - 1;
        let counts = fold_kept(
            values,
            counting,
            inside.as_deref(),
            &passes.part(0.5, 0.5),
            "Counting values",
            || vec![0u64; bins],
            |mut counts, value| {
                let bin = ((value - min) / width).clamp(0.0, last as f64) as usize;
                counts[bin] += 1;
                counts
            },
            add_counts,
        )?;
        progress.report(1.0, "Counted values")?;
        Ok(Some(Self { min, max, counts }))
    }

    pub fn total(&self) -> u64 {
        self.counts.iter().sum()
    }

    pub fn bin_width(&self) -> f64 {
        (self.max - self.min) / self.counts.len() as f64
    }

    pub fn percentile(&self, fraction: f64) -> f64 {
        let target = fraction.clamp(0.0, 1.0) * self.total() as f64;
        let mut before = 0.0;
        for (bin, &count) in self.counts.iter().enumerate() {
            let after = before + count as f64;
            if count > 0 && after >= target {
                let inside = (target - before) / count as f64;
                return self.min + (bin as f64 + inside) * self.bin_width();
            }
            before = after;
        }
        self.max
    }
}

pub fn to_csv<'a>(histograms: impl IntoIterator<Item = (usize, &'a str, &'a Histogram)>) -> String {
    let mut csv = String::from("layer,name,bin_start,bin_end,count,probability,cumulative\n");
    for (layer, name, histogram) in histograms {
        let (width, total) = (histogram.bin_width(), histogram.total().max(1) as f64);
        let name = quoted(name);
        let mut below = 0;
        for (bin, &count) in histogram.counts.iter().enumerate() {
            below += count;
            let start = histogram.min + bin as f64 * width;
            let end = if bin + 1 == histogram.counts.len() {
                histogram.max
            } else {
                histogram.min + (bin + 1) as f64 * width
            };
            let _ = writeln!(
                csv,
                "{layer},{name},{start},{end},{count},{},{}",
                count as f64 / total,
                below as f64 / total,
            );
        }
    }
    csv
}

/// `text` as one CSV field.
fn quoted(text: &str) -> Cow<'_, str> {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\"")).into()
    } else {
        text.into()
    }
}

/// How the histogram window should count values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counting {
    pub bins: Option<usize>,
    pub ignore_zeros: bool,
}

/// Values that lie on `label` of a labelmap or on any label when `label` is None.
#[derive(Clone, Copy)]
pub struct LabelMask<'a> {
    pub labels: Source<'a>,
    pub label: Option<i64>,
}

impl LabelMask<'_> {
    /// Whether the voxel xyz of a volume lies inside the mask.
    fn contains(&self, geometry: &VolumeGeometry, [x, y, z]: [usize; 3]) -> bool {
        let voxel = Point3D::new(x as f64, y as f64, z as f64);
        let world = geometry.voxel_to_world.point(voxel);
        let value = self
            .labels
            .geometry
            .nearest_voxel(world)
            .and_then(|voxel| self.labels.volume.sample_value(voxel, self.labels.frame));
        match (value, self.label) {
            (Some(value), Some(label)) => value == label as f64,
            (Some(value), None) => value != 0.0,
            (None, _) => false,
        }
    }
}

#[derive(Clone, Copy)]
struct Spread {
    min: f64,
    max: f64,
    count: u64,
    whole: bool,
}

impl Default for Spread {
    fn default() -> Self {
        Self {
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            count: 0,
            whole: true,
        }
    }
}

impl Spread {
    fn add(self, value: f64) -> Self {
        Self {
            min: self.min.min(value),
            max: self.max.max(value),
            count: self.count + 1,
            whole: self.whole && value == value.trunc(),
        }
    }

    fn merge(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
            count: self.count + other.count,
            whole: self.whole && other.whole,
        }
    }
}

fn unrepresentable_range(min: f64, max: f64) -> Error {
    Error::invalid(format!(
        "Histogram range [{min}, {max}] cannot be represented with finite, distinct bin edges"
    ))
}

fn validate_bin_edges(min: f64, max: f64, bins: usize) -> Result<()> {
    let span = max - min;
    let width = span / bins as f64;
    if !min.is_finite()
        || !max.is_finite()
        || !span.is_finite()
        || !width.is_finite()
        || width <= 0.0
    {
        return Err(unrepresentable_range(min, max));
    }
    let mut previous = min;
    for bin in 1..=bins {
        let edge = if bin == bins {
            max
        } else {
            min + bin as f64 * width
        };
        if !edge.is_finite() || edge <= previous {
            return Err(unrepresentable_range(min, max));
        }
        previous = edge;
    }
    Ok(())
}

// The first and last bin edges and the number of bins.
fn binning(spread: Spread, bins: Option<usize>) -> Result<(f64, f64, usize)> {
    let Spread {
        min, max, whole, ..
    } = spread;
    let invalid = || unrepresentable_range(min, max);
    let span = max - min;
    if !span.is_finite() || span < 0.0 {
        return Err(invalid());
    }
    let (start, end, bins) = if span == 0.0 {
        let scale = if min == 0.0 { 1.0 } else { min.abs() };
        let padding = (scale.min(1.0) * 0.5)
            .max(scale * f64::EPSILON)
            .max(f64::from_bits(1));
        (min - padding, max + padding, 1)
    } else if let Some(bins) = bins {
        let bins = bins.clamp(1, HISTOGRAM_MAX_BINS);
        if whole {
            // whole bin widths so each bin holds the same number of whole values
            let values = span + 1.0;
            let width = (values / bins as f64).ceil();
            let bins = (values / width).ceil() as usize;
            let start = min - 0.5;
            (start, start + bins as f64 * width, bins)
        } else {
            (min, max, bins)
        }
    } else {
        let base = 10f64.powf((span / 100.0).log10().floor());
        let mut width = [5.0, 2.0, 1.0]
            .map(|step| step * base)
            .into_iter()
            .find(|&width| span / width >= 100.0)
            .unwrap_or(base);
        if !width.is_finite() || width <= 0.0 {
            return Err(invalid());
        }
        let mut start = (min / width).floor() * width;
        if whole {
            width = width.max(1.0);
            start = (min / width).floor() * width - 0.5;
        }
        let intervals = ((max - start) / width).floor();
        if !intervals.is_finite() || !(0.0..HISTOGRAM_MAX_BINS as f64).contains(&intervals) {
            return Err(invalid());
        }
        let bins = intervals as usize + 1;
        (start, start + bins as f64 * width, bins)
    };
    if start > min || end < max {
        return Err(invalid());
    }
    validate_bin_edges(start, end, bins)?;
    Ok((start, end, bins))
}

/// u64 words of one z slice in a [`mask_bits`] bitset
fn slice_words(meta: &VolumeMeta) -> usize {
    (meta.width() * meta.height()).div_ceil(64)
}

/// One bit per voxel of `values`, set where `mask` covers it. Each z slice starts a new word.
fn mask_bits(values: Source, mask: LabelMask, progress: &Progress) -> Result<Vec<u64>> {
    let meta = values.volume.meta();
    let (width, height, depth) = (meta.width(), meta.height(), meta.depth());
    let words = slice_words(meta);
    let mut bits = vec![0u64; words * depth];
    let done = AtomicUsize::new(0);
    bits.par_chunks_mut(words.max(1))
        .enumerate()
        .try_for_each(|(z, slice)| {
            for y in 0..height {
                for x in 0..width {
                    if mask.contains(values.geometry, [x, y, z]) {
                        let bit = x + width * y;
                        slice[bit / 64] |= 1u64 << (bit % 64);
                    }
                }
            }
            let done = done.fetch_add(1, Ordering::Relaxed) + 1;
            progress.report(done as f64 / depth as f64, "Finding the region")
        })?;
    Ok(bits)
}

/// The counts of two parts of a volume added bin by bin.
fn add_counts(mut a: Vec<u64>, b: Vec<u64>) -> Vec<u64> {
    for (a, b) in a.iter_mut().zip(b) {
        *a += b;
    }
    a
}

/// Folds the values of `values` that `counting` keeps, and that are set in `inside`  when there is one.
#[allow(clippy::too_many_arguments)]
fn fold_kept<A, I, F, M>(
    values: Source,
    counting: Counting,
    inside: Option<&[u64]>,
    progress: &Progress,
    stage: &str,
    init: I,
    add: F,
    merge: M,
) -> Result<A>
where
    A: Send,
    I: Fn() -> A + Sync + Send,
    F: Fn(A, f64) -> A + Sync + Send,
    M: Fn(A, A) -> A + Sync + Send,
{
    let meta = values.volume.meta();
    let scaling = meta.scaling;
    let bytes = values.volume.frame(values.frame);
    let kept = |value: f64| value.is_finite() && !(counting.ignore_zeros && value == 0.0);
    let Some(inside) = inside else {
        let add = |sum, raw| {
            let value = scaling.apply(raw);
            if kept(value) { add(sum, value) } else { sum }
        };
        return voxel::Values::new(bytes, meta.datatype, meta.endian)
            .fold(progress, stage, init, add, merge);
    };
    // only the voxels inside counted slice by slice
    let (slice_voxels, depth) = (meta.width() * meta.height(), meta.depth());
    let words = slice_words(meta);
    let done = AtomicUsize::new(0);
    (0..depth)
        .into_par_iter()
        .try_fold(&init, |mut sum, z| {
            for (word, &bits) in inside[z * words..(z + 1) * words].iter().enumerate() {
                let mut bits = bits;
                while bits != 0 {
                    let bit = word * 64 + bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let index = bit + slice_voxels * z;
                    let Some(raw) = voxel::read_value(bytes, index, meta.datatype, meta.endian)
                    else {
                        continue;
                    };
                    let value = scaling.apply(raw);
                    if kept(value) {
                        sum = add(sum, value);
                    }
                }
            }
            let done = done.fetch_add(1, Ordering::Relaxed) + 1;
            progress.report(done as f64 / depth as f64, stage)?;
            Ok(sum)
        })
        .try_reduce(&init, |a, b| Ok(merge(a, b)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::GeometryOptions;
    use crate::test_util::{Fixture, load};
    use crate::voxel::DataType;

    #[test]
    fn counts_values_into_bins() {
        let dir = tempfile::tempdir().unwrap();
        let steps = Fixture {
            datatype: DataType::F32,
            value: |i| (i % 4) as f64 * 10.0,
            ..Fixture::default()
        };
        let volume = load(&dir, &steps, "steps.nii");
        let histogram = Histogram::of(&volume, 0, 4, &Progress::default())
            .unwrap()
            .unwrap();
        assert_eq!((histogram.min, histogram.max), (0.0, 30.0));
        assert_eq!(histogram.counts, [12, 12, 12, 12]);
        assert_eq!(histogram.percentile(0.0), 0.0);
        assert_eq!(histogram.percentile(0.5), 15.0);
        assert_eq!(histogram.percentile(1.0), 30.0);
    }

    #[test]
    fn counts_subnormal_values_without_an_overflowing_reciprocal() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture {
            dims: vec![4, 1, 1],
            datatype: DataType::F64,
            value: |i| [0.0, 1e-310, 2e-310, 1e-309][i],
            ..Fixture::default()
        };
        let volume = load(&dir, &fixture, "subnormal.nii");
        let histogram = Histogram::of(&volume, 0, 2, &Progress::default())
            .unwrap()
            .unwrap();
        assert_eq!(histogram.counts, [3, 1]);
    }

    #[test]
    fn caps_sidebar_bin_allocations() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture {
            dims: vec![2, 1, 1],
            datatype: DataType::F64,
            value: |i| i as f64,
            ..Fixture::default()
        };
        let volume = load(&dir, &fixture, "bounds.nii");
        for (requested, expected) in [(0, 1), (usize::MAX, HISTOGRAM_MAX_BINS)] {
            let histogram = Histogram::of(&volume, 0, requested, &Progress::default())
                .unwrap()
                .unwrap();
            assert_eq!(histogram.counts.len(), expected);
            assert_eq!(histogram.total(), 2);
        }
    }

    #[test]
    fn rejects_overflowing_sidebar_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture {
            dims: vec![3, 1, 1],
            datatype: DataType::F64,
            value: |i| [-1e308, 0.0, 1e308][i],
            ..Fixture::default()
        };
        let volume = load(&dir, &fixture, "overflow.nii");
        let error = Histogram::of(&volume, 0, 2, &Progress::default()).unwrap_err();
        assert!(error.to_string().contains("Histogram range"));
    }

    #[test]
    fn falls_back_to_one_sidebar_bin_for_an_underflowing_width() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture {
            dims: vec![2, 1, 1],
            datatype: DataType::F64,
            value: |i| [0.0, f64::from_bits(1)][i],
            ..Fixture::default()
        };
        let volume = load(&dir, &fixture, "smallest_span.nii");
        for bins in [1, 2, 128, usize::MAX] {
            let histogram = Histogram::of(&volume, 0, bins, &Progress::default())
                .unwrap()
                .unwrap();
            assert_eq!((histogram.min, histogram.max), (0.0, f64::from_bits(1)));
            assert_eq!(histogram.bin_width(), f64::from_bits(1));
            assert_eq!(histogram.counts, [2]);
        }
    }

    #[test]
    fn falls_back_to_fewer_sidebar_bins_for_collapsed_offset_edges() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture {
            dims: vec![2, 1, 1],
            datatype: DataType::F64,
            value: |i| 1e16 + 2.0 * i as f64,
            ..Fixture::default()
        };
        let volume = load(&dir, &fixture, "narrow_offset.nii");
        for bins in [1, 2, 128] {
            let histogram = Histogram::of(&volume, 0, bins, &Progress::default())
                .unwrap()
                .unwrap();
            assert_eq!((histogram.min, histogram.max), (1e16, 1e16 + 2.0));
            assert_eq!(histogram.bin_width(), 2.0);
            assert_eq!(histogram.counts, [2]);
        }
        let wider = Fixture {
            value: |i| 1e16 + 8.0 * i as f64,
            ..fixture
        };
        let volume = load(&dir, &wider, "wider_offset.nii");
        let histogram = Histogram::of(&volume, 0, 128, &Progress::default())
            .unwrap()
            .unwrap();
        assert_eq!(histogram.counts, [1, 0, 0, 1]);
    }

    #[test]
    fn preserves_equal_bound_sidebar_constant_ranges() {
        let dir = tempfile::tempdir().unwrap();
        let constants: [fn(usize) -> f64; 4] = [
            |_| 7.0,
            |_| f64::from_bits(1),
            |_| 2f64.powi(53),
            |_| f64::MAX,
        ];
        for value in constants {
            let fixture = Fixture {
                dims: vec![2, 1, 1],
                datatype: DataType::F64,
                value,
                ..Fixture::default()
            };
            let volume = load(&dir, &fixture, "constant.nii");
            let histogram = Histogram::of(&volume, 0, 128, &Progress::default())
                .unwrap()
                .unwrap();
            assert_eq!((histogram.min, histogram.max), (value(0), value(0)));
            assert_eq!(histogram.counts.len(), 128);
            assert_eq!(histogram.counts[0], 2);
            assert_eq!(histogram.total(), 2);
            assert_eq!(histogram.bin_width(), 0.0);
            assert_eq!(histogram.percentile(0.5), value(0));
        }
    }

    #[test]
    fn has_no_histogram_without_finite_values() {
        let dir = tempfile::tempdir().unwrap();
        let empty = Fixture {
            datatype: DataType::F32,
            value: |_| f64::NAN,
            ..Fixture::default()
        };
        let volume = load(&dir, &empty, "nan.nii");
        assert_eq!(
            Histogram::of(&volume, 0, 8, &Progress::default()).unwrap(),
            None
        );
    }

    #[test]
    fn picks_round_bins() {
        let spread = |min, max, whole| Spread {
            min,
            max,
            count: 9,
            whole,
        };
        assert_eq!(
            binning(spread(0.0, 30.0, true), None).unwrap(),
            (-0.5, 30.5, 31)
        );
        assert_eq!(
            binning(spread(-1024.0, 3071.0, true), None).unwrap(),
            (-1040.5, 3079.5, 206)
        );
        assert_eq!(
            binning(spread(0.0, 50.0, false), None).unwrap(),
            (0.0, 50.5, 101)
        );
        assert_eq!(
            binning(spread(0.0, 50.0, false), Some(10)).unwrap(),
            (0.0, 50.0, 10)
        );
        assert_eq!(
            binning(spread(0.0, 1000.0, true), Some(512)).unwrap(),
            (-0.5, 1001.5, 501)
        );
        assert_eq!(
            binning(spread(0.0, 1000.0, true), Some(1024)).unwrap(),
            (-0.5, 1000.5, 1001)
        );
        assert_eq!(
            binning(spread(7.0, 7.0, true), None).unwrap(),
            (6.5, 7.5, 1)
        );
    }

    fn counted_f64(
        samples: u16,
        value: fn(usize) -> f64,
        bins: Option<usize>,
    ) -> Result<Histogram> {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture {
            dims: vec![samples, 1, 1],
            datatype: DataType::F64,
            value,
            ..Fixture::default()
        };
        let volume = load(&dir, &fixture, "f64.nii");
        let geometry = volume.geometry(GeometryOptions::default()).unwrap();
        Histogram::counted(
            Source {
                volume: &volume,
                geometry: &geometry,
                frame: 0,
            },
            Counting {
                bins,
                ignore_zeros: false,
            },
            None,
            &Progress::default(),
        )
        .map(|histogram| histogram.unwrap())
    }

    #[test]
    fn rejects_unrepresentable_extreme_counted_ranges() {
        let error = counted_f64(2, |i| [-f64::MAX, 0.0][i], None).unwrap_err();
        assert!(error.to_string().contains("finite, distinct bin edges"));
        for bins in [None, Some(2), Some(usize::MAX)] {
            let error = counted_f64(3, |i| [-1e308, 0.0, 1e308][i], bins).unwrap_err();
            assert!(error.to_string().contains("finite, distinct bin edges"));
        }
        for bins in [None, Some(2)] {
            let error = counted_f64(2, |i| [0.0, f64::from_bits(1)][i], bins).unwrap_err();
            assert!(error.to_string().contains("finite, distinct bin edges"));
        }
        let smallest = counted_f64(2, |i| [0.0, f64::from_bits(1)][i], Some(1)).unwrap();
        assert_eq!(smallest.bin_width(), f64::from_bits(1));
        assert_eq!(smallest.counts, [2]);
    }

    #[test]
    fn constant_counted_ranges_have_representable_padding() {
        let constants: [fn(usize) -> f64; 6] = [
            |_| 0.0,
            |_| 7.0,
            |_| 2f64.powi(53),
            |_| 1e-310,
            |_| -1e-310,
            |_| f64::from_bits(1),
        ];
        for value in constants {
            for bins in [None, Some(usize::MAX)] {
                let histogram = counted_f64(3, value, bins).unwrap();
                assert!(histogram.min < value(0));
                assert!(histogram.max > value(0));
                assert!(histogram.bin_width().is_finite());
                assert!(histogram.bin_width() > 0.0);
                assert_eq!(histogram.counts, [3]);
                assert_eq!(histogram.percentile(0.5), value(0));
            }
        }
        let error = counted_f64(2, |_| f64::MAX, None).unwrap_err();
        assert!(error.to_string().contains("finite, distinct bin edges"));
    }

    #[test]
    fn caps_counted_bins_and_rejects_collapsed_edges() {
        for (requested, expected) in [(0, 1), (usize::MAX, HISTOGRAM_MAX_BINS)] {
            let histogram = counted_f64(2, |i| i as f64 * 0.5, Some(requested)).unwrap();
            assert_eq!(histogram.counts.len(), expected);
            assert_eq!(histogram.total(), 2);
            assert_eq!(
                histogram.counts[expected - 1],
                if expected == 1 { 2 } else { 1 }
            );
        }
        let error = counted_f64(2, |i| 1e16 + 2.0 * i as f64, Some(100)).unwrap_err();
        assert!(error.to_string().contains("finite, distinct bin edges"));
        // whole values fill manual bins evenly, without a comb of dips
        let whole = counted_f64(1001, |i| i as f64, Some(512)).unwrap();
        assert_eq!(
            (whole.min, whole.bin_width(), whole.counts.len()),
            (-0.5, 2.0, 501)
        );
        assert!(whole.counts[..500].iter().all(|&count| count == 2));
        assert_eq!(whole.counts[500], 1);
    }

    #[test]
    fn keeps_explicit_counted_endpoints_without_rounding_them_inward() {
        let histogram = counted_f64(2, |i| [0.0, 0.9][i], Some(7)).unwrap();
        assert_eq!((histogram.min, histogram.max), (0.0, 0.9));
        assert_eq!(histogram.counts[0], 1);
        assert_eq!(histogram.counts[6], 1);
    }

    #[test]
    fn counts_inside_labels_and_without_zeros() {
        let dir = tempfile::tempdir().unwrap();
        let steps = Fixture {
            datatype: DataType::F32,
            value: |i| (i % 4) as f64 * 10.0,
            ..Fixture::default()
        };
        let labels = Fixture {
            value: |i| (i % 3) as f64,
            ..Fixture::default()
        };
        let (values, labels) = (
            load(&dir, &steps, "steps.nii"),
            load(&dir, &labels, "labels.nii"),
        );
        let options = GeometryOptions::default();
        let value_geometry = values.geometry(options).unwrap();
        let label_geometry = labels.geometry(options).unwrap();
        let source = |volume, geometry| Source {
            volume,
            geometry,
            frame: 0,
        };
        let mask = |label| {
            Some(LabelMask {
                labels: source(&labels, &label_geometry),
                label,
            })
        };
        let all = Counting {
            bins: None,
            ignore_zeros: false,
        };
        let count = |counting, mask| {
            let values = source(&values, &value_geometry);
            Histogram::counted(values, counting, mask, &Progress::default()).unwrap()
        };

        let whole = count(all, None).unwrap();
        assert_eq!((whole.min, whole.max, whole.counts.len()), (-0.5, 30.5, 31));
        assert_eq!(
            (whole.counts[0], whole.counts[30], whole.total()),
            (12, 12, 48)
        );
        let no_zeros = Counting {
            ignore_zeros: true,
            ..all
        };
        let nonzero = count(no_zeros, None).unwrap();
        assert_eq!((nonzero.min, nonzero.total()), (9.5, 36));

        let one = count(all, mask(Some(1))).unwrap();
        assert_eq!((one.total(), one.counts[10]), (16, 4));
        assert_eq!(count(all, mask(None)).unwrap().total(), 32);
        assert_eq!(count(all, mask(Some(7))), None);
        let three = Counting {
            bins: Some(3),
            ..all
        };
        let wide = count(three, None).unwrap();
        assert_eq!((wide.min, wide.max, wide.bin_width()), (-0.5, 32.5, 11.0));
    }

    #[test]
    fn stops_counting_when_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let volume = load(&dir, &Fixture::default(), "values.nii");
        let geometry = volume.geometry(GeometryOptions::default()).unwrap();
        let source = Source {
            volume: &volume,
            geometry: &geometry,
            frame: 0,
        };
        let counting = Counting {
            bins: None,
            ignore_zeros: false,
        };
        let cancelled = Progress::cancellable(std::sync::Arc::new(true.into()));
        let mask = LabelMask {
            labels: source,
            label: None,
        };
        for mask in [None, Some(mask)] {
            let result = Histogram::counted(source, counting, mask, &cancelled);
            assert!(matches!(result, Err(Error::Cancelled)));
        }
    }

    #[test]
    fn masks_slices_wider_than_one_word() {
        let dir = tempfile::tempdir().unwrap();
        let steps = Fixture {
            dims: vec![10, 10, 2],
            datatype: DataType::F32,
            value: |i| (i % 4) as f64 * 10.0,
            ..Fixture::default()
        };
        let labels = Fixture {
            dims: vec![10, 10, 2],
            value: |i| (i % 3) as f64,
            ..Fixture::default()
        };
        let (values, labels) = (
            load(&dir, &steps, "steps.nii"),
            load(&dir, &labels, "labels.nii"),
        );
        let options = GeometryOptions::default();
        let (value_geometry, label_geometry) = (
            values.geometry(options).unwrap(),
            labels.geometry(options).unwrap(),
        );
        let histogram = Histogram::counted(
            Source {
                volume: &values,
                geometry: &value_geometry,
                frame: 0,
            },
            Counting {
                bins: None,
                ignore_zeros: false,
            },
            Some(LabelMask {
                labels: Source {
                    volume: &labels,
                    geometry: &label_geometry,
                    frame: 0,
                },
                label: Some(1),
            }),
            &Progress::default(),
        )
        .unwrap()
        .unwrap();
        // voxels 1, 4, 7, ..., 199 have label 1
        assert_eq!(histogram.total(), 67);
        let at = |value: usize| histogram.counts[value];
        assert_eq!((at(0), at(10), at(20), at(30)), (17, 17, 16, 17));
    }

    #[test]
    fn csv_round_trips_distinct_edges_and_probabilities() {
        let histogram = Histogram {
            min: 1e12,
            max: 1e12 + 3.0,
            counts: vec![1, 2, 4],
        };
        let csv = to_csv([(0, "precision.nii", &histogram)]);
        let rows: Vec<_> = csv.lines().skip(1).collect();
        assert_eq!(rows.len(), 3);
        let mut below = 0;
        for (bin, row) in rows.iter().enumerate() {
            let fields: Vec<_> = row.split(',').collect();
            let start: f64 = fields[2].parse().unwrap();
            let end: f64 = fields[3].parse().unwrap();
            let probability: f64 = fields[5].parse().unwrap();
            let cumulative: f64 = fields[6].parse().unwrap();
            below += histogram.counts[bin];
            assert_eq!(start, histogram.min + bin as f64);
            assert_eq!(end, histogram.min + (bin + 1) as f64);
            assert_eq!(end - start, 1.0);
            assert_eq!(probability, histogram.counts[bin] as f64 / 7.0);
            assert_eq!(cumulative, below as f64 / 7.0);
        }

        let fractional = Histogram {
            min: 0.1 + 0.2,
            max: 2.0 / 3.0,
            counts: vec![1],
        };
        let csv = to_csv([(0, "fractional.nii", &fractional)]);
        let fields: Vec<_> = csv.lines().nth(1).unwrap().split(',').collect();
        assert_eq!(fields[2].parse::<f64>().unwrap(), fractional.min);
        assert_eq!(fields[3].parse::<f64>().unwrap(), fractional.max);
    }

    #[test]
    fn writes_one_row_per_bin() {
        let quarters = Histogram {
            min: -0.5,
            max: 2.5,
            counts: vec![1, 0, 3],
        };
        let tenths = Histogram {
            min: 0.0,
            max: 0.3,
            counts: vec![1, 1],
        };
        let csv = to_csv([(2, "a,b \"c\".nii", &quarters), (5, "pet.nii", &tenths)]);
        let expected = "layer,name,bin_start,bin_end,count,probability,cumulative\n\
            2,\"a,b \"\"c\"\".nii\",-0.5,0.5,1,0.25,0.25\n\
            2,\"a,b \"\"c\"\".nii\",0.5,1.5,0,0,0.25\n\
            2,\"a,b \"\"c\"\".nii\",1.5,2.5,3,0.75,1\n\
            5,pet.nii,0,0.15,1,0.5,0.5\n\
            5,pet.nii,0.15,0.3,1,0.5,1\n";
        assert_eq!(csv, expected);
    }
}
