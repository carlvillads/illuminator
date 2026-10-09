use std::collections::HashMap;

use crate::error::{Error, Result};
use crate::progress::Progress;
use crate::volume::Volume;
use crate::voxel;

/// Label IDs to pallette indices
#[derive(Debug, Clone)]
pub struct LabelTable {
    indices: HashMap<i64, u32>,
    labels: Vec<i64>,
}

impl Default for LabelTable {
    fn default() -> Self {
        Self {
            indices: HashMap::from([(0, 0)]),
            labels: vec![0],
        }
    }
}

impl LabelTable {
    /// Label ID by palette index
    pub fn labels(&self) -> &[i64] {
        &self.labels
    }

    /// Converts frame t into palette index per voxel
    pub fn encode(
        &mut self,
        volume: &Volume,
        t: usize,
        limit: usize,
        progress: &Progress,
    ) -> Result<Vec<u32>> {
        let h = volume.meta();
        let count = h.frame_voxels();
        let mut indices = Vec::with_capacity(count);
        // Skip hash map if the voxel label repeats
        let mut last = (0, 0);
        voxel::for_each_value(volume.frame(t), h.datatype, h.endian, |i, raw| {
            if i % 65_536 == 0 {
                progress.report(i as f64 / count as f64, "Preparing labels")?;
            }
            let label = voxel::label_id(h.scaling.apply(raw))?;
            if label != last.0 {
                let index = match self.indices.get(&label).copied() {
                    Some(index) => index,
                    None => self.add(label, limit)?,
                };
                last = (label, index);
            }
            indices.push(last.1);
            Ok(())
        })?;
        progress.report(1.0, "Prepared labels")?;
        Ok(indices)
    }

    fn add(&mut self, label: i64, limit: usize) -> Result<u32> {
        let too_many = || Error::invalid("Too many distinct segmentation labels for the palette");
        if self.labels.len() >= limit {
            return Err(too_many());
        }
        let index = u32::try_from(self.labels.len()).map_err(|_| too_many())?;
        self.labels.push(label);
        self.indices.insert(label, index);
        Ok(index)
    }
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
    fn indices_stay_stable_across_frames() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Fixture {
            datatype: DataType::I32,
            dims: vec![4, 3, 4, 2],
            value: |i| {
                const IDS: [f64; 4] = [0.0, 300.0, 1000.0, 16_777_217.0];
                IDS[(i + usize::from(i >= 48)) % 4]
            },
            ..Fixture::default()
        };
        let volume = load(&dir, &fixture);
        let none = Progress::default();

        let mut table = LabelTable::default();
        let first = table.encode(&volume, 0, 256, &none).unwrap();
        let second = table.encode(&volume, 1, 256, &none).unwrap();
        let again = table.encode(&volume, 0, 256, &none).unwrap();
        assert_eq!(first, again);
        assert_eq!(first[1], second[0]);
        assert!(first[1] != first[2] && first[2] != first[3]);
        assert_eq!(table.labels(), [0, 300, 1000, 16_777_217]);
        assert_eq!(table.labels()[first[3] as usize], 16_777_217);

        let mut small = LabelTable::default();
        assert!(small.encode(&volume, 0, 2, &none).is_err());
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
        let result = LabelTable::default().encode(&volume, 0, 256, &Progress::default());
        assert!(result.is_err());
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
