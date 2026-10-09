use crate::error::Result;
use crate::progress::Progress;
use crate::volume::Volume;
use crate::voxel;

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
        let h = volume.meta();
        let bins = bins.max(1);
        let last = bins - 1;
        let per_value = if range.max > range.min {
            bins as f64 / (range.max - range.min)
        } else {
            0.0
        };
        let scaling = h.scaling;
        let counts = voxel::Values::new(volume.frame(t), h.datatype, h.endian).fold(
            &progress.part(0.5, 0.5),
            "Counting values",
            || vec![0u64; bins],
            |mut counts, raw| {
                let value = scaling.apply(raw);
                if value.is_finite() {
                    let bin = ((value - range.min) * per_value) as usize;
                    counts[bin.min(last)] += 1;
                }
                counts
            },
            |mut a, b| {
                for (a, b) in a.iter_mut().zip(b) {
                    *a += b;
                }
                a
            },
        )?;
        progress.report(1.0, "Counted values")?;
        Ok(Some(Self {
            min: range.min,
            max: range.max,
            counts,
        }))
    }

    pub fn bin_width(&self) -> f64 {
        (self.max - self.min) / self.counts.len() as f64
    }

    pub fn percentile(&self, fraction: f64) -> f64 {
        let total: u64 = self.counts.iter().sum();
        let target = fraction.clamp(0.0, 1.0) * total as f64;
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

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::cache::VolumeCache;
    use crate::test_util::Fixture;
    use crate::voxel::DataType;

    fn load(dir: &TempDir, fixture: &Fixture, name: &str) -> Volume {
        let path = dir.path().join(name);
        fixture.write(&path);
        let cache = VolumeCache::new(dir.path().join("cache"), 1 << 20).unwrap();
        Volume::load(&path, &cache, &Progress::default()).unwrap()
    }

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
}
