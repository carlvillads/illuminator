use rayon::prelude::*;
use std::collections::HashMap;
use std::fmt::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::Result;
use crate::geometry::{Point3D, VolumeGeometry};
use crate::progress::Progress;
use crate::volume::Volume;
use crate::voxel;

#[derive(Clone, Copy)]
pub struct Source<'a> {
    pub volume: &'a Volume,
    pub geometry: &'a VolumeGeometry,
    pub frame: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub label: i64,
    pub voxels: usize,
    pub volume_ml: f64,
    pub values: Option<Summary>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub count: usize,
    pub mean: f64,
    pub std: f64,
    pub min: f64,
    pub max: f64,
}

pub fn label_statistics(
    labels: Source,
    values: Option<Source>,
    progress: &Progress,
) -> Result<Vec<Region>> {
    let meta = labels.volume.meta();
    let [width, height, depth] = labels.geometry.dimensions;
    let bytes = labels.volume.frame(labels.frame);
    let done = AtomicUsize::new(0);
    let regions = (0..depth)
        .into_par_iter()
        .try_fold(
            HashMap::new,
            |mut regions: HashMap<i64, Counts>, z| -> Result<_> {
                progress.check()?;
                for y in 0..height {
                    for x in 0..width {
                        let index = x + width * (y + height * z);
                        let raw = voxel::read_value(bytes, index, meta.datatype, meta.endian)
                            .unwrap_or(0.0);
                        let label = voxel::label_id(meta.scaling.apply(raw))?;
                        if label == 0 {
                            continue;
                        }
                        let counts = regions.entry(label).or_default();
                        counts.voxels += 1;
                        let voxel = Point3D::new(x as f64, y as f64, z as f64);
                        let world = labels.geometry.voxel_to_world.point(voxel);
                        if let Some(value) = values.and_then(|values| value_at(values, world)) {
                            counts.values.push(value);
                        }
                    }
                }
                let done = done.fetch_add(1, Ordering::Relaxed) + 1;
                progress.report(done as f64 / depth as f64, "Measuring labels")?;
                Ok(regions)
            },
        )
        .try_reduce(HashMap::new, |mut a, b| {
            for (label, counts) in b {
                let merged = a.remove(&label).unwrap_or_default().merge(counts);
                a.insert(label, merged);
            }
            Ok(a)
        })?;

    let voxel_ml = voxel_volume_mm3(labels.geometry) / 1000.0;
    let mut regions: Vec<Region> = regions
        .into_iter()
        .map(|(label, counts)| Region {
            label,
            voxels: counts.voxels,
            volume_ml: counts.voxels as f64 * voxel_ml,
            values: counts.values.summary(),
        })
        .collect();
    regions.sort_by_key(|region| region.label);
    Ok(regions)
}

fn value_at(values: Source, world: Point3D) -> Option<f64> {
    let voxel = values.geometry.world_to_voxel.point(world);
    if !values.geometry.contains_voxel(voxel) {
        return None;
    }
    let nearest = [0, 1, 2].map(|axis| (voxel[axis] + 0.5).floor() as usize);
    values
        .volume
        .sample_value(nearest, values.frame)
        .filter(|value| value.is_finite())
}

fn voxel_volume_mm3(geometry: &VolumeGeometry) -> f64 {
    let m = &geometry.voxel_to_world.m;
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    det.abs()
}

#[derive(Default)]
struct Counts {
    voxels: usize,
    values: Running,
}

