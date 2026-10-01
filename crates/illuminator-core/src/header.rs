//! Reads NIfTI-1 headers
//!
//! Should support `.nii`, `.nii.gz` files and `.hdr` + `.img` pairs.

use nifti::{Endianness, NiftiHeader};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::voxel::{self, DataType, Endian, Scaling};

/// Top three rows of a 4x4 affine mmtrix
pub type Mat3x4 = [[f64; 4]; 3];

/// Spatial units from `xyzt_units`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpatialUnits {
    Unknown,
    Meter,
    Millimeter,
    Micron,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NiiKind {
    /// `.nii`, `.nii.gz`. magic: `n+1`
    Single,
    /// `.hdr` + `.img`. magic: `ni1`
    Pair,
}

#[derive(Debug, Clone)]
pub struct Header {
    pub kind: NiiKind,
    pub endian: Endian,
    pub datatype: DataType,
    pub dim: [u16; 8], // raw dim! `dim[0]` is n dimensions
    pub pixdim: [f32; 8],
    pub scaling: Scaling,
    pub cal_min: f32,
    pub cal_max: f32,
    pub units: SpatialUnits,
    pub qform_code: i16,
    pub sform_code: i16,
    /// Voxel to world matrix from quaternion if `qform_code > 0`
    pub qform: Option<Mat3x4>,
    /// Voxel-to-world matrix from `srow_x/y/z` if `sform_code > 0`
    pub sform: Option<Mat3x4>,
    pub data_path: PathBuf,
    pub data_offset: u64,
    pub data_bytes: usize,
}

impl Header {
    pub fn read(path: &Path) -> Result<Self> {
        let raw = NiftiHeader::from_file(path).map_err(|error| {
            Error::invalid(format!(
                "Cannot read nifti-1 header {}: {error}",
                path.display()
            ))
        })?;
        Self::from_nifti(&raw, path)
    }

    pub fn axis_size(&self, axis: usize) -> usize {
        debug_assert!((1..=7).contains(&axis));
        if axis > self.dim[0] as usize {
            1
        } else {
            self.dim[axis] as usize
        }
    }

    pub fn width(&self) -> usize {
        self.axis_size(1)
    }

    pub fn height(&self) -> usize {
        self.axis_size(2)
    }

    pub fn depth(&self) -> usize {
        self.axis_size(3)
    }

    pub fn timepoints(&self) -> usize {
        self.axis_size(4)
    }

    /// Voxels in one 3D frame
    pub fn frame_voxels(&self) -> usize {
        self.width() * self.height() * self.depth()
    }

    /// True if voxel data has to be decompreseed
    pub fn is_gzipped(&self) -> bool {
        is_gzip(&self.data_path)
    }

    fn from_nifti(raw: &NiftiHeader, path: &Path) -> Result<Self> {
        if raw.sizeof_hdr != 348 {
            return Err(Error::invalid("Wrong header: sizeof_hdr is not 348"));
        }
        let kind = match &raw.magic {
            b"n+1\0" => NiiKind::Single,
            b"ni1\0" => NiiKind::Pair,
            _ => {
                return Err(Error::invalid("Only nifti-1 images are supported"));
            }
        };
        let endian = match raw.endianness {
            Endianness::Little => Endian::Little,
            Endianness::Big => Endian::Big,
        };
        let datatype = DataType::from_code(raw.datatype)?;

        let scaling = Scaling::from_header(finite(raw.scl_slope), finite(raw.scl_inter))?;
        let units = match raw.xyzt_units & 0x07 {
            1 => SpatialUnits::Meter,
            2 => SpatialUnits::Millimeter,
            3 => SpatialUnits::Micron,
            _ => SpatialUnits::Unknown,
        };
        let qform_code = raw.qform_code.max(0);
        let sform_code = raw.sform_code.max(0);

        let qform = (qform_code > 0).then(|| {
            quaternion_to_matrix(
                [raw.quatern_b, raw.quatern_c, raw.quatern_d].map(finite),
                [raw.quatern_x, raw.quatern_y, raw.quatern_z].map(finite),
                [raw.pixdim[1], raw.pixdim[2], raw.pixdim[3]],
                raw.pixdim[0] < 0.0,
            )
        });
        let sform = (sform_code > 0)
            .then(|| [raw.srow_x, raw.srow_y, raw.srow_z].map(|row| row.map(f64::from)));

        let ndim = usize::from(raw.dim[0]);
        if !(2..=7).contains(&ndim) {
            return Err(Error::invalid("Expected a scalar 2D, 3D, or 4D image"));
        }
        let axes = &raw.dim[1..=ndim];
        if axes.iter().any(|&size| size == 0 || size > i16::MAX as u16) {
            return Err(Error::invalid("Invalid dimension"));
        }
        if axes.iter().skip(4).any(|&size| size != 1) {
            return Err(Error::invalid(
                "Vector and higher dimensional images aren't supported (yet)",
            ));
        }
        let voxel_count = axes.iter().try_fold(1usize, |count, &size| {
            voxel::checked_mul(count, usize::from(size))
        })?;
        let data_bytes = voxel::checked_mul(voxel_count, datatype.size())?;

        if !raw.vox_offset.is_finite() || raw.vox_offset < 0.0 {
            return Err(Error::invalid("Invalid nifti data offset"));
        }
        let data_offset = raw.vox_offset as u64;
        if kind == NiiKind::Single && data_offset < 352 {
            return Err(Error::invalid("Invalid nifti data offset"));
        }

        let data_path = match kind {
            NiiKind::Pair => image_path_for(path)?,
            NiiKind::Single => path.to_path_buf(),
        };

        Ok(Self {
            kind,
            endian,
            datatype,
            dim: raw.dim,
            pixdim: raw.pixdim,
            scaling,
            cal_min: finite(raw.cal_min),
            cal_max: finite(raw.cal_max),
            units,
            qform_code,
            sform_code,
            qform,
            sform,
            data_path,
            data_offset,
            data_bytes,
        })
    }
}

