//! A loaded volume. Its header, and its voxels mapped into memory.

use memmap2::Mmap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::cache::{CacheLease, VolumeCache};
use crate::error::{Error, Result};
use crate::geometry::{GeometryOptions, VolumeGeometry};
use crate::metadata::VolumeMetadata;
use crate::progress::Progress;
use crate::projection::MaximumIntensityProjection;
use crate::voxel;

const MAXIMUM_CACHED_PROJECTIONS: usize = 6;

/// Value range
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValueRange {
    pub(crate) minimum: f64,
    pub(crate) maximum: f64,
}

fn next_id() -> u64 {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// Loaded volume.
/// Can be NIfTI, Analyze, or NRRD.
pub struct Volume {
    metadata: VolumeMetadata,
    path: PathBuf,
    id: u64,
    memory_map: Arc<Mmap>,
    data_path: PathBuf,
    offset: usize,
    projections: Mutex<Vec<Arc<MaximumIntensityProjection>>>,
    /// Keeps a decompressed cache file from being evicted.
    lease: Option<Arc<CacheLease>>,
}

impl Volume {
    pub(crate) fn load(path: &Path, cache: &VolumeCache, progress: &Progress) -> Result<Self> {
        progress.check_cancelled()?;
        let metadata = VolumeMetadata::read(path)?;
        let options = GeometryOptions {
            // Allow unknown units, as the viewer lets the user assume mm.
            assume_millimeters: true,
            ..Default::default()
        };
        VolumeGeometry::new(&metadata, options)?;

        let data = &metadata.data;
        let start = if data.gzip_compressed {
            0
        } else {
            data.stream_offset
        };
        let offset = start
            .checked_add(data.voxel_offset)
            .and_then(|offset| usize::try_from(offset).ok())
            .ok_or_else(|| Error::invalid("Data offset too large"))?;
        let required_length = offset
            .checked_add(data.byte_count)
            .ok_or_else(|| Error::invalid("Volume size overflow!"))?;

        let (data_path, lease) = if data.gzip_compressed {
            let lease = cache.acquire_from(
                &data.path,
                data.stream_offset,
                required_length as u64,
                progress,
            )?;
            (lease.path().to_path_buf(), Some(lease))
        } else {
            (data.path.clone(), None)
        };

        let file = File::open(&data_path)?;
        if !file.metadata()?.is_file() {
            return Err(Error::invalid("Invalid volume data file"));
        }

        // SAFETY: Memory map is only sound while no one else modifies the file.
        // Decompressed volumes are written once into the cache and never changed in place. However,
        // uncompressed volumes aren't stored in our cache, so if another program truncates or rewrites one
        // while it is open, the views may change, or the app may crash. I think the risk is relatively low,
        // and its an alright trade-off to avoid copying whole uncompressed volumes to memory.
        let map = unsafe { Mmap::map(&file)? };
        voxel::validate_region(map.len(), offset, data.byte_count)?;
        progress.check_cancelled()?;

        Ok(Self {
            metadata,
            path: path.to_path_buf(),
            id: next_id(),
            memory_map: Arc::new(map),
            data_path,
            offset,
            projections: Mutex::default(),
            lease,
        })
    }

    pub fn metadata(&self) -> &VolumeMetadata {
        &self.metadata
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn rescaled(&self, factor: f64) -> Self {
        let mut metadata = self.metadata.clone();
        metadata.scaling.slope *= factor;
        metadata.scaling.intercept *= factor;
        Self {
            metadata,
            path: self.path.clone(),
            id: next_id(),
            memory_map: Arc::clone(&self.memory_map),
            data_path: self.data_path.clone(),
            offset: self.offset,
            projections: Mutex::default(),
            lease: self.lease.clone(),
        }
    }

    pub(crate) fn projection(&self, frame: usize, axis: usize) -> Arc<MaximumIntensityProjection> {
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
        let projection = Arc::new(MaximumIntensityProjection::of(self, frame, axis));
        let mut cached = lock();
        cached.retain(|other| (other.frame, other.axis) != (frame, axis));
        cached.push(Arc::clone(&projection));
        let excess = cached.len().saturating_sub(MAXIMUM_CACHED_PROJECTIONS);
        cached.drain(..excess);
        projection
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn geometry(&self, options: GeometryOptions) -> Result<VolumeGeometry> {
        VolumeGeometry::new(&self.metadata, options)
    }

    /// ALL voxel bytes
    pub(crate) fn voxel_bytes(&self) -> &[u8] {
        &self.memory_map[self.offset..self.offset + self.metadata.data.byte_count]
    }

    /// Frames past the last available `frame` show the last available frame
    pub(crate) fn clamp_frame(&self, frame: usize) -> usize {
        frame.min(self.metadata.timepoints() - 1)
    }

    /// Bytes from one 3D frame
    pub(crate) fn frame_bytes(&self, frame: usize) -> &[u8] {
        // Should not be able to overflow?
        let frame_bytes =
            self.metadata.voxels_per_frame() * self.metadata.data_type.bytes_per_voxel();
        let start = self.clamp_frame(frame) * frame_bytes;
        &self.voxel_bytes()[start..start + frame_bytes]
    }

    /// Scaled value for one voxel, None if outside volume
    pub(crate) fn sample_value(&self, [x, y, z]: [usize; 3], frame: usize) -> Option<f64> {
        let metadata = &self.metadata;
        if x >= metadata.width() || y >= metadata.height() || z >= metadata.depth() {
            return None;
        }
        let index = x + metadata.width() * (y + metadata.height() * z);
        let raw = voxel::read_value(
            self.frame_bytes(frame),
            index,
            metadata.data_type,
            metadata.endian,
        )?;
        Some(metadata.scaling.apply(raw))
    }

    pub fn read_ahead(&self, frame: usize) -> std::io::Result<u64> {
        const BLOCK: usize = 8 << 20;
        let frame_bytes = self.frame_bytes(0).len();
        let start = self.offset + self.clamp_frame(frame) * frame_bytes;
        let mut file = File::open(&self.data_path)?;
        file.seek(SeekFrom::Start(start as u64))?;
        let mut buffer = vec![0; BLOCK.min(frame_bytes)];
        let mut remaining = frame_bytes;
        while remaining > 0 {
            let read = file.read(&mut buffer[..BLOCK.min(remaining)])?;
            if read == 0 {
                break;
            }
            remaining -= read;
        }
        Ok((frame_bytes - remaining) as u64)
    }

    pub fn time_curve(&self, voxel: [usize; 3]) -> Option<Vec<f64>> {
        (0..self.metadata.timepoints())
            .map(|frame| self.sample_value(voxel, frame))
            .collect()
    }

    pub(crate) fn frame_value_range(
        &self,
        frame: usize,
        progress: &Progress,
    ) -> Result<Option<ValueRange>> {
        let metadata = &self.metadata;
        let scaling = metadata.scaling;
        let (minimum, maximum) =
            voxel::VoxelBuffer::new(self.frame_bytes(frame), metadata.data_type, metadata.endian)
                .fold(
                progress,
                "Measuring contrast",
                || (f64::INFINITY, f64::NEG_INFINITY),
                |(minimum, maximum), raw| {
                    let value = scaling.apply(raw);
                    if value.is_finite() {
                        (minimum.min(value), maximum.max(value))
                    } else {
                        (minimum, maximum)
                    }
                },
                |(a_minimum, a_maximum), (b_minimum, b_maximum)| {
                    (a_minimum.min(b_minimum), a_maximum.max(b_maximum))
                },
            )?;
        progress.report(1.0, "Measured contrast")?;
        Ok((minimum <= maximum).then_some(ValueRange { minimum, maximum }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{Fixture, write_and_load};
    use crate::voxel::DataType;

    fn range(volume: &Volume, frame: usize) -> Option<ValueRange> {
        volume
            .frame_value_range(frame, &Progress::default())
            .unwrap()
    }

    #[test]
    fn reads_every_type_byte_order_and_compression() {
        let directory = tempfile::tempdir().unwrap();
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
                    let volume = write_and_load(&directory, &fixture, &name);
                    assert_eq!(volume.sample_value([3, 2, 3], 0), Some(47.0), "{name}");
                    assert_eq!(volume.sample_value([4, 0, 0], 0), None, "{name}");
                    assert_eq!(volume.sample_value([0, 0, 4], 0), None, "{name}");
                    assert_eq!(
                        range(&volume, 0),
                        Some(ValueRange {
                            minimum: 0.0,
                            maximum: 47.0
                        }),
                        "{name}"
                    );
                }
            }
        }
    }

    #[test]
    fn applies_scaling() {
        let directory = tempfile::tempdir().unwrap();
        let zero_slope = Fixture {
            slope: 0.0,
            intercept: 100.0,
            ..Fixture::default()
        };
        let volume = write_and_load(&directory, &zero_slope, "zero.nii");
        assert_eq!(volume.sample_value([3, 0, 0], 0), Some(3.0));
        let doubled = volume.rescaled(2.0);
        assert_eq!(doubled.sample_value([3, 0, 0], 0), Some(6.0));
        assert_ne!(doubled.id(), volume.id());

        let negative = Fixture {
            slope: -2.0,
            intercept: 10.0,
            ..Fixture::default()
        };
        let volume = write_and_load(&directory, &negative, "negative.nii");
        assert_eq!(volume.sample_value([3, 0, 0], 0), Some(4.0));
        assert_eq!(
            range(&volume, 0),
            Some(ValueRange {
                minimum: -84.0,
                maximum: 10.0
            })
        );
    }

    #[test]
    fn clamps_frames_of_4d_images() {
        let directory = tempfile::tempdir().unwrap();
        let four_d = Fixture {
            dimensions: vec![4, 3, 4, 2],
            ..Fixture::default()
        };
        let volume = write_and_load(&directory, &four_d, "4d.nii");
        assert_eq!(volume.time_curve([1, 0, 0]), Some(vec![1.0, 49.0]));
        assert_eq!(volume.time_curve([4, 0, 0]), None);
        assert_eq!(volume.read_ahead(1).unwrap(), 4 * 3 * 4 * 2);
        assert_eq!(volume.read_ahead(999).unwrap(), 4 * 3 * 4 * 2);
        assert_eq!(volume.sample_value([0, 0, 0], 0), Some(0.0));
        assert_eq!(volume.sample_value([0, 0, 0], 999), Some(48.0));
        assert_eq!(
            range(&volume, 1),
            Some(ValueRange {
                minimum: 48.0,
                maximum: 95.0
            })
        );
    }

    #[test]
    fn reads_2d_images_and_pairs() {
        let directory = tempfile::tempdir().unwrap();
        let flat = Fixture {
            dimensions: vec![4, 3],
            ..Fixture::default()
        };
        let volume = write_and_load(&directory, &flat, "2d.nii");
        assert_eq!(
            (volume.metadata().depth(), volume.metadata().timepoints()),
            (1, 1)
        );
        assert_eq!(volume.sample_value([3, 2, 0], 0), Some(11.0));

        let pair = write_and_load(&directory, &Fixture::default(), "pair.hdr");
        assert_eq!(pair.sample_value([3, 2, 3], 0), Some(47.0));
    }

    #[test]
    fn keeps_large_integers_exact() {
        let directory = tempfile::tempdir().unwrap();
        let large = Fixture {
            datatype: DataType::U32,
            value: |i| 16_777_217.0 + i as f64,
            ..Fixture::default()
        };
        let volume = write_and_load(&directory, &large, "exact.nii");
        assert_eq!(volume.sample_value([0, 0, 0], 0), Some(16_777_217.0));
    }

    #[test]
    fn range_skips_non_finite_values() {
        let directory = tempfile::tempdir().unwrap();
        let all_nan = Fixture {
            datatype: DataType::F32,
            value: |_| f64::NAN,
            ..Fixture::default()
        };
        let volume = write_and_load(&directory, &all_nan, "nan.nii");
        assert_eq!(range(&volume, 0), None);

        let one_infinity = Fixture {
            datatype: DataType::F32,
            value: |i| if i == 0 { f64::INFINITY } else { i as f64 },
            ..Fixture::default()
        };
        let volume = write_and_load(&directory, &one_infinity, "inf.nii");
        assert_eq!(
            range(&volume, 0),
            Some(ValueRange {
                minimum: 1.0,
                maximum: 47.0
            })
        );
    }

    #[test]
    fn rejects_truncated_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("truncated.nii");
        Fixture::default().write(&path);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(352)
            .unwrap();
        let cache = VolumeCache::new(directory.path().join("cache"), 1 << 20).unwrap();
        assert!(Volume::load(&path, &cache, &Progress::default()).is_err());
    }

    #[test]
    fn every_volume_gets_its_own_id() {
        let directory = tempfile::tempdir().unwrap();
        let a = write_and_load(&directory, &Fixture::default(), "a.nii");
        let b = write_and_load(&directory, &Fixture::default(), "b.nii");
        assert_ne!(a.id(), b.id());
    }
}
