//! Label statistics. The size of each label, and a summary of another volume's values inside it.

use rayon::prelude::*;
use std::collections::HashMap;
use std::fmt::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::Result;
use crate::geometry::{Point3D, VolumeGeometry};
use crate::labels::label_id;
use crate::progress::Progress;
use crate::volume::Volume;
use crate::voxel;

#[derive(Clone, Copy)]
pub struct VolumeFrame<'a> {
    pub volume: &'a Volume,
    pub geometry: &'a VolumeGeometry,
    pub frame: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub label: i64,
    pub voxel_count: usize,
    pub volume_ml: f64,
    pub value_summary: Option<ValueSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValueSummary {
    pub(crate) count: usize,
    pub mean: f64,
    pub standard_deviation: f64,
    pub minimum: f64,
    pub maximum: f64,
}

pub fn label_statistics(
    labels: VolumeFrame,
    values: Option<VolumeFrame>,
    progress: &Progress,
) -> Result<Vec<Region>> {
    let depth = labels.geometry.dimensions[2];
    let done = AtomicUsize::new(0);
    let regions = (0..depth)
        .into_par_iter()
        .try_fold(
            HashMap::new,
            |mut regions: HashMap<i64, RegionAccumulator>, z| -> Result<_> {
                progress.check_cancelled()?;
                measure_slice(labels, values, z, &mut regions)?;
                let done = done.fetch_add(1, Ordering::Relaxed) + 1;
                progress.report(done as f64 / depth as f64, "Measuring labels")?;
                Ok(regions)
            },
        )
        .try_reduce(HashMap::new, |a, b| Ok(merge_regions(a, b)))?;

    let voxel_volume_ml = labels.geometry.voxel_to_world.determinant().abs() / 1000.0;
    let mut regions: Vec<Region> = regions
        .into_iter()
        .map(|(label, counts)| Region {
            label,
            voxel_count: counts.voxels,
            volume_ml: counts.voxels as f64 * voxel_volume_ml,
            value_summary: counts.values.summary(),
        })
        .collect();
    regions.sort_by_key(|region| region.label);
    Ok(regions)
}

/// Adds every labelled voxel of slice `z` to `regions`
fn measure_slice(
    labels: VolumeFrame,
    values: Option<VolumeFrame>,
    z: usize,
    regions: &mut HashMap<i64, RegionAccumulator>,
) -> Result<()> {
    let metadata = labels.volume.metadata();
    let [width, height, _] = labels.geometry.dimensions;
    let bytes = labels.volume.frame_bytes(labels.frame);
    for y in 0..height {
        for x in 0..width {
            let index = x + width * (y + height * z);
            let raw =
                voxel::read_value(bytes, index, metadata.data_type, metadata.endian).unwrap_or(0.0);
            let label = label_id(metadata.scaling.apply(raw))?;
            if label == 0 {
                continue;
            }
            let counts = regions.entry(label).or_default();
            counts.voxels += 1;
            let voxel = Point3D::new(x as f64, y as f64, z as f64);
            let world = labels.geometry.voxel_to_world.transform_point(voxel);
            if let Some(value) = values.and_then(|values| value_at(values, world)) {
                counts.values.push(value);
            }
        }
    }
    Ok(())
}

/// Combines the regions that two threads measured
fn merge_regions(
    mut a: HashMap<i64, RegionAccumulator>,
    b: HashMap<i64, RegionAccumulator>,
) -> HashMap<i64, RegionAccumulator> {
    for (label, counts) in b {
        let merged = a.remove(&label).unwrap_or_default().merge(counts);
        a.insert(label, merged);
    }
    a
}

fn value_at(values: VolumeFrame, world: Point3D) -> Option<f64> {
    let nearest = values.geometry.nearest_voxel(world)?;
    values
        .volume
        .sample_value(nearest, values.frame)
        .filter(|value| value.is_finite())
}

#[derive(Default)]
struct RegionAccumulator {
    voxels: usize,
    values: RunningStatistics,
}

