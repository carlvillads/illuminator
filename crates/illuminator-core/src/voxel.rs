//! Voxel datatypes, intensity scaling, raw reading.

use rayon::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::{Error, Result};
use crate::progress::Progress;

// Scalar voxel types that are supported
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
    pub(crate) fn bytes_per_voxel(self) -> usize {
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
pub(crate) enum ByteOrder {
    LittleEndian,
    BigEndian,
}

/// Intensity scaling = raw * slope + intercept
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Scaling {
    pub(crate) slope: f64,
    pub(crate) intercept: f64,
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
    pub(crate) fn apply(self, raw: f64) -> f64 {
        raw * self.slope + self.intercept
    }
}

pub(crate) fn checked_multiply(a: usize, b: usize) -> Result<usize> {
    a.checked_mul(b)
        .ok_or_else(|| Error::invalid("Volume size overflow!"))
}

/// Check that 'data_bytes' starting at 'offset' fit in the file
pub(crate) fn validate_region(file_bytes: usize, offset: usize, data_bytes: usize) -> Result<()> {
    if offset > file_bytes || data_bytes > file_bytes - offset {
        return Err(Error::invalid(
            "The data is truncated or its offset is invalid",
        ));
    }
    Ok(())
}

/// Scalar voxel type that can be decoded from raw bytes
pub(crate) trait Scalar: Sync {
    const SIZE: usize;
    fn read(bytes: &[u8], byte_order: ByteOrder) -> f64;
}

macro_rules! implement_scalar {
    ($t:ty) => {
        impl Scalar for $t {
            const SIZE: usize = std::mem::size_of::<$t>();
            fn read(bytes: &[u8], byte_order: ByteOrder) -> f64 {
                let array: [u8; std::mem::size_of::<$t>()] =
                    bytes.try_into().expect("caller passes exactly SIZE bytes");
                let value = match byte_order {
                    ByteOrder::BigEndian => <$t>::from_be_bytes(array),
                    ByteOrder::LittleEndian => <$t>::from_le_bytes(array),
                };
                value as f64
            }
        }
    };
}

implement_scalar!(u8);
implement_scalar!(i8);
implement_scalar!(u16);
implement_scalar!(i16);
implement_scalar!(u32);
implement_scalar!(i32);
implement_scalar!(f32);
implement_scalar!(f64);

macro_rules! dispatch_data_type {
    ($datatype:expr, |$t:ident| $body:expr) => {
        match $datatype {
            $crate::voxel::DataType::U8 => {
                type $t = u8;
                $body
            }
            $crate::voxel::DataType::I8 => {
                type $t = i8;
                $body
            }
            $crate::voxel::DataType::U16 => {
                type $t = u16;
                $body
            }
            $crate::voxel::DataType::I16 => {
                type $t = i16;
                $body
            }
            $crate::voxel::DataType::U32 => {
                type $t = u32;
                $body
            }
            $crate::voxel::DataType::I32 => {
                type $t = i32;
                $body
            }
            $crate::voxel::DataType::F32 => {
                type $t = f32;
                $body
            }
            $crate::voxel::DataType::F64 => {
                type $t = f64;
                $body
            }
        }
    };
}
pub(crate) use dispatch_data_type;

/// Read voxel index af unscaled float64, returns none if outside of bytes
pub(crate) fn read_value(
    bytes: &[u8],
    index: usize,
    data_type: DataType,
    byte_order: ByteOrder,
) -> Option<f64> {
    let size = data_type.bytes_per_voxel();
    let start = index.checked_mul(size)?;
    let chunk = bytes.get(start..start.checked_add(size)?)?;
    Some(dispatch_data_type!(data_type, |T| T::read(
        chunk, byte_order
    )))
}

/// Calls `f(idx, val)` for every voxel in `bytes`, stopping at first error.
pub(crate) fn for_each_value<F>(
    bytes: &[u8],
    data_type: DataType,
    byte_order: ByteOrder,
    mut f: F,
) -> Result<()>
where
    F: FnMut(usize, f64) -> Result<()>,
{
    dispatch_data_type!(data_type, |T| for_each_typed::<T, _>(
        bytes, byte_order, &mut f
    ))
}

fn for_each_typed<T: Scalar, F>(bytes: &[u8], byte_order: ByteOrder, f: &mut F) -> Result<()>
where
    F: FnMut(usize, f64) -> Result<()>,
{
    for (index, chunk) in bytes.chunks_exact(T::SIZE).enumerate() {
        f(index, T::read(chunk, byte_order))?;
    }
    Ok(())
}

const BLOCK_LENGTH: usize = 1 << 16;

pub(crate) struct VoxelBuffer<'a> {
    bytes: &'a [u8],
    data_type: DataType,
    byte_order: ByteOrder,
}

