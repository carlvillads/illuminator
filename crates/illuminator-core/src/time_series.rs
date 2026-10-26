//! Values of an area in every frame of a 4D volume.

use rayon::prelude::*;
use std::fmt::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::Result;
use crate::geometry::VolumeGeometry;
use crate::histogram::{LabelMask, quoted};
use crate::progress::Progress;
use crate::stats::{RunningStats, Summary};
use crate::volume::Volume;
use crate::voxel;

pub enum Area<'a> {
    Cube { center: [usize; 3], radius: usize },
    Labels(LabelMask<'a>),
}

/// Summary of the values inside `area` in each frame of `volume`
pub fn time_series(
    volume: &Volume,
    geometry: &VolumeGeometry,
    area: Area,
    progress: &Progress,
) -> Result<Vec<Option<Summary>>> {
    let voxels = area_voxels(volume, geometry, area, &progress.part(0.0, 0.2))?;
    let meta = volume.meta();
    let frames = meta.timepoints();
    let passes = progress.part(0.2, 0.8);
    let done = AtomicUsize::new(0);
    (0..frames)
        .into_par_iter()
        .map(|t| {
            let bytes = volume.frame(t);
            let mut values = RunningStats::default();
            for &index in &voxels {
                let Some(raw) = voxel::read_value(bytes, index, meta.datatype, meta.endian) else {
                    continue;
                };
                let value = meta.scaling.apply(raw);
                if value.is_finite() {
                    values.push(value);
                }
            }
            let done = done.fetch_add(1, Ordering::Relaxed) + 1;
            passes.report(done as f64 / frames as f64, "Measuring frames")?;
            Ok(values.summary())
        })
        .collect()
}

/// Indices of voxels of `volume` inside `area`
fn area_voxels(
    volume: &Volume,
    geometry: &VolumeGeometry,
    area: Area,
    progress: &Progress,
) -> Result<Vec<usize>> {
    let meta = volume.meta();
    let (width, height, depth) = (meta.width(), meta.height(), meta.depth());
    let index = |x: usize, y: usize, z: usize| x + width * (y + height * z);
    match area {
        Area::Cube {
            center: [x, y, z],
            radius,
        } => {
            // cut off at the edges of the volume
            let span = |center: usize, size: usize| {
                center.saturating_sub(radius)..=center.saturating_add(radius).min(size - 1)
            };
            let mut voxels = Vec::new();
            for z in span(z, depth) {
                for y in span(y, height) {
                    voxels.extend(span(x, width).map(|x| index(x, y, z)));
                }
            }
            Ok(voxels)
        }
        Area::Labels(mask) => {
            let done = AtomicUsize::new(0);
            let slices: Vec<Vec<usize>> = (0..depth)
                .into_par_iter()
                .map(|z| {
                    let mut inside = Vec::new();
                    for y in 0..height {
                        for x in 0..width {
                            if mask.contains(geometry, [x, y, z]) {
                                inside.push(index(x, y, z));
                            }
                        }
                    }
                    let done = done.fetch_add(1, Ordering::Relaxed) + 1;
                    progress.report(done as f64 / depth as f64, "Finding area")?;
                    Ok(inside)
                })
                .collect::<Result<_>>()?;
            Ok(slices.concat())
        }
    }
}

pub fn to_csv<'a>(
    series: impl IntoIterator<Item = (usize, &'a str, &'a [Option<Summary>])>,
) -> String {
    let mut csv = String::from("layer,name,frame,voxels,mean,std,min,max\n");
    for (layer, name, frames) in series {
        let name = quoted(name);
        for (frame, summary) in frames.iter().enumerate() {
            let _ = write!(csv, "{layer},{name},{}", frame + 1);
            match summary {
                Some(v) => {
                    let _ = writeln!(csv, ",{},{},{},{},{}", v.count, v.mean, v.std, v.min, v.max);
                }
                None => csv.push_str(",0,,,,\n"),
            }
        }
    }
    csv
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::GeometryOptions;
    use crate::stats::Source;
    use crate::test_util::{Fixture, load};

    fn assert_means(series: &[Option<Summary>], expected: &[f64]) {
        assert_eq!(series.len(), expected.len());
        for (summary, expected) in series.iter().zip(expected) {
            assert!((summary.unwrap().mean - expected).abs() < 1e-9);
        }
    }

    #[test]
    fn measures_an_area_in_every_frame() {
        let dir = tempfile::tempdir().unwrap();
        let movie = Fixture {
            dims: vec![4, 3, 4, 2],
            ..Fixture::default()
        };
        let volume = load(&dir, &movie, "movie.nii");
        let geometry = volume.geometry(GeometryOptions::default()).unwrap();
        let measure = |area| time_series(&volume, &geometry, area, &Progress::default()).unwrap();

        let voxel = measure(Area::Cube {
            center: [1, 0, 0],
            radius: 0,
        });
        assert_means(&voxel, &volume.time_curve([1, 0, 0]).unwrap());
        let corner = measure(Area::Cube {
            center: [0, 0, 0],
            radius: 1,
        });
        assert_means(&corner, &[8.5, 56.5]);
        assert_eq!(corner[0].unwrap().count, 8);
        assert!(to_csv([(1, "movie", &corner[..])]).contains("\n1,movie,2,8,"));

        let labels = Fixture {
            value: |i| (i % 3) as f64,
            ..Fixture::default()
        };
        let label_volume = load(&dir, &labels, "labels.nii");
        let label_geometry = label_volume.geometry(GeometryOptions::default()).unwrap();
        let on = |label| {
            Area::Labels(LabelMask {
                labels: Source {
                    volume: &label_volume,
                    geometry: &label_geometry,
                    frame: 0,
                },
                label,
            })
        };
        assert_means(&measure(on(Some(1))), &[23.5, 71.5]);
        assert_eq!(measure(on(Some(7))), vec![None, None]);
    }
}
