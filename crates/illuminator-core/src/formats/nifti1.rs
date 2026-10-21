//! Reads NIfTI-1 and Analyze 7.5 headers.
//!
//! Supports `.nii` and `.nii.gz` files, and `.hdr` + `.img` pairs.
//! See <https://nifti.nimh.nih.gov/index.html>

use flate2::read::MultiGzDecoder;
use nifti::{Endianness, NiftiHeader};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::meta::{
    DataFile, Format, Mat3x4, SpatialUnits, Transform, VolumeMeta, strip_suffix_ignore_case,
};
use crate::voxel::{self, DataType, Endian, Scaling};

const MAGIC_OFFSET: usize = 344;

pub(crate) fn read(path: &Path) -> Result<VolumeMeta> {
    let header_path = header_path_for(path)?;
    let (raw, analyze) = read_header(&header_path).map_err(|reason| {
        Error::invalid(format!(
            "Cannot read header {}: {reason}",
            header_path.display()
        ))
    })?;
    from_nifti(&raw, analyze, &header_path)
}

fn read_header(path: &Path) -> std::result::Result<(NiftiHeader, bool), String> {
    let mut bytes = [0u8; 348];
    let mut file = BufReader::new(File::open(path).map_err(|error| error.to_string())?);
    let read = if is_gzip(path) {
        MultiGzDecoder::new(file).read_exact(&mut bytes)
    } else {
        file.read_exact(&mut bytes)
    };
    read.map_err(|error| error.to_string())?;
    let analyze = !matches!(&bytes[MAGIC_OFFSET..MAGIC_OFFSET + 4], b"n+1\0" | b"ni1\0");
    if analyze {
        bytes[MAGIC_OFFSET..MAGIC_OFFSET + 4].copy_from_slice(b"ni1\0");
    }
    let raw = NiftiHeader::from_reader(&bytes[..]).map_err(|error| match error {
        // The nifti crates Io error has no message, so we show the message of the io::Error inside it.
        nifti::NiftiError::Io(error) => error.to_string(),
        other => other.to_string(),
    })?;
    Ok((raw, analyze))
}

fn from_nifti(raw: &NiftiHeader, analyze: bool, path: &Path) -> Result<VolumeMeta> {
    if raw.sizeof_hdr != 348 {
        return Err(Error::invalid("Wrong header: sizeof_hdr is not 348"));
    }
    let format = match (&raw.magic, analyze) {
        (_, true) => Format::Analyze75,
        (b"n+1\0", false) => Format::Nifti1,
        _ => Format::Nifti1Pair,
    };
    let endian = match raw.endianness {
        Endianness::Big => Endian::Big,
        Endianness::Little => Endian::Little,
    };
    let datatype = data_type(raw.datatype)?;

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
            "Vector and higher dimensional images are not supported. (yet)",
        ));
    }
    let dims: [usize; 4] = std::array::from_fn(|i| axes.get(i).map_or(1, |&size| size.into()));
    let data_bytes = dims.iter().try_fold(datatype.size(), |count, &size| {
        voxel::checked_mul(count, size)
    })?;

    let spacing = [1, 2, 3].map(|axis| f64::from(raw.pixdim[axis]));
    let (cal_min, cal_max) = (finite_or_zero(raw.cal_min), finite_or_zero(raw.cal_max));
    let display_range = (cal_max > cal_min).then(|| (f64::from(cal_min), f64::from(cal_max)));

    // account for the different header structure of analyze
    let (units, scaling, transform) = if analyze {
        // analyze is always in mm
        let scaling = intensity_scaling(finite_or_zero(raw.scl_slope), 0.0)?;
        (SpatialUnits::Millimeter, scaling, None)
    } else {
        let units = match raw.xyzt_units & 0x07 {
            1 => SpatialUnits::Meter,
            2 => SpatialUnits::Millimeter,
            3 => SpatialUnits::Micron,
            _ => SpatialUnits::Unknown,
        };
        let scaling =
            intensity_scaling(finite_or_zero(raw.scl_slope), finite_or_zero(raw.scl_inter))?;
        (units, scaling, transform(raw))
    };

    if !raw.vox_offset.is_finite() || raw.vox_offset < 0.0 {
        return Err(Error::invalid("Invalid nifti data offset"));
    }
    let offset = raw.vox_offset as u64;
    if format == Format::Nifti1 && offset < 352 {
        return Err(Error::invalid("Invalid nifti data offset"));
    }
    let data_path = match format {
        Format::Nifti1 => path.to_path_buf(),
        _ => image_path_for(path)?,
    };

    Ok(VolumeMeta {
        format,
        dims,
        spacing,
        units,
        transform,
        datatype,
        endian,
        scaling,
        display_range,
        data: DataFile {
            gzip: is_gzip(&data_path),
            path: data_path,
            offset,
            bytes: data_bytes,
            start: 0,
        },
    })
}