impl RegionAccumulator {
    fn merge(self, other: Self) -> Self {
        Self {
            voxels: self.voxels + other.voxels,
            values: self.values.merge(other.values),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RunningStatistics {
    count: usize,
    mean: f64,
    sum_of_squared_deviations: f64,
    minimum: f64,
    maximum: f64,
}

impl Default for RunningStatistics {
    fn default() -> Self {
        Self {
            count: 0,
            mean: 0.0,
            sum_of_squared_deviations: 0.0,
            minimum: f64::INFINITY,
            maximum: f64::NEG_INFINITY,
        }
    }
}

impl RunningStatistics {
    pub(crate) fn push(&mut self, value: f64) {
        self.count += 1;
        let delta = value - self.mean;
        self.mean += delta / self.count as f64;
        self.sum_of_squared_deviations += delta * (value - self.mean);
        self.minimum = self.minimum.min(value);
        self.maximum = self.maximum.max(value);
    }

    fn merge(self, other: Self) -> Self {
        if self.count == 0 {
            return other;
        }
        if other.count == 0 {
            return self;
        }
        let count = self.count + other.count;
        let delta = other.mean - self.mean;
        let share = other.count as f64 / count as f64;
        Self {
            count,
            mean: self.mean + delta * share,
            sum_of_squared_deviations: self.sum_of_squared_deviations
                + other.sum_of_squared_deviations
                + delta * delta * self.count as f64 * share,
            minimum: self.minimum.min(other.minimum),
            maximum: self.maximum.max(other.maximum),
        }
    }

    pub(crate) fn summary(self) -> Option<ValueSummary> {
        (self.count > 0).then(|| ValueSummary {
            count: self.count,
            mean: self.mean,
            standard_deviation: if self.count > 1 {
                (self.sum_of_squared_deviations / (self.count - 1) as f64).sqrt()
            } else {
                0.0
            },
            minimum: self.minimum,
            maximum: self.maximum,
        })
    }
}

pub fn to_csv(regions: &[Region]) -> String {
    let mut csv = String::from("label,voxels,volume_ml,values,mean,std,min,max\n");
    for region in regions {
        let _ = write!(
            csv,
            "{},{},{}",
            region.label, region.voxel_count, region.volume_ml
        );
        match region.value_summary {
            Some(v) => {
                let _ = writeln!(
                    csv,
                    ",{},{},{},{},{}",
                    v.count, v.mean, v.standard_deviation, v.minimum, v.maximum
                );
            }
            None => csv.push_str(",0,,,,\n"),
        }
    }
    csv
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::GeometryOptions;
    use crate::test_fixtures::{Fixture, write_and_load};

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn running_statistics_merge_like_one_pass() {
        let values = [3.0, -1.0, 4.0, 1.0, 5.0, 9.0, 2.0];
        let mut whole = RunningStatistics::default();
        values.iter().for_each(|&v| whole.push(v));
        let (mut a, mut b) = (RunningStatistics::default(), RunningStatistics::default());
        values[..3].iter().for_each(|&v| a.push(v));
        values[3..].iter().for_each(|&v| b.push(v));
        let (whole, merged) = (whole.summary().unwrap(), a.merge(b).summary().unwrap());
        assert!(
            close(whole.mean, merged.mean)
                && close(whole.standard_deviation, merged.standard_deviation)
        );
        assert_eq!(
            (merged.minimum, merged.maximum, merged.count),
            (-1.0, 9.0, 7)
        );
        let mut five = RunningStatistics::default();
        (1..=5).for_each(|v| five.push(f64::from(v)));
        assert!(close(
            five.summary().unwrap().standard_deviation,
            2.5f64.sqrt()
        ));
    }

    #[test]
    fn measures_values_inside_labels() {
        let directory = tempfile::tempdir().unwrap();
        let load_with_geometry = |fixture: &Fixture, name: &str| {
            let volume = write_and_load(&directory, fixture, name);
            let geometry = volume.geometry(GeometryOptions::default()).unwrap();
            (volume, geometry)
        };
        let labels = Fixture {
            value: |i| (i % 3) as f64,
            ..Fixture::default()
        };
        let (label_volume, label_geometry) = load_with_geometry(&labels, "labels.nii");
        let (value_volume, value_geometry) = load_with_geometry(&Fixture::default(), "values.nii");
        let source = |volume, geometry| VolumeFrame {
            volume,
            geometry,
            frame: 0,
        };
        let regions = label_statistics(
            source(&label_volume, &label_geometry),
            Some(source(&value_volume, &value_geometry)),
            &Progress::default(),
        )
        .unwrap();

        assert_eq!(regions.len(), 2);
        let one = &regions[0];
        assert_eq!((one.label, one.voxel_count), (1, 16));
        assert!(close(one.volume_ml, 0.016));
        let expected: Vec<f64> = (1..48).step_by(3).map(|i| i as f64).collect();
        let mean = expected.iter().sum::<f64>() / 16.0;
        let variance = expected.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / 15.0;
        let values = one.value_summary.unwrap();
        assert!(close(values.mean, mean) && close(values.standard_deviation, variance.sqrt()));
        assert_eq!(
            (values.minimum, values.maximum, values.count),
            (1.0, 46.0, 16)
        );

        // without values, only what the labels cover
        let volumes = label_statistics(
            source(&label_volume, &label_geometry),
            None,
            &Progress::default(),
        )
        .unwrap();
        assert_eq!(volumes[1].voxel_count, 16);
        assert_eq!(volumes[1].value_summary, None);
        assert!(to_csv(&volumes).ends_with("2,16,0.016,0,,,,\n"));
    }

    #[test]
    fn follows_the_world_across_grids() {
        let directory = tempfile::tempdir().unwrap();
        let labels = Fixture {
            dimensions: vec![2, 2, 2],
            value: |i| if i == 0 { 5.0 } else { 0.0 },
            sform: [
                [2.0, 0.0, 0.0, 1.0],
                [0.0, 2.0, 0.0, 1.0],
                [0.0, 0.0, 2.0, 1.0],
            ],
            ..Fixture::default()
        };
        let label_volume = write_and_load(&directory, &labels, "coarse.nii");
        let value_volume = write_and_load(&directory, &Fixture::default(), "fine.nii");
        let label_geometry = label_volume.geometry(GeometryOptions::default()).unwrap();
        let value_geometry = value_volume.geometry(GeometryOptions::default()).unwrap();
        let regions = label_statistics(
            VolumeFrame {
                volume: &label_volume,
                geometry: &label_geometry,
                frame: 0,
            },
            Some(VolumeFrame {
                volume: &value_volume,
                geometry: &value_geometry,
                frame: 0,
            }),
            &Progress::default(),
        )
        .unwrap();
        assert_eq!((regions.len(), regions[0].label), (1, 5));
        assert!(close(regions[0].volume_ml, 0.008));
        let values = regions[0].value_summary.unwrap();
        assert_eq!((values.count, values.mean), (1, 17.0));
    }
}
