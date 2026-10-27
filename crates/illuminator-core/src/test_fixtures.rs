//! Test helpers.
//!
//! Writing small NIfTI files and loading them.

use std::fs::File;
use std::io::Write;
use std::path::Path;

use flate2::Compression;
use flate2::write::GzEncoder;
use tempfile::TempDir;

use crate::cache::VolumeCache;
use crate::metadata::Matrix3x4;
use crate::progress::Progress;
use crate::volume::Volume;
use crate::voxel::DataType;

pub fn write_and_load(directory: &TempDir, fixture: &Fixture, name: &str) -> Volume {
    let path = directory.path().join(name);
    fixture.write(&path);
    let cache = VolumeCache::new(directory.path().join("cache"), 1 << 20).unwrap();
    Volume::load(&path, &cache, &Progress::default())
        .unwrap_or_else(|error| panic!("{name}: {error}"))
}

pub struct Fixture {
    pub dimensions: Vec<u16>,
    pub datatype: DataType,
    pub big_endian: bool,
    pub slope: f32,
    pub intercept: f32,
    pub sform: Matrix3x4,
    /// The stored (unscaled) value of voxel number `i`.
    pub value: fn(usize) -> f64,
    pub analyze_format: bool,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            dimensions: vec![4, 3, 4],
            datatype: DataType::I16,
            big_endian: false,
            slope: 1.0,
            intercept: 0.0,
            sform: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
            value: |i| i as f64,
            analyze_format: false,
        }
    }
}

impl Fixture {
    pub fn write(&self, path: &Path) {
        let name = path.to_string_lossy();
        let pair = name.ends_with(".hdr");
        let header = self.header_bytes(pair);
        let voxels = self.voxel_bytes();
        if pair {
            std::fs::write(path, &header).unwrap();
            std::fs::write(path.with_extension("img"), &voxels).unwrap();
            return;
        }
        let bytes = [header, voxels].concat();
        if name.ends_with(".gz") {
            let mut encoder = GzEncoder::new(File::create(path).unwrap(), Compression::fast());
            encoder.write_all(&bytes).unwrap();
            encoder.finish().unwrap();
        } else {
            std::fs::write(path, bytes).unwrap();
        }
    }

    fn voxel_count(&self) -> usize {
        self.dimensions
            .iter()
            .map(|&size| usize::from(size))
            .product()
    }

    fn header_bytes(&self, pair: bool) -> Vec<u8> {
        let big_endian = self.big_endian;
        let i16_bytes = |value: i16| {
            if big_endian {
                value.to_be_bytes()
            } else {
                value.to_le_bytes()
            }
        };
        let f32_bytes = |value: f32| {
            if big_endian {
                value.to_be_bytes()
            } else {
                value.to_le_bytes()
            }
        };

        let mut bytes = vec![0u8; if pair { 348 } else { 352 }];
        let mut write_field = |offset: usize, field: &[u8]| {
            bytes[offset..offset + field.len()].copy_from_slice(field);
        };
        let size = if big_endian {
            348i32.to_be_bytes()
        } else {
            348i32.to_le_bytes()
        };
        write_field(0, &size);
        let mut dim = [1i16; 8];
        dim[0] = self.dimensions.len() as i16;
        for (axis, &size) in self.dimensions.iter().enumerate() {
            dim[axis + 1] = size as i16;
        }
        for (i, &value) in dim.iter().enumerate() {
            write_field(40 + 2 * i, &i16_bytes(value));
        }
        write_field(70, &i16_bytes(nifti_datatype_code(self.datatype)));
        write_field(72, &i16_bytes(8 * self.datatype.bytes_per_voxel() as i16));
        for i in 0..8 {
            write_field(76 + 4 * i, &f32_bytes(1.0));
        }
        write_field(108, &f32_bytes(if pair { 0.0 } else { 352.0 }));
        write_field(112, &f32_bytes(self.slope));
        write_field(116, &f32_bytes(self.intercept));
        write_field(123, &[2]); // millimeters
        write_field(254, &i16_bytes(1)); // sform_code
        for (row, values) in self.sform.iter().enumerate() {
            for (col, &value) in values.iter().enumerate() {
                write_field(280 + 16 * row + 4 * col, &f32_bytes(value as f32));
            }
        }
        let magic = match (self.analyze_format, pair) {
            (true, _) => &[0; 4],
            (false, true) => b"ni1\0",
            (false, false) => b"n+1\0",
        };
        write_field(344, magic);
        bytes
    }

    fn voxel_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        for i in 0..self.voxel_count() {
            let value = (self.value)(i);
            macro_rules! append_value {
                ($t:ty) => {{
                    let value = value as $t;
                    let raw = if self.big_endian {
                        value.to_be_bytes()
                    } else {
                        value.to_le_bytes()
                    };
                    bytes.extend_from_slice(&raw);
                }};
            }
            match self.datatype {
                DataType::U8 => append_value!(u8),
                DataType::I8 => append_value!(i8),
                DataType::U16 => append_value!(u16),
                DataType::I16 => append_value!(i16),
                DataType::U32 => append_value!(u32),
                DataType::I32 => append_value!(i32),
                DataType::F32 => append_value!(f32),
                DataType::F64 => append_value!(f64),
            }
        }
        bytes
    }
}

fn nifti_datatype_code(datatype: DataType) -> i16 {
    match datatype {
        DataType::U8 => 2,
        DataType::I16 => 4,
        DataType::I32 => 8,
        DataType::F32 => 16,
        DataType::F64 => 64,
        DataType::I8 => 256,
        DataType::U16 => 512,
        DataType::U32 => 768,
    }
}