/// The voxel type of a NIfTI file.
fn data_type(code: i16) -> Result<DataType> {
    match code {
        2 => Ok(DataType::U8),
        4 => Ok(DataType::I16),
        8 => Ok(DataType::I32),
        16 => Ok(DataType::F32),
        64 => Ok(DataType::F64),
        256 => Ok(DataType::I8),
        512 => Ok(DataType::U16),
        768 => Ok(DataType::U32),
        _ => Err(Error::invalid(
            "Unsupported datatype: expected 8/16/32-bit scalar integer, or 32/64-bit float",
        )),
    }
}

/// The scaling from `scl_slope` and `scl_inter`.
fn intensity_scaling(slope: f32, intercept: f32) -> Result<Scaling> {
    if slope == 0.0 {
        return Ok(Scaling::default());
    }
    if !slope.is_finite() || !intercept.is_finite() {
        return Err(Error::invalid("Invalid nifti intensity scaling"));
    }
    Ok(Scaling {
        slope: slope as f64,
        intercept: intercept as f64,
    })
}

/// sform if possible, else qform
fn transform(raw: &NiftiHeader) -> Option<Transform> {
    if raw.sform_code > 0 {
        let rows = [raw.srow_x, raw.srow_y, raw.srow_z];
        return Some(Transform {
            name: "sform",
            matrix: rows.map(|row| row.map(f64::from)),
        });
    }
    (raw.qform_code > 0).then(|| Transform {
        name: "qform",
        matrix: quaternion_to_matrix(
            [raw.quatern_b, raw.quatern_c, raw.quatern_d].map(finite_or_zero),
            [raw.quatern_x, raw.quatern_y, raw.quatern_z].map(finite_or_zero),
            [raw.pixdim[1], raw.pixdim[2], raw.pixdim[3]],
            raw.pixdim[0] < 0.0,
        ),
    })
}

/// Copy nifti_clib behaviour: 'FIXED_FLOAT' -> non finite values become 0
fn finite_or_zero(value: f32) -> f32 {
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

fn header_path_for(path: &Path) -> Result<PathBuf> {
    match split_name(path, &[".img.gz", ".img"]) {
        Some(stem) => find_sibling(path, stem, &["hdr", "hdr.gz"]),
        None => Ok(path.to_path_buf()),
    }
}

fn image_path_for(header: &Path) -> Result<PathBuf> {
    let stem = split_name(header, &[".hdr.gz", ".hdr"]).ok_or_else(|| {
        Error::invalid("The header of a two-file image must use the .hdr/.hdr.gz extension")
    })?;
    find_sibling(header, stem, &["img", "img.gz"])
}

fn split_name<'a>(path: &'a Path, suffixes: &[&str]) -> Option<&'a str> {
    strip_suffix_ignore_case(path.file_name()?.to_str()?, suffixes)
}

