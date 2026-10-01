//! Voxel datatypes, intensity scaling, raw reading.

use crate::error::{Error, Result};

// Scalar voxeltype support
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    F32,
    F64,
}

impl DataType {
    pub fn from_code(code: i16) -> Result<Self> {
        match code {
            2 => Ok(Self::U8),
            4 => Ok(Self::I16),
            8 => Ok(Self::I32),
            16 => Ok(Self::F32),
            64 => Ok(Self::F64),
            256 => Ok(Self::I8),
            512 => Ok(Self::U16),
            768 => Ok(Self::U32),
            _ => Err(Error::invalid(
                "Unsupported datatype: expected 8/16/32-bit scalar integer, or 32- or 64-bit float",
            )),
        }
    }

    /// Bytes per voxel
    pub fn size(self) -> usize {
        match self {
            Self::U8 | Self::I8 => 1,
            Self::U16 | Self::I16 => 2,
            Self::U32 | Self::I32 | Self::F32 => 4,
            Self::F64 => 8,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::U8 => "UINT8",
            Self::I8 => "INT8",
            Self::U16 => "UINT16",
            Self::I16 => "INT16",
            Self::U32 => "UINT32",
            Self::I32 => "INT32",
            Self::F32 => "FLOAT32",
            Self::F64 => "FLOAT64",
        }
    }
}

/// Voxel data byteorder on disk
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endian {
    Little,
    Big,
}

/// Intensity scaling = raw * slope + intercept
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scaling {
    pub slope: f64,
    pub intercept: f64,
}

impl Default for Scaling {
    fn default() -> Self {
        Self {
            slope: 1.0,
            intercept: 0.0,
        }
    }
}

impl Scaling {
    pub fn from_header(slope: f32, intercept: f32) -> Result<Self> {
        if slope == 0.0 {
            return Ok(Self::default());
        }
        if !slope.is_finite() || !intercept.is_finite() {
            return Err(Error::invalid("Invalid nifti intensity scaling"));
        }
        Ok(Self {
            slope: slope as f64,
            intercept: intercept as f64,
        })
    }

    pub fn apply(self, raw: f64) -> f64 {
        raw * self.slope + self.intercept
    }
}

pub fn checked_mul(a: usize, b: usize) -> Result<usize> {
    a.checked_mul(b)
        .ok_or_else(|| Error::invalid("Nifti size overflow!"))
}

/// Check that 'data_bytes' starting at 'offset' fit in the file
pub fn validate_region(file_bytes: usize, offset: usize, data_bytes: usize) -> Result<()> {
    if offset > file_bytes || data_bytes > file_bytes - offset {
        return Err(Error::invalid(
            "Nifti has an invalid offset or data is truncated",
        ));
    }
    Ok(())
}

pub fn label_id(value: f64) -> Result<i64> {
    // avoid that i64::MAX rounds up
    const POWER_63: f64 = 9223372036854775808_f64; // 2^63
    if !value.is_finite() || value != value.trunc() || value < -POWER_63 || value >= POWER_63 {
        return Err(Error::invalid(
            "Segmentation labels must be finite and integral 64-bit IDs",
        ));
    }
    Ok(value as i64)
}

/// Scalar voxel type that can be decoded from raw bytes
trait Scalar {
    const SIZE: usize;
    fn read(bytes: &[u8], endian: Endian) -> f64;
}

macro_rules! impl_scalar {
    ($t:ty) => {
        impl Scalar for $t {
            const SIZE: usize = std::mem::size_of::<$t>();
            fn read(bytes: &[u8], endian: Endian) -> f64 {
                let array: [u8; std::mem::size_of::<$t>()] =
                    bytes.try_into().expect("caller passes exactly SIZE bytes");
                let value = match endian {
                    Endian::Big => <$t>::from_be_bytes(array),
                    Endian::Little => <$t>::from_le_bytes(array),
                };
                value as f64
            }
        }
    };
}

impl_scalar!(u8);
impl_scalar!(i8);
impl_scalar!(u16);
impl_scalar!(i16);
impl_scalar!(u32);
impl_scalar!(i32);
impl_scalar!(f32);
impl_scalar!(f64);

