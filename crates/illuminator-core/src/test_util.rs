use std::fs::File;
use std::io::Write;
use std::path::Path;

use flate2::Compression;
use flate2::write::GzEncoder;

use crate::header::Mat3x4;
use crate::voxel::DataType;

pub struct Fixture {
    pub dims: Vec<u16>,
    pub datatype: DataType,
    pub big_endian: bool,
    pub slope: f32,
    pub intercept: f32,
    pub sform: Mat3x4,
    /// The stored (unscaled) value of voxel number `i`.
    pub value: fn(usize) -> f64,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            dims: vec![4, 3, 4],
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
        self.dims.iter().map(|&size| usize::from(size)).product()
    }

    fn header_bytes(&self, pair: bool) -> Vec<u8> {
        let big = self.big_endian;
        let i16_bytes = |value: i16| {
            if big {
                value.to_be_bytes()
            } else {
                value.to_le_bytes()
            }
        };
        let f32_bytes = |value: f32| {
            if big {
                value.to_be_bytes()
            } else {
                value.to_le_bytes()
            }
        };

        let mut bytes = vec![0u8; if pair { 348 } else { 352 }];
        let mut put = |offset: usize, field: &[u8]| {
            bytes[offset..offset + field.len()].copy_from_slice(field);
        };
        let size = if big {
            348i32.to_be_bytes()
        } else {
            348i32.to_le_bytes()
        };
        put(0, &size);
        let mut dim = [1i16; 8];
        dim[0] = self.dims.len() as i16;
        for (axis, &size) in self.dims.iter().enumerate() {
            dim[axis + 1] = size as i16;
        }
        for (i, &value) in dim.iter().enumerate() {
            put(40 + 2 * i, &i16_bytes(value));
        }
        put(70, &i16_bytes(datatype_code(self.datatype)));
        put(72, &i16_bytes(8 * self.datatype.size() as i16));
        for i in 0..8 {
            put(76 + 4 * i, &f32_bytes(1.0));
        }
        put(108, &f32_bytes(if pair { 0.0 } else { 352.0 }));
        put(112, &f32_bytes(self.slope));
        put(116, &f32_bytes(self.intercept));
        put(123, &[2]); // millimeters
        put(254, &i16_bytes(1)); // sform_code
        for (row, values) in self.sform.iter().enumerate() {
            for (col, &value) in values.iter().enumerate() {
                put(280 + 16 * row + 4 * col, &f32_bytes(value as f32));
            }
        }
        put(344, if pair { b"ni1\0" } else { b"n+1\0" });
        bytes
    }

    fn voxel_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        for i in 0..self.voxel_count() {
            let value = (self.value)(i);
            macro_rules! push {
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
                DataType::U8 => push!(u8),
                DataType::I8 => push!(i8),
                DataType::U16 => push!(u16),
                DataType::I16 => push!(i16),
                DataType::U32 => push!(u32),
                DataType::I32 => push!(i32),
                DataType::F32 => push!(f32),
                DataType::F64 => push!(f64),
            }
        }
        bytes
    }
}

fn datatype_code(datatype: DataType) -> i16 {
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