impl<'a> VoxelBuffer<'a> {
    pub(crate) fn new(bytes: &'a [u8], data_type: DataType, byte_order: ByteOrder) -> Self {
        Self {
            bytes,
            data_type,
            byte_order,
        }
    }

    pub(crate) fn fold<A, I, F, M>(
        &self,
        progress: &Progress,
        stage: &str,
        initial: I,
        accumulate: F,
        combine: M,
    ) -> Result<A>
    where
        A: Send,
        I: Fn() -> A + Sync + Send,
        F: Fn(A, f64) -> A + Sync + Send,
        M: Fn(A, A) -> A + Sync + Send,
    {
        dispatch_data_type!(self.data_type, |T| {
            self.fold_typed::<T, A, I, F, M>(progress, stage, initial, accumulate, combine)
        })
    }

    fn fold_typed<T: Scalar, A, I, F, M>(
        &self,
        progress: &Progress,
        stage: &str,
        init: I,
        add: F,
        merge: M,
    ) -> Result<A>
    where
        A: Send,
        I: Fn() -> A + Sync + Send,
        F: Fn(A, f64) -> A + Sync + Send,
        M: Fn(A, A) -> A + Sync + Send,
    {
        let block_bytes = T::SIZE * BLOCK_LENGTH;
        let blocks = self.bytes.len().div_ceil(block_bytes).max(1);
        let done = AtomicUsize::new(0);
        self.bytes
            .par_chunks(block_bytes)
            .try_fold(&init, |sum, block| {
                let sum = block
                    .chunks_exact(T::SIZE)
                    .fold(sum, |sum, raw| add(sum, T::read(raw, self.byte_order)));
                let done = done.fetch_add(1, Ordering::Relaxed) + 1;
                progress.report(done as f64 / blocks as f64, stage)?;
                Ok(sum)
            })
            .try_reduce(&init, |a, b| Ok(merge(a, b)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn folds_many_blocks_on_all_cores() {
        let count = 5 * BLOCK_LENGTH + 123;
        let bytes: Vec<u8> = (0..count)
            .flat_map(|i| ((i % 1000) as u16).to_le_bytes())
            .collect();
        let values = VoxelBuffer::new(&bytes, DataType::U16, ByteOrder::LittleEndian);
        let sum = values
            .fold(
                &Progress::default(),
                "",
                || 0.0,
                |sum, v| sum + v,
                |a, b| a + b,
            )
            .unwrap();
        let expected: f64 = (0..count).map(|i| (i % 1000) as f64).sum();
        assert_eq!(sum, expected);

        let cancelled = Progress::new(Arc::new(AtomicBool::new(true)), |_, _| {});
        let result = values.fold(&cancelled, "", || 0.0, |sum, v| sum + v, |a, b| a + b);
        assert!(matches!(result, Err(Error::Cancelled)));
    }

    #[test]
    fn reads_both_byte_orders() {
        let little = 0x1234u16.to_le_bytes();
        let big = 0x1234u16.to_be_bytes();
        assert_eq!(
            read_value(&little, 0, DataType::U16, ByteOrder::LittleEndian),
            Some(4660.0)
        );
        assert_eq!(
            read_value(&big, 0, DataType::U16, ByteOrder::BigEndian),
            Some(4660.0)
        );
    }

    #[test]
    fn reads_unaligned_and_out_of_range() {
        let mut bytes = vec![0u8];
        bytes.extend_from_slice(&1.5f32.to_le_bytes());
        assert_eq!(
            read_value(&bytes[1..], 0, DataType::F32, ByteOrder::LittleEndian),
            Some(1.5)
        );
        assert_eq!(
            read_value(&bytes[1..], 1, DataType::F32, ByteOrder::LittleEndian),
            None
        );
    }

    #[test]
    fn visits_every_value() {
        let bytes: Vec<u8> = [-2i16, 0, 7].iter().flat_map(|v| v.to_be_bytes()).collect();
        let mut seen = Vec::new();
        for_each_value(
            &bytes,
            DataType::I16,
            ByteOrder::BigEndian,
            |index, value| {
                seen.push((index, value));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(seen, [(0, -2.0), (1, 0.0), (2, 7.0)]);
    }

    #[test]
    fn stops_on_first_error() {
        let bytes = [1u8, 2, 3];
        let mut visited = 0;
        let result = for_each_value(&bytes, DataType::U8, ByteOrder::LittleEndian, |_, _| {
            visited += 1;
            Err(Error::Cancelled)
        });
        assert!(matches!(result, Err(Error::Cancelled)));
        assert_eq!(visited, 1);
    }

    #[test]
    fn size_checks() {
        assert!(checked_multiply(usize::MAX, 2).is_err());
        assert!(validate_region(100, 96, 4).is_ok());
        assert!(validate_region(100, 97, 4).is_err());
        assert!(validate_region(100, 101, 0).is_err());
    }
}