/// Copy nifti_clib behaviour: 'FIXED_FLOAT' -> non finite values become 0
fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn quaternion_to_matrix(
    quatern: [f32; 3],
    offset: [f32; 3],
    spacing: [f32; 3],
    flip_z: bool,
) -> Mat3x4 {
    let [mut b, mut c, mut d] = quatern.map(f64::from);
    let mut a = 1.0 - (b * b + c * c + d * d);
    if a < 1e-7 {
        // len(b,c,d) approx 1, thus we normalize it. if a = 0, it's a 180 deg rotation
        let norm = 1.0 / (b * b + c * c + d * d).sqrt();
        b *= norm;
        c *= norm;
        d *= norm;
        a = 0.0;
    } else {
        a = a.sqrt();
    }
    let [x, y, z] = spacing.map(|s| if s > 0.0 { f64::from(s) } else { 1.0 });
    let z = if flip_z { -z } else { z };
    let [ox, oy, oz] = offset.map(f64::from);
    [
        [
            (a * a + b * b - c * c - d * d) * x,
            2.0 * (b * c - a * d) * y,
            2.0 * (b * d + a * c) * z,
            ox,
        ],
        [
            2.0 * (b * c + a * d) * x,
            (a * a + c * c - b * b - d * d) * y,
            2.0 * (c * d - a * b) * z,
            oy,
        ],
        [
            2.0 * (b * d - a * c) * x,
            2.0 * (c * d + a * b) * y,
            (a * a + d * d - c * c - b * b) * z,
            oz,
        ],
    ]
}

fn is_gzip(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("gz"))
}