fn find_sibling(path: &Path, stem: &str, extensions: &[&str]) -> Result<PathBuf> {
    // some analyze images have uppercase extensions
    extensions
        .iter()
        .flat_map(|extension| [extension.to_string(), extension.to_ascii_uppercase()])
        .map(|extension| path.with_file_name(format!("{stem}.{extension}")))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            Error::invalid(format!(
                "Cannot find the {} file for {}",
                extensions[0],
                path.display()
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::Fixture;

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

    fn convert(edit: impl FnOnce(&mut NiftiHeader)) -> Result<VolumeMeta> {
        let mut raw = test_header();
        edit(&mut raw);
        from_nifti(&raw, false, Path::new("test.nii"))
    }

    #[test]
    fn datatype_codes() {
        assert_eq!(data_type(4).unwrap(), DataType::I16);
        assert_eq!(data_type(16).unwrap().size(), 4);
        assert!(data_type(128).is_err());
    }

    #[test]
    fn scaling() {
        assert_eq!(intensity_scaling(0.0, 5.0).unwrap(), Scaling::default());
        assert_eq!(intensity_scaling(2.0, 1.0).unwrap().apply(3.0), 7.0);
        assert!(intensity_scaling(f32::NAN, 0.0).is_err());
    }

    #[test]
    fn converts_valid_header() {
        let meta = convert(|_| {}).unwrap();
        assert_eq!(meta.format, Format::Nifti1);
        assert_eq!(meta.endian, Endian::Little);
        assert_eq!(meta.datatype, DataType::I16);
        assert_eq!(meta.dims, [4, 3, 2, 1]);
        assert_eq!(meta.spacing, [2.0, 3.0, 4.0]);
        assert_eq!(meta.units, SpatialUnits::Millimeter);
        assert_eq!(meta.transform, None);
        assert_eq!(
            meta.data,
            DataFile {
                path: PathBuf::from("test.nii"),
                offset: 352,
                bytes: 4 * 3 * 2 * 2,
                gzip: false,
                start: 0,
            }
        );
    }

    #[test]
    fn keeps_byte_order() {
        let meta = convert(|h| h.endianness = Endianness::Big).unwrap();
        assert_eq!(meta.endian, Endian::Big);
    }

    #[test]
    fn finds_image_of_pair() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brain.hdr");
        let mut raw = test_header();
        raw.magic = *b"ni1\0";
        raw.vox_offset = 0.0;
        assert!(from_nifti(&raw, false, &path).is_err());

        let image = dir.path().join("brain.img");
        std::fs::write(&image, []).unwrap();
        let meta = from_nifti(&raw, false, &path).unwrap();
        assert_eq!(meta.format, Format::Nifti1Pair);
        assert_eq!(meta.data.path, image);
        assert_eq!(meta.data.offset, 0);
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
        let meta = convert(|h| h.dim = [3, 4, 3, 2, 0, 0, 0, 0]).unwrap();
        assert_eq!(meta.timepoints(), 1);

        let meta = convert(|h| h.dim = [4, 4, 3, 2, 5, 1, 1, 1]).unwrap();
        assert_eq!(meta.timepoints(), 5);
        assert_eq!(meta.data.bytes, 4 * 3 * 2 * 5 * 2);
    }

    #[test]
    fn non_finite_scaling_is_ignored() {
        let meta = convert(|h| {
            h.scl_slope = f32::NAN;
            h.scl_inter = 5.0;
        })
        .unwrap();
        assert_eq!(meta.scaling, Scaling::default());

        let meta = convert(|h| {
            h.scl_slope = 2.0;
            h.scl_inter = f32::INFINITY;
        })
        .unwrap();
        assert_eq!(
            meta.scaling,
            Scaling {
                slope: 2.0,
                intercept: 0.0
            }
        );
    }

    #[test]
    fn qform_from_quaternion() {
        let meta = convert(|h| {
            h.qform_code = 1;
            h.quatern_x = 10.0;
            h.quatern_y = 20.0;
            h.quatern_z = 30.0;
        })
        .unwrap();
        assert_eq!(
            meta.transform,
            Some(Transform {
                name: "qform",
                matrix: [
                    [2.0, 0.0, 0.0, 10.0],
                    [0.0, 3.0, 0.0, 20.0],
                    [0.0, 0.0, 4.0, 30.0]
                ],
            })
        );

        let meta = convert(|h| {
            h.qform_code = 1;
            h.quatern_d = 1.0;
            h.pixdim[0] = -1.0;
        })
        .unwrap();
        assert_eq!(
            meta.transform.unwrap().matrix,
            [
                [-2.0, 0.0, 0.0, 0.0],
                [0.0, -3.0, 0.0, 0.0],
                [0.0, 0.0, -4.0, 0.0]
            ]
        );

        let meta = convert(|h| {
            h.qform_code = 1;
            h.pixdim[1] = -2.0;
        })
        .unwrap();
        assert_eq!(meta.transform.unwrap().matrix[0][0], 1.0);

        assert_eq!(convert(|h| h.qform_code = -1).unwrap().transform, None);
    }

    #[test]
    fn prefers_sform_to_qform() {
        let meta = convert(|h| {
            h.qform_code = 1;
            h.sform_code = 2;
            h.srow_x = [1.0, 0.0, 0.0, -90.0];
            h.srow_y = [0.0, 1.0, 0.0, -126.0];
            h.srow_z = [0.0, 0.0, 1.0, -72.0];
        })
        .unwrap();
        assert_eq!(
            meta.transform,
            Some(Transform {
                name: "sform",
                matrix: [
                    [1.0, 0.0, 0.0, -90.0],
                    [0.0, 1.0, 0.0, -126.0],
                    [0.0, 0.0, 1.0, -72.0]
                ],
            })
        );
    }

    #[test]
    fn reads_analyze_headers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brain.hdr");
        let analyze = Fixture {
            analyze: true,
            intercept: 5.0,
            ..Fixture::default()
        };
        analyze.write(&path);
        let meta = read(&path).unwrap();
        assert_eq!(meta.format, Format::Analyze75);
        assert_eq!(meta.transform, None);
        assert_eq!(meta.scaling, Scaling::default());
        assert_eq!(meta.units, SpatialUnits::Millimeter);
        assert_eq!(meta.data.path, dir.path().join("brain.img"));
    }

    #[test]
    fn opens_pairs_from_either_file() {
        let dir = tempfile::tempdir().unwrap();
        let header = dir.path().join("pair.hdr");
        Fixture::default().write(&header);
        let from_header = read(&header).unwrap();
        assert_eq!(from_header.format, Format::Nifti1Pair);
        assert_eq!(read(&dir.path().join("pair.img")).unwrap(), from_header);
        assert!(read(&dir.path().join("lonely.img")).is_err());
    }

    #[test]
    fn opens_pairs_with_upper_case_extensions() {
        let dir = tempfile::tempdir().unwrap();
        Fixture::default().write(&dir.path().join("pair.hdr"));
        std::fs::rename(dir.path().join("pair.hdr"), dir.path().join("PAIR.HDR")).unwrap();
        std::fs::rename(dir.path().join("pair.img"), dir.path().join("PAIR.IMG")).unwrap();
        assert!(read(&dir.path().join("PAIR.HDR")).is_ok());
        assert!(read(&dir.path().join("PAIR.IMG")).is_ok());
    }
}
