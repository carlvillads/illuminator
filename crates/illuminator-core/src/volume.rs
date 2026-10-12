use memmap2::Mmap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::cache::{CacheLease, VolumeCache};
use crate::error::{Error, Result};
use crate::geometry::{GeometryOptions, VolumeGeometry};
use crate::meta::VolumeMeta;
use crate::progress::Progress;
use crate::projection::Projection;
use crate::voxel;

const NUM_MIP: usize = 6;

/// Value range
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    pub min: f64,
    pub max: f64,
}

fn next_id() -> u64 {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// Loaded nifti volume
pub struct Volume {
    meta: VolumeMeta,
    path: PathBuf,
    id: u64,
    map: Arc<Mmap>,
    data_path: PathBuf,
    offset: usize,
    projections: Mutex<Vec<Arc<Projection>>>,
    /// Keeps a decompressed cache file from being evicted.
    _lease: Option<Arc<CacheLease>>,
}

impl Volume {
    pub fn load(path: &Path, cache: &VolumeCache, progress: &Progress) -> Result<Self> {
        progress.check()?;
        let meta = VolumeMeta::read(path)?;
        let assume_mm = GeometryOptions {
            // Allow unknown units, as the viewer lets the user assume mm.
            assume_mm: true,
            ..Default::default()
        };
        VolumeGeometry::new(&meta, assume_mm)?;

        let data = &meta.data;
        let start = if data.gzip { 0 } else { data.start };
        let offset = start
            .checked_add(data.offset)
            .and_then(|offset| usize::try_from(offset).ok())
            .ok_or_else(|| Error::invalid("Data offset too large"))?;
        let needed = offset
            .checked_add(data.bytes)
            .ok_or_else(|| Error::invalid("Volume size overflow!"))?;

        let (data_path, lease) = if data.gzip {
            let lease = cache.acquire_from(&data.path, data.start, needed as u64, progress)?;
            (lease.path().to_path_buf(), Some(lease))
        } else {
            (data.path.clone(), None)
        };

        let file = File::open(&data_path)?;
        let info = file.metadata()?;
        if !info.is_file() {
            return Err(Error::invalid("Invalid volume data file"));
        }
        let file_bytes = usize::try_from(info.len())
            .map_err(|_| Error::invalid("Volume data file is too large"))?;
        voxel::validate_region(file_bytes, offset, data.bytes)?;

        // NOTE: I haven't done anything about what happens if someone changes a uncompressed file while open.

        let map = unsafe { Mmap::map(&file)? };
        if map.len() < needed {
            return Err(Error::invalid(
                "Nifti data is truncated or has an invalid offset",
            ));
        }
        progress.check()?;

        Ok(Self {
            meta,
            path: path.to_path_buf(),
            id: next_id(),
            map: Arc::new(map),
            data_path,
            offset,
            projections: Mutex::default(),
            _lease: lease,
        })
    }

    pub fn meta(&self) -> &VolumeMeta {
        &self.meta
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn rescaled(&self, factor: f64) -> Self {
        let mut meta = self.meta.clone();
        meta.scaling.slope *= factor;
        meta.scaling.intercept *= factor;
        Self {
            meta,
            path: self.path.clone(),
            id: next_id(),
            map: Arc::clone(&self.map),
            data_path: self.data_path.clone(),
            offset: self.offset,
            projections: Mutex::default(),
            _lease: self._lease.clone(),
        }
    }

    pub fn projection(&self, frame: usize, axis: usize) -> Arc<Projection> {
        let frame = self.clamp_frame(frame);
        let lock = || {
            self.projections
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        };
        let found = lock()
            .iter()
            .find(|projection| (projection.frame, projection.axis) == (frame, axis))
            .cloned();
        if let Some(projection) = found {
            return projection;
        }

        // compute new projection without the lock to keep the others available meanwhile
        let projection = Arc::new(Projection::of(self, frame, axis));
        let mut kept = lock();
        kept.retain(|other| (other.frame, other.axis) != (frame, axis));
        kept.push(Arc::clone(&projection));
        let excess = kept.len().saturating_sub(NUM_MIP);
        kept.drain(..excess);
        projection
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn geometry(&self, options: GeometryOptions) -> Result<VolumeGeometry> {
        VolumeGeometry::new(&self.meta, options)
    }

    /// ALL voxel bytes
    pub fn data(&self) -> &[u8] {
        &self.map[self.offset..self.offset + self.meta.data.bytes]
    }

    /// Frames > available t just show the last available frame
    pub fn clamp_frame(&self, t: usize) -> usize {
        t.min(self.meta.timepoints() - 1)
    }

    /// Bytes from one 3D frame
    pub fn frame(&self, t: usize) -> &[u8] {
        // Should not be able to overflow?
        let frame_bytes = self.meta.frame_voxels() * self.meta.datatype.size();
        let start = self.clamp_frame(t) * frame_bytes;
        &self.data()[start..start + frame_bytes]
    }

    /// Scaled value for one voxel, None if outside volume
    pub fn sample_value(&self, [x, y, z]: [usize; 3], t: usize) -> Option<f64> {
        let h = &self.meta;
        if x >= h.width() || y >= h.height() || z >= h.depth() {
            return None;
        }
        let index = x + h.width() * (y + h.height() * z);
        let raw = voxel::read_value(self.frame(t), index, h.datatype, h.endian)?;
        Some(h.scaling.apply(raw))
    }

    pub fn read_ahead(&self, t: usize) -> std::io::Result<u64> {
        const BLOCK: usize = 8 << 20;
        let frame_bytes = self.frame(0).len();
        let start = self.offset + self.clamp_frame(t) * frame_bytes;
        let mut file = File::open(&self.data_path)?;
        file.seek(SeekFrom::Start(start as u64))?;
        let mut buffer = vec![0; BLOCK.min(frame_bytes)];
        let mut left = frame_bytes;
        while left > 0 {
            let read = file.read(&mut buffer[..BLOCK.min(left)])?;
            if read == 0 {
                break;
            }
            left -= read;
        }
        Ok((frame_bytes - left) as u64)
    }

    pub fn time_curve(&self, voxel: [usize; 3]) -> Option<Vec<f64>> {
        (0..self.meta.timepoints())
            .map(|t| self.sample_value(voxel, t))
            .collect()
    }

    pub fn frame_range(&self, t: usize, progress: &Progress) -> Result<Option<Range>> {
        let h = &self.meta;
        let scaling = h.scaling;
        let (min, max) = voxel::Values::new(self.frame(t), h.datatype, h.endian).fold(
            progress,
            "Measuring contrast",
            || (f64::INFINITY, f64::NEG_INFINITY),
            |(min, max), raw| {
                let value = scaling.apply(raw);
                if value.is_finite() {
                    (min.min(value), max.max(value))
                } else {
                    (min, max)
                }
            },
            |(a_min, a_max), (b_min, b_max)| (a_min.min(b_min), a_max.max(b_max)),
        )?;
        progress.report(1.0, "Measured contrast")?;
        Ok((min <= max).then_some(Range { min, max }))
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::test_util::Fixture;
    use crate::voxel::DataType;

    fn load(dir: &TempDir, fixture: &Fixture, name: &str) -> Result<Volume> {
        let path = dir.path().join(name);
        fixture.write(&path);
        let cache = VolumeCache::new(dir.path().join("cache"), 1 << 20)?;
        Volume::load(&path, &cache, &Progress::default())
    }

    fn range(volume: &Volume, t: usize) -> Option<Range> {
        volume.frame_range(t, &Progress::default()).unwrap()
    }

    #[test]
    fn reads_every_type_byte_order_and_compression() {
        let dir = tempfile::tempdir().unwrap();
        let types = [
            DataType::U8,
            DataType::I8,
            DataType::U16,
            DataType::I16,
            DataType::U32,
            DataType::I32,
            DataType::F32,
            DataType::F64,
        ];
        for datatype in types {
            for big_endian in [false, true] {
                for extension in ["nii", "nii.gz"] {
                    let fixture = Fixture {
                        datatype,
                        big_endian,
                        ..Fixture::default()
                    };
                    let name = format!("{datatype:?}-{big_endian}.{extension}");
                    let volume = load(&dir, &fixture, &name).expect(&name);
                    assert_eq!(volume.sample_value([3, 2, 3], 0), Some(47.0), "{name}");
                    assert_eq!(volume.sample_value([4, 0, 0], 0), None, "{name}");
                    assert_eq!(volume.sample_value([0, 0, 4], 0), None, "{name}");
                    assert_eq!(
                        range(&volume, 0),
                        Some(Range {
                            min: 0.0,
                            max: 47.0
                        }),
                        "{name}"
                    );
                }
            }
        }
    }

    #[test]
    fn applies_scaling() {
        let dir = tempfile::tempdir().unwrap();
        let zero_slope = Fixture {
            slope: 0.0,
            intercept: 100.0,
            ..Fixture::default()
        };
        let volume = load(&dir, &zero_slope, "zero.nii").unwrap();
        assert_eq!(volume.sample_value([3, 0, 0], 0), Some(3.0));
        let doubled = volume.rescaled(2.0);
        assert_eq!(doubled.sample_value([3, 0, 0], 0), Some(6.0));
        assert_ne!(doubled.id(), volume.id());

        let negative = Fixture {
            slope: -2.0,
            intercept: 10.0,
            ..Fixture::default()
        };
        let volume = load(&dir, &negative, "negative.nii").unwrap();
        assert_eq!(volume.sample_value([3, 0, 0], 0), Some(4.0));
        assert_eq!(
            range(&volume, 0),
            Some(Range {
                min: -84.0,
                max: 10.0
            })
        );
    }

    #[test]
    fn clamps_frames_of_4d_images() {
        let dir = tempfile::tempdir().unwrap();
        let four_d = Fixture {
            dims: vec![4, 3, 4, 2],
            ..Fixture::default()
        };
        let volume = load(&dir, &four_d, "4d.nii").unwrap();
        assert_eq!(volume.time_curve([1, 0, 0]), Some(vec![1.0, 49.0]));
        assert_eq!(volume.time_curve([4, 0, 0]), None);
        assert_eq!(volume.read_ahead(1).unwrap(), 4 * 3 * 4 * 2);
        assert_eq!(volume.read_ahead(999).unwrap(), 4 * 3 * 4 * 2);
        assert_eq!(volume.sample_value([0, 0, 0], 0), Some(0.0));
        assert_eq!(volume.sample_value([0, 0, 0], 999), Some(48.0));
        assert_eq!(
            range(&volume, 1),
            Some(Range {
                min: 48.0,
                max: 95.0
            })
        );
    }

    #[test]
    fn reads_2d_images_and_pairs() {
        let dir = tempfile::tempdir().unwrap();
        let flat = Fixture {
            dims: vec![4, 3],
            ..Fixture::default()
        };
        let volume = load(&dir, &flat, "2d.nii").unwrap();
        assert_eq!((volume.meta().depth(), volume.meta().timepoints()), (1, 1));
        assert_eq!(volume.sample_value([3, 2, 0], 0), Some(11.0));

        let pair = load(&dir, &Fixture::default(), "pair.hdr").unwrap();
        assert_eq!(pair.sample_value([3, 2, 3], 0), Some(47.0));
    }

    #[test]
    fn keeps_large_integers_exact() {
        let dir = tempfile::tempdir().unwrap();
        let large = Fixture {
            datatype: DataType::U32,
            value: |i| 16_777_217.0 + i as f64,
            ..Fixture::default()
        };
        let volume = load(&dir, &large, "exact.nii").unwrap();
        assert_eq!(volume.sample_value([0, 0, 0], 0), Some(16_777_217.0));
    }

    #[test]
    fn range_skips_non_finite_values() {
        let dir = tempfile::tempdir().unwrap();
        let all_nan = Fixture {
            datatype: DataType::F32,
            value: |_| f64::NAN,
            ..Fixture::default()
        };
        let volume = load(&dir, &all_nan, "nan.nii").unwrap();
        assert_eq!(range(&volume, 0), None);

        let one_infinity = Fixture {
            datatype: DataType::F32,
            value: |i| if i == 0 { f64::INFINITY } else { i as f64 },
            ..Fixture::default()
        };
        let volume = load(&dir, &one_infinity, "inf.nii").unwrap();
        assert_eq!(
            range(&volume, 0),
            Some(Range {
                min: 1.0,
                max: 47.0
            })
        );
    }

    #[test]
    fn rejects_truncated_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("truncated.nii");
        Fixture::default().write(&path);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(352)
            .unwrap();
        let cache = VolumeCache::new(dir.path().join("cache"), 1 << 20).unwrap();
        assert!(Volume::load(&path, &cache, &Progress::default()).is_err());
    }

    #[test]
    fn every_volume_gets_its_own_id() {
        let dir = tempfile::tempdir().unwrap();
        let a = load(&dir, &Fixture::default(), "a.nii").unwrap();
        let b = load(&dir, &Fixture::default(), "b.nii").unwrap();
        assert_ne!(a.id(), b.id());
    }
}
