use std::collections::HashSet;

use crate::error::{Error, Result};
use crate::progress::Progress;
use crate::volume::Volume;
use crate::voxel;

/// Counts the number of distinct labels in frame `t` without background.
///
/// Fails if a voxel isn't an integer or if there are more than `limit` labels.
pub fn count_labels(volume: &Volume, t: usize, limit: usize, progress: &Progress) -> Result<usize> {
    let meta = volume.meta();
    let voxel_count = meta.frame_voxels();
    let mut labels = HashSet::new();
    // neighboring voxels usually share a label, so only look at changes
    let mut previous = 0;
    voxel::for_each_value(volume.frame(t), meta.datatype, meta.endian, |i, raw| {
        if i % 65536 == 0 {
            progress.report(i as f64 / voxel_count as f64, "Preparing labels")?;
        }
        let label = voxel::label_id(meta.scaling.apply(raw))?;
        if label != previous {
            previous = label;
            if label != 0 && labels.insert(label) && labels.len() > limit {
                return Err(Error::invalid(format!(
                    "More than {limit} distinct labels. Are you sure this is a label map?"
                )));
            }
        }
        Ok(())
    })?;
    progress.report(1.0, "Prepared labels")?;
    Ok(labels.len())
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
    use crate::cache::VolumeCache;
    use crate::test_util::Fixture;
    use crate::voxel::DataType;

    fn load(dir: &tempfile::TempDir, fixture: &Fixture) -> Volume {
        let path = dir.path().join("labels.nii");
        fixture.write(&path);
        let cache = VolumeCache::new(dir.path().join("cache"), 1 << 20).unwrap();
        Volume::load(&path, &cache, &Progress::default()).unwrap()
    }

    #[test]
    fn counts_the_labels_of_one_frame() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture {
            datatype: DataType::I32,
            dims: vec![4, 3, 4, 2],
            value: |i| {
                const IDS: [f64; 4] = [0.0, 300.0, 1000.0, 16_777_217.0];
                if i < 48 { IDS[i % 4] } else { IDS[i % 2] }
            },
            ..Fixture::default()
        };
        let volume = load(&dir, &fixture);
        let none = Progress::default();

        assert_eq!(count_labels(&volume, 0, 256, &none).unwrap(), 3);
        assert_eq!(count_labels(&volume, 1, 256, &none).unwrap(), 1);
        assert!(count_labels(&volume, 0, 2, &none).is_err());
    }

    #[test]
    fn rejects_fractional_labels() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture {
            datatype: DataType::F32,
            value: |i| i as f64 + 0.25,
            ..Fixture::default()
        };
        let volume = load(&dir, &fixture);
        assert!(count_labels(&volume, 0, 256, &Progress::default()).is_err());
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