impl Counts {
    fn merge(self, other: Self) -> Self {
        Self {
            voxels: self.voxels + other.voxels,
            values: self.values.merge(other.values),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Running {
    count: usize,
    mean: f64,
    m2: f64,
    min: f64,
    max: f64,
}

impl Default for Running {
    fn default() -> Self {
        Self {
            count: 0,
            mean: 0.0,
            m2: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }
}

impl Running {
    fn push(&mut self, value: f64) {
        self.count += 1;
        let delta = value - self.mean;
        self.mean += delta / self.count as f64;
        self.m2 += delta * (value - self.mean);
        self.min = self.min.min(value);
        self.max = self.max.max(value);
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
            m2: self.m2 + other.m2 + delta * delta * self.count as f64 * share,
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    fn summary(self) -> Option<Summary> {
        (self.count > 0).then(|| Summary {
            count: self.count,
            mean: self.mean,
            std: if self.count > 1 {
                (self.m2 / (self.count - 1) as f64).sqrt()
            } else {
                0.0
            },
            min: self.min,
            max: self.max,
        })
    }
}

pub fn to_csv(regions: &[Region]) -> String {
    let mut csv = String::from("label,voxels,volume_ml,values,mean,std,min,max\n");
    for region in regions {
        let _ = write!(
            csv,
            "{},{},{}",
            region.label, region.voxels, region.volume_ml
        );
        match region.values {
            Some(v) => {
                let _ = writeln!(csv, ",{},{},{},{},{}", v.count, v.mean, v.std, v.min, v.max);
            }
            None => csv.push_str(",0,,,,\n"),
        }
    }
    csv
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::VolumeCache;
    use crate::geometry::GeometryOptions;
    use crate::test_util::Fixture;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn running_statistics_merge_like_one_pass() {
        let values = [3.0, -1.0, 4.0, 1.0, 5.0, 9.0, 2.0];
        let mut whole = Running::default();
        values.iter().for_each(|&v| whole.push(v));
        let (mut a, mut b) = (Running::default(), Running::default());
        values[..3].iter().for_each(|&v| a.push(v));
        values[3..].iter().for_each(|&v| b.push(v));
        let (whole, merged) = (whole.summary().unwrap(), a.merge(b).summary().unwrap());
        assert!(close(whole.mean, merged.mean) && close(whole.std, merged.std));
        assert_eq!((merged.min, merged.max, merged.count), (-1.0, 9.0, 7));
        let mut five = Running::default();
        (1..=5).for_each(|v| five.push(f64::from(v)));
        assert!(close(five.summary().unwrap().std, 2.5f64.sqrt()));
    }

    #[test]
    fn measures_values_inside_labels() {
        let dir = tempfile::tempdir().unwrap();
        let cache = VolumeCache::new(dir.path().join("cache"), 1 << 20).unwrap();
        let load = |fixture: &Fixture, name: &str| {
            let path = dir.path().join(name);
            fixture.write(&path);
            let volume = Volume::load(&path, &cache, &Progress::default()).unwrap();
            let geometry = volume.geometry(GeometryOptions::default()).unwrap();
            (volume, geometry)
        };
        let labels = Fixture {
            value: |i| (i % 3) as f64,
            ..Fixture::default()
        };
        let (label_volume, label_geometry) = load(&labels, "labels.nii");
        let (value_volume, value_geometry) = load(&Fixture::default(), "values.nii");
        let source = |volume, geometry| Source {
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
        assert_eq!((one.label, one.voxels), (1, 16));
        assert!(close(one.volume_ml, 0.016));
        let expected: Vec<f64> = (1..48).step_by(3).map(|i| i as f64).collect();
        let mean = expected.iter().sum::<f64>() / 16.0;
        let variance = expected.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / 15.0;
        let values = one.values.unwrap();
        assert!(close(values.mean, mean) && close(values.std, variance.sqrt()));
        assert_eq!((values.min, values.max, values.count), (1.0, 46.0, 16));

        // without values, only what the labels cover
        let volumes = label_statistics(
            source(&label_volume, &label_geometry),
            None,
            &Progress::default(),
        )
        .unwrap();
        assert_eq!(volumes[1].voxels, 16);
        assert_eq!(volumes[1].values, None);
        assert!(to_csv(&volumes).ends_with("2,16,0.016,0,,,,\n"));
    }

    #[test]
    fn follows_the_world_across_grids() {
        let dir = tempfile::tempdir().unwrap();
        let cache = VolumeCache::new(dir.path().join("cache"), 1 << 20).unwrap();
        let labels = Fixture {
            dims: vec![2, 2, 2],
            value: |i| if i == 0 { 5.0 } else { 0.0 },
            sform: [
                [2.0, 0.0, 0.0, 1.0],
                [0.0, 2.0, 0.0, 1.0],
                [0.0, 0.0, 2.0, 1.0],
            ],
            ..Fixture::default()
        };
        let label_path = dir.path().join("coarse.nii");
        labels.write(&label_path);
        let value_path = dir.path().join("fine.nii");
        Fixture::default().write(&value_path);
        let label_volume = Volume::load(&label_path, &cache, &Progress::default()).unwrap();
        let value_volume = Volume::load(&value_path, &cache, &Progress::default()).unwrap();
        let label_geometry = label_volume.geometry(GeometryOptions::default()).unwrap();
        let value_geometry = value_volume.geometry(GeometryOptions::default()).unwrap();
        let regions = label_statistics(
            Source {
                volume: &label_volume,
                geometry: &label_geometry,
                frame: 0,
            },
            Some(Source {
                volume: &value_volume,
                geometry: &value_geometry,
                frame: 0,
            }),
            &Progress::default(),
        )
        .unwrap();
        assert_eq!((regions.len(), regions[0].label), (1, 5));
        assert!(close(regions[0].volume_ml, 0.008));
        let values = regions[0].values.unwrap();
        assert_eq!((values.count, values.mean), (1, 17.0));
    }
}