/// finds the image file that belongs to the header file if NiiType::Pair
fn image_path_for(header: &Path) -> Result<PathBuf> {
    let name = header
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    let suffix = [".hdr.gz", ".hdr"]
        .into_iter()
        .find(|suffix| lower.ends_with(suffix))
        .ok_or_else(|| Error::invalid("A two-file nifti header must use the .hdr extension"))?;
    let stem = &name[..name.len() - suffix.len()];
    for extension in ["img", "img.gz"] {
        let candidate = header.with_file_name(format!("{stem}.{extension}"));
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err(Error::invalid(format!(
        "Cannot find the .img file for {}",
        header.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_header() -> NiftiHeader {
        NiftiHeader {
            sizeof_hdr: 348,
            dim: [3, 4, 3, 2, 1, 1, 1, 1],
            datatype: 4,
            bitpix: 16,
            pixdim: [1.0, 2.0, 3.0, 4.0, 1.0, 1.0, 1.0, 1.0],
            vox_offset: 352.0,
            xyzt_units: 2,
            qform_code: 0,
            sform_code: 0,
            magic: *b"n+1\0",
            endianness: Endianness::Little,
            ..NiftiHeader::default()
        }
    }

    fn convert(edit: impl FnOnce(&mut NiftiHeader)) -> Result<Header> {
        let mut raw = test_header();
        edit(&mut raw);
        Header::from_nifti(&raw, Path::new("test.nii"))
    }

    #[test]
    fn converts_valid_header() {
        let header = convert(|_| {}).unwrap();
        assert_eq!(header.kind, NiiKind::Single);
        assert_eq!(header.endian, Endian::Little);
        assert_eq!(header.datatype, DataType::I16);
        assert_eq!(
            (
                header.width(),
                header.height(),
                header.depth(),
                header.timepoints()
            ),
            (4, 3, 2, 1)
        );
        assert_eq!(header.data_bytes, 4 * 3 * 2 * 2);
        assert_eq!(header.data_offset, 352);
        assert_eq!(header.data_path, Path::new("test.nii"));
        assert_eq!(header.units, SpatialUnits::Millimeter);
        assert_eq!(header.qform, None);
        assert_eq!(header.sform, None);
    }

    #[test]
    fn keeps_byte_order() {
        let header = convert(|h| h.endianness = Endianness::Big).unwrap();
        assert_eq!(header.endian, Endian::Big);
    }

    #[test]
    fn finds_image_of_pair() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brain.hdr");
        let mut raw = test_header();
        raw.magic = *b"ni1\0";
        raw.vox_offset = 0.0;
        assert!(Header::from_nifti(&raw, &path).is_err());

        let image = dir.path().join("brain.img");
        std::fs::write(&image, []).unwrap();
        let header = Header::from_nifti(&raw, &path).unwrap();
        assert_eq!(header.kind, NiiKind::Pair);
        assert_eq!(header.data_path, image);
        assert_eq!(header.data_offset, 0);
    }

    #[test]
    fn rejects_unsupported_headers() {
        assert!(convert(|h| h.sizeof_hdr = 540).is_err());
        assert!(convert(|h| h.datatype = 128).is_err());
        assert!(convert(|h| h.dim[0] = 1).is_err());
        assert!(convert(|h| h.dim = [5, 4, 3, 2, 1, 3, 1, 1]).is_err());
        assert!(convert(|h| h.dim[1] = 0).is_err());
        assert!(convert(|h| h.dim[1] = (-1i16) as u16).is_err());
        assert!(convert(|h| h.vox_offset = 348.0).is_err());
        assert!(convert(|h| h.vox_offset = f32::NAN).is_err());
    }

    #[test]
    fn dimensions_past_dim0_are_ignored() {
        let header = convert(|h| h.dim = [3, 4, 3, 2, 0, 0, 0, 0]).unwrap();
        assert_eq!(header.timepoints(), 1);

        let header = convert(|h| h.dim = [4, 4, 3, 2, 5, 1, 1, 1]).unwrap();
        assert_eq!(header.timepoints(), 5);
        assert_eq!(header.data_bytes, 4 * 3 * 2 * 5 * 2);
    }

    #[test]
    fn non_finite_scaling_is_ignored() {
        let header = convert(|h| {
            h.scl_slope = f32::NAN;
            h.scl_inter = 5.0;
        })
        .unwrap();
        assert_eq!(header.scaling, Scaling::default());

        let header = convert(|h| {
            h.scl_slope = 2.0;
            h.scl_inter = f32::INFINITY;
        })
        .unwrap();
        assert_eq!(
            header.scaling,
            Scaling {
                slope: 2.0,
                intercept: 0.0
            }
        );
    }

    #[test]
    fn qform_from_quaternion() {
        let header = convert(|h| {
            h.qform_code = 1;
            h.quatern_x = 10.0;
            h.quatern_y = 20.0;
            h.quatern_z = 30.0;
        })
        .unwrap();
        assert_eq!(
            header.qform,
            Some([
                [2.0, 0.0, 0.0, 10.0],
                [0.0, 3.0, 0.0, 20.0],
                [0.0, 0.0, 4.0, 30.0]
            ])
        );

        let header = convert(|h| {
            h.qform_code = 1;
            h.quatern_d = 1.0;
            h.pixdim[0] = -1.0;
        })
        .unwrap();
        assert_eq!(
            header.qform,
            Some([
                [-2.0, 0.0, 0.0, 0.0],
                [0.0, -3.0, 0.0, 0.0],
                [0.0, 0.0, -4.0, 0.0]
            ])
        );

        let header = convert(|h| {
            h.qform_code = 1;
            h.pixdim[1] = -2.0;
        })
        .unwrap();
        assert_eq!(header.qform.unwrap()[0][0], 1.0);

        let header = convert(|h| h.qform_code = -1).unwrap();
        assert_eq!(header.qform_code, 0);
        assert_eq!(header.qform, None);
    }

    #[test]
    fn sform_rows() {
        let header = convert(|h| {
            h.sform_code = 2;
            h.srow_x = [1.0, 0.0, 0.0, -90.0];
            h.srow_y = [0.0, 1.0, 0.0, -126.0];
            h.srow_z = [0.0, 0.0, 1.0, -72.0];
        })
        .unwrap();
        assert_eq!(header.sform_code, 2);
        assert_eq!(
            header.sform,
            Some([
                [1.0, 0.0, 0.0, -90.0],
                [0.0, 1.0, 0.0, -126.0],
                [0.0, 0.0, 1.0, -72.0]
            ])
        );
    }
}
