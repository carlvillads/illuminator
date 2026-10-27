//! Values of a region in every frame of a 4D volume.

use rayon::prelude::*;
use std::fmt::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::Result;
use crate::geometry::VolumeGeometry;
use crate::histogram::{LabelMask, quote_csv_field};
use crate::progress::Progress;
use crate::statistics::{RunningStatistics, ValueSummary};
use crate::volume::Volume;
use crate::voxel;

pub enum RegionOfInterest<'a> {
    Cube {
        center: [usize; 3],
        half_width: usize,
    },
    LabelMask(LabelMask<'a>),
}

/// Summary of the values inside `region` in each frame of `volume`
pub fn time_series(
    volume: &Volume,
    geometry: &VolumeGeometry,
    region: RegionOfInterest,
    progress: &Progress,
) -> Result<Vec<Option<ValueSummary>>> {
    let voxels = region_voxel_indices(volume, geometry, region, &progress.subrange(0.0, 0.2))?;
    let metadata = volume.metadata();
    let frames = metadata.timepoints();
    let passes = progress.subrange(0.2, 0.8);
    let done = AtomicUsize::new(0);
    (0..frames)
        .into_par_iter()
        .map(|frame| {
            let bytes = volume.frame_bytes(frame);
            let mut values = RunningStatistics::default();
            for &index in &voxels {
                let Some(raw) =
                    voxel::read_value(bytes, index, metadata.data_type, metadata.endian)
                else {
                    continue;
                };
                let value = metadata.scaling.apply(raw);
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

/// Indices of voxels of `volume` inside `region`
fn region_voxel_indices(
    volume: &Volume,
    geometry: &VolumeGeometry,
    region: RegionOfInterest,
    progress: &Progress,
) -> Result<Vec<usize>> {
    let metadata = volume.metadata();
    let (width, height, depth) = (metadata.width(), metadata.height(), metadata.depth());
    let index = |x: usize, y: usize, z: usize| x + width * (y + height * z);
    match region {
        RegionOfInterest::Cube {
            center: [x, y, z],
            half_width: radius,
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
        RegionOfInterest::LabelMask(mask) => {
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
                    progress.report(done as f64 / depth as f64, "Finding region")?;
                    Ok(inside)
                })
                .collect::<Result<_>>()?;
            Ok(slices.concat())
        }
    }
}

pub fn to_csv<'a>(
    series: impl IntoIterator<Item = (usize, &'a str, &'a [Option<ValueSummary>])>,
) -> String {
    let mut csv = String::from("layer,name,frame,voxels,mean,std,min,max\n");
    for (layer, name, frames) in series {
        let name = quote_csv_field(name);
        for (frame, summary) in frames.iter().enumerate() {
            let _ = write!(csv, "{layer},{name},{}", frame + 1);
            match summary {
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
    }
    csv
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::GeometryOptions;
    use crate::statistics::VolumeFrame;
    use crate::test_fixtures::{Fixture, write_and_load};

    fn assert_means(series: &[Option<ValueSummary>], expected: &[f64]) {
        assert_eq!(series.len(), expected.len());
        for (summary, expected) in series.iter().zip(expected) {
            assert!((summary.unwrap().mean - expected).abs() < 1e-9);
        }
    }

    #[test]
    fn measures_a_region_in_every_frame() {
        let directory = tempfile::tempdir().unwrap();
        let movie = Fixture {
            dimensions: vec![4, 3, 4, 2],
            ..Fixture::default()
        };
        let volume = write_and_load(&directory, &movie, "movie.nii");
        let geometry = volume.geometry(GeometryOptions::default()).unwrap();
        let measure =
            |region| time_series(&volume, &geometry, region, &Progress::default()).unwrap();

        let voxel = measure(RegionOfInterest::Cube {
            center: [1, 0, 0],
            half_width: 0,
        });
        assert_means(&voxel, &volume.time_curve([1, 0, 0]).unwrap());
        let corner = measure(RegionOfInterest::Cube {
            center: [0, 0, 0],
            half_width: 1,
        });
        assert_means(&corner, &[8.5, 56.5]);
        assert_eq!(corner[0].unwrap().count, 8);
        assert!(to_csv([(1, "movie", &corner[..])]).contains("\n1,movie,2,8,"));

        let labels = Fixture {
            value: |i| (i % 3) as f64,
            ..Fixture::default()
        };
        let label_volume = write_and_load(&directory, &labels, "labels.nii");
        let label_geometry = label_volume.geometry(GeometryOptions::default()).unwrap();
        let on = |label| {
            RegionOfInterest::LabelMask(LabelMask {
                label_volume: VolumeFrame {
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