/// Read voxel index af unscaled float64, returns none if outside of bytes
pub fn read_value(bytes: &[u8], index: usize, data_type: DataType, endian: Endian) -> Option<f64> {
    let size = data_type.size();
    let start = index.checked_mul(size)?;
    let chunk = bytes.get(start..start.checked_add(size)?)?;
    Some(match data_type {
        DataType::U8 => u8::read(chunk, endian),
        DataType::I8 => i8::read(chunk, endian),
        DataType::I16 => i16::read(chunk, endian),
        DataType::U16 => u16::read(chunk, endian),
        DataType::U32 => u32::read(chunk, endian),
        DataType::I32 => i32::read(chunk, endian),
        DataType::F32 => f32::read(chunk, endian),
        DataType::F64 => f64::read(chunk, endian),
    })
}

/// Calls `f(idx, val)` for every voxel in `bytes`, stopping at first error.
pub fn for_each_value<F>(bytes: &[u8], data_type: DataType, endian: Endian, mut f: F) -> Result<()>
where
    F: FnMut(usize, f64) -> Result<()>,
{
    match data_type {
        DataType::U8 => for_each_typed::<u8, _>(bytes, endian, &mut f),
        DataType::I8 => for_each_typed::<i8, _>(bytes, endian, &mut f),
        DataType::U16 => for_each_typed::<u16, _>(bytes, endian, &mut f),
        DataType::I16 => for_each_typed::<i16, _>(bytes, endian, &mut f),
        DataType::U32 => for_each_typed::<u32, _>(bytes, endian, &mut f),
        DataType::I32 => for_each_typed::<i32, _>(bytes, endian, &mut f),
        DataType::F32 => for_each_typed::<f32, _>(bytes, endian, &mut f),
        DataType::F64 => for_each_typed::<f64, _>(bytes, endian, &mut f),
    }
}

fn for_each_typed<T: Scalar, F>(bytes: &[u8], endian: Endian, f: &mut F) -> Result<()>
where
    F: FnMut(usize, f64) -> Result<()>,
{
    for (index, chunk) in bytes.chunks_exact(T::SIZE).enumerate() {
        f(index, T::read(chunk, endian))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datatype_codes() {
        assert_eq!(DataType::from_code(4).unwrap(), DataType::I16);
        assert_eq!(DataType::from_code(16).unwrap().size(), 4);
        assert!(DataType::from_code(128).is_err());
    }

    #[test]
    fn reads_both_byte_orders() {
        let little = 0x1234u16.to_le_bytes();
        let big = 0x1234u16.to_be_bytes();
        assert_eq!(
            read_value(&little, 0, DataType::U16, Endian::Little),
            Some(4660.0)
        );
        assert_eq!(
            read_value(&big, 0, DataType::U16, Endian::Big),
            Some(4660.0)
        );
    }

    #[test]
    fn reads_unaligned_and_out_of_range() {
        let mut bytes = vec![0u8];
        bytes.extend_from_slice(&1.5f32.to_le_bytes());
        assert_eq!(
            read_value(&bytes[1..], 0, DataType::F32, Endian::Little),
            Some(1.5)
        );
        assert_eq!(
            read_value(&bytes[1..], 1, DataType::F32, Endian::Little),
            None
        );
    }

    #[test]
    fn visits_every_value() {
        let bytes: Vec<u8> = [-2i16, 0, 7].iter().flat_map(|v| v.to_be_bytes()).collect();
        let mut seen = Vec::new();
        for_each_value(&bytes, DataType::I16, Endian::Big, |index, value| {
            seen.push((index, value));
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, [(0, -2.0), (1, 0.0), (2, 7.0)]);
    }

    #[test]
    fn stops_on_first_error() {
        let bytes = [1u8, 2, 3];
        let mut visited = 0;
        let result = for_each_value(&bytes, DataType::U8, Endian::Little, |_, _| {
            visited += 1;
            Err(Error::Cancelled)
        });
        assert!(matches!(result, Err(Error::Cancelled)));
        assert_eq!(visited, 1);
    }

    #[test]
    fn scaling() {
        assert_eq!(Scaling::from_header(0.0, 5.0).unwrap(), Scaling::default());
        assert_eq!(Scaling::from_header(2.0, 1.0).unwrap().apply(3.0), 7.0);
        assert!(Scaling::from_header(f32::NAN, 0.0).is_err());
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
    fn size_checks() {
        assert!(checked_mul(usize::MAX, 2).is_err());
        assert!(validate_region(100, 96, 4).is_ok());
        assert!(validate_region(100, 97, 4).is_err());
        assert!(validate_region(100, 101, 0).is_err());
    }
}
