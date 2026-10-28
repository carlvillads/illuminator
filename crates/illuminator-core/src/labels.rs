//! Label maps. Counting their labels and giving each label a distinct color.

use std::collections::HashSet;

use crate::error::{Error, Result};
use crate::progress::Progress;
use crate::volume::Volume;
use crate::voxel;

/// The distinct labels in `frame` from low to high without background.
/// Fails if a voxel isn't an integer or if there are more than `limit` labels.
pub(crate) fn find_labels(
    volume: &Volume,
    frame: usize,
    limit: usize,
    progress: &Progress,
) -> Result<Vec<i64>> {
    let metadata = volume.metadata();
    let voxel_count = metadata.voxels_per_frame();
    let mut labels = HashSet::new();
    // neighboring voxels usually share a label, so only look at changes
    let mut previous = 0;
    voxel::for_each_value(
        volume.frame_bytes(frame),
        metadata.data_type,
        metadata.endian,
        |i, raw| {
            if i % 65536 == 0 {
                progress.report(i as f64 / voxel_count as f64, "Preparing labels")?;
            }
            let label = label_id(metadata.scaling.apply(raw))?;
            if label != previous {
                previous = label;
                if label != 0 && labels.insert(label) && labels.len() > limit {
                    return Err(Error::invalid(format!(
                        "More than {limit} distinct labels. Are you sure this is a label map?"
                    )));
                }
            }
            Ok(())
        },
    )?;
    progress.report(1.0, "Prepared labels")?;
    let mut labels: Vec<i64> = labels.into_iter().collect();
    labels.sort_unstable();
    Ok(labels)
}

/// The label ID of a voxel value. Fails unless the value is a whole number that fits an `i64`.
pub(crate) fn label_id(value: f64) -> Result<i64> {
    // avoid that i64::MAX rounds up
    const POWER_63: f64 = 9223372036854775808_f64; // 2^63
    if !value.is_finite() || value != value.trunc() || !(-POWER_63..POWER_63).contains(&value) {
        return Err(Error::invalid(
            "Segmentation labels must be finite and integral 64-bit IDs",
        ));
    }
    Ok(value as i64)
}

pub fn label_color(label: i64) -> [u8; 4] {
    if label == 0 {
        return [0, 0, 0, 0];
    }
    // splitmix64
    let mut hash = (label as u64).wrapping_add(0x9e37_79b9_7f4a_7c15);
    hash = (hash ^ (hash >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    hash = (hash ^ (hash >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    hash ^= hash >> 31;
    let hue = hash as u32 as f32 / 4294967296.0;
    let saturation = 0.65 + ((hash >> 32) % 3) as f32 * 0.1;
    let value = 0.8 + ((hash >> 36) % 2) as f32 * 0.15;
    let [r, g, b] = hsv_to_rgb(hue, saturation, value).map(|c| (c * 255.0).round() as u8);
    [r, g, b, 255]
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let sector = (h * 6.0).floor();
    let f = h * 6.0 - sector;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    match sector as i32 % 6 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        5 => [v, p, q],
        _ => [0.0; 3],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{Fixture, write_and_load};
    use crate::voxel::DataType;

    #[test]
    fn counts_the_labels_of_one_frame() {
        let directory = tempfile::tempdir().unwrap();
        let fixture = Fixture {
            datatype: DataType::I32,
            dimensions: vec![4, 3, 4, 2],
            value: |i| {
                const IDS: [f64; 4] = [0.0, 300.0, 1000.0, 16_777_217.0];
                if i < 48 { IDS[i % 4] } else { IDS[i % 2] }
            },
            ..Fixture::default()
        };
        let volume = write_and_load(&directory, &fixture, "labels.nii");
        let none = Progress::default();

        assert_eq!(
            find_labels(&volume, 0, 256, &none).unwrap(),
            [300, 1000, 16_777_217]
        );
        assert_eq!(find_labels(&volume, 1, 256, &none).unwrap(), [300]);
        assert!(find_labels(&volume, 0, 2, &none).is_err());
    }

    #[test]
    fn rejects_fractional_labels() {
        let directory = tempfile::tempdir().unwrap();
        let fixture = Fixture {
            datatype: DataType::F32,
            value: |i| i as f64 + 0.25,
            ..Fixture::default()
        };
        let volume = write_and_load(&directory, &fixture, "labels.nii");
        assert!(find_labels(&volume, 0, 256, &Progress::default()).is_err());
    }

    #[test]
    fn label_ids() {
        assert_eq!(label_id(42.0).unwrap(), 42);
        assert_eq!(label_id(-(2f64.powi(63))).unwrap(), i64::MIN);
        assert!(label_id(1.5).is_err());
        assert!(label_id(f64::NAN).is_err());
        assert!(label_id(2f64.powi(63)).is_err());
    }

    #[test]
    fn colors_match_the_cpp_viewer() {
        assert_eq!(label_color(0), [0, 0, 0, 0]);
        let expected = [
            (1, [51, 172, 204, 255]),
            (2, [242, 182, 61, 255]),
            (117, [242, 36, 75, 255]),
            (-5, [51, 204, 88, 255]),
            (16_777_217, [85, 109, 242, 255]),
        ];
        for (label, color) in expected {
            assert_eq!(label_color(label), color, "label {label}");
        }
    }
}
