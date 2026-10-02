use std::collections::HashMap;

use crate::error::{Error, Result};
use crate::progress::{self, Progress};
use crate::volume::{self, NiiVolume};
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
        volume: &NiiVolume,
        t: usize,
        limit: usize,
        progress: &Progress,
    ) -> Result<Vec<u32>> {
        let h = volume.header();
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

    fn add(&mut self, label: i64, limit: usize) -> Result<u32> {}
}
