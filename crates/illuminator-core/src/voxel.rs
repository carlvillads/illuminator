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
    /// Bytes per voxel
    pub(crate) fn size(self) -> usize {
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
pub(crate) enum Endian {
    Little,
    Big,
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

pub(crate) fn checked_mul(a: usize, b: usize) -> Result<usize> {
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

macro_rules! with_scalar {
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
pub(crate) use with_scalar;

/// Read voxel index af unscaled float64, returns none if outside of bytes
pub(crate) fn read_value(
    bytes: &[u8],
    index: usize,
    data_type: DataType,
    endian: Endian,
) -> Option<f64> {
    let size = data_type.size();
    let start = index.checked_mul(size)?;
    let chunk = bytes.get(start..start.checked_add(size)?)?;
    Some(with_scalar!(data_type, |T| T::read(chunk, endian)))
}

/// Calls `f(idx, val)` for every voxel in `bytes`, stopping at first error.
pub(crate) fn for_each_value<F>(
    bytes: &[u8],
    data_type: DataType,
    endian: Endian,
    mut f: F,
) -> Result<()>
where
    F: FnMut(usize, f64) -> Result<()>,
{
    with_scalar!(data_type, |T| for_each_typed::<T, _>(bytes, endian, &mut f))
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

const PIECE: usize = 1 << 16;

pub(crate) struct Values<'a> {
    bytes: &'a [u8],
    data_type: DataType,
    endian: Endian,
}

impl<'a> Values<'a> {
    pub(crate) fn new(bytes: &'a [u8], data_type: DataType, endian: Endian) -> Self {
        Self {
            bytes,
            data_type,
            endian,
        }
    }

    pub(crate) fn fold<A, I, F, M>(
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
        with_scalar!(self.data_type, |T| {
            self.fold_typed::<T, A, I, F, M>(progress, stage, init, add, merge)
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
        let piece_bytes = T::SIZE * PIECE;
        let pieces = self.bytes.len().div_ceil(piece_bytes).max(1);
        let done = AtomicUsize::new(0);
        self.bytes
            .par_chunks(piece_bytes)
            .try_fold(&init, |sum, piece| {
                let sum = piece
                    .chunks_exact(T::SIZE)
                    .fold(sum, |sum, raw| add(sum, T::read(raw, self.endian)));
                let done = done.fetch_add(1, Ordering::Relaxed) + 1;
                progress.report(done as f64 / pieces as f64, stage)?;
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
    fn folds_many_pieces_on_all_cores() {
        let count = 5 * PIECE + 123;
        let bytes: Vec<u8> = (0..count)
            .flat_map(|i| ((i % 1000) as u16).to_le_bytes())
            .collect();
        let values = Values::new(&bytes, DataType::U16, Endian::Little);
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
    fn size_checks() {
        assert!(checked_mul(usize::MAX, 2).is_err());
        assert!(validate_region(100, 96, 4).is_ok());
        assert!(validate_region(100, 97, 4).is_err());
        assert!(validate_region(100, 101, 0).is_err());
    }
}
