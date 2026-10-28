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
use crate::metadata::{
    DataFile, Format, Matrix3x4, SpatialUnits, Transform, VolumeMetadata, strip_suffix_ignore_case,
};
use crate::voxel::{self, ByteOrder, DataType, Scaling};

const MAGIC_OFFSET: usize = 344;

pub(crate) fn read(path: &Path) -> Result<VolumeMetadata> {
    let header_path = header_path_for(path)?;
    let (raw, analyze) = read_header(&header_path).map_err(|reason| {
        Error::invalid(format!(
            "Cannot read header {}: {reason}",
            header_path.display()
        ))
    })?;
    convert_header(&raw, analyze, &header_path)
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

fn convert_header(header: &NiftiHeader, is_analyze: bool, path: &Path) -> Result<VolumeMetadata> {
    if header.sizeof_hdr != 348 {
        return Err(Error::invalid("Wrong header: sizeof_hdr is not 348"));
    }
    let format = match (&header.magic, is_analyze) {
        (_, true) => Format::Analyze75,
        (b"n+1\0", false) => Format::Nifti1,
        _ => Format::Nifti1Pair,
    };
    let endian = match header.endianness {
        Endianness::Big => ByteOrder::BigEndian,
        Endianness::Little => ByteOrder::LittleEndian,
    };
    let datatype = data_type(header.datatype)?;

    let ndim = usize::from(header.dim[0]);
    if !(2..=7).contains(&ndim) {
        return Err(Error::invalid("Expected a scalar 2D, 3D, or 4D image"));
    }
    let axes = &header.dim[1..=ndim];
    if axes.iter().any(|&size| size == 0 || size > i16::MAX as u16) {
        return Err(Error::invalid("Invalid dimension"));
    }
    if axes.iter().skip(4).any(|&size| size != 1) {
        return Err(Error::invalid(
            "Vector and higher dimensional images are not supported. (yet)",
        ));
    }
    let dimensions: [usize; 4] =
        std::array::from_fn(|i| axes.get(i).map_or(1, |&size| size.into()));
    let data_bytes = dimensions
        .iter()
        .try_fold(datatype.bytes_per_voxel(), |count, &size| {
            voxel::checked_multiply(count, size)
        })?;

    let spacing = [1, 2, 3].map(|axis| f64::from(header.pixdim[axis]));
    let (cal_min, cal_max) = (
        finite_or_zero(header.cal_min),
        finite_or_zero(header.cal_max),
    );
    let display_range = (cal_max > cal_min).then(|| (f64::from(cal_min), f64::from(cal_max)));

    // account for the different header structure of analyze
    let (units, scaling, transform) = if is_analyze {
        // analyze is always in mm
        let scaling = intensity_scaling(finite_or_zero(header.scl_slope), 0.0)?;
        (SpatialUnits::Millimeter, scaling, None)
    } else {
        let units = match header.xyzt_units & 0x07 {
            1 => SpatialUnits::Meter,
            2 => SpatialUnits::Millimeter,
            3 => SpatialUnits::Micrometer,
            _ => SpatialUnits::Unknown,
        };
        let scaling = intensity_scaling(
            finite_or_zero(header.scl_slope),
            finite_or_zero(header.scl_inter),
        )?;
        (units, scaling, transform(header))
    };

    if !header.vox_offset.is_finite() || header.vox_offset < 0.0 {
        return Err(Error::invalid("Invalid nifti data offset"));
    }
    let offset = header.vox_offset as u64;
    if format == Format::Nifti1 && offset < 352 {
        return Err(Error::invalid("Invalid nifti data offset"));
    }
    let data_path = match format {
        Format::Nifti1 => path.to_path_buf(),
        _ => image_path_for(path)?,
    };

    Ok(VolumeMetadata {
        format,
        dimensions,
        spacing,
        spatial_units: units,
        transform,
        data_type: datatype,
        endian,
        scaling,
        display_range,
        data: DataFile {
            gzip_compressed: is_gzip(&data_path),
            path: data_path,
            voxel_offset: offset,
            byte_count: data_bytes,
            stream_offset: 0,
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
    quaternion: [f32; 3],
    offset: [f32; 3],
    spacing: [f32; 3],
    flip_z: bool,
) -> Matrix3x4 {
    let [mut b, mut c, mut d] = quaternion.map(f64::from);
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
        .is_some_and(|extension| extension.eq_ignore_ascii_case("gz"))
}

fn header_path_for(path: &Path) -> Result<PathBuf> {
    match strip_name_suffix(path, &[".img.gz", ".img"]) {
        Some(stem) => find_sibling(path, stem, &["hdr", "hdr.gz"]),
        None => Ok(path.to_path_buf()),
    }
}

fn image_path_for(header: &Path) -> Result<PathBuf> {
    let stem = strip_name_suffix(header, &[".hdr.gz", ".hdr"]).ok_or_else(|| {
        Error::invalid("The header of a two-file image must use the .hdr/.hdr.gz extension")
    })?;
    find_sibling(header, stem, &["img", "img.gz"])
}

fn strip_name_suffix<'a>(path: &'a Path, suffixes: &[&str]) -> Option<&'a str> {
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
    use crate::test_fixtures::Fixture;

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

    fn convert(edit: impl FnOnce(&mut NiftiHeader)) -> Result<VolumeMetadata> {
        let mut raw = test_header();
        edit(&mut raw);
        convert_header(&raw, false, Path::new("test.nii"))
    }

    #[test]
    fn datatype_codes() {
        assert_eq!(data_type(4).unwrap(), DataType::I16);
        assert_eq!(data_type(16).unwrap().bytes_per_voxel(), 4);
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
        let metadata = convert(|_| {}).unwrap();
        assert_eq!(metadata.format, Format::Nifti1);
        assert_eq!(metadata.endian, ByteOrder::LittleEndian);
        assert_eq!(metadata.data_type, DataType::I16);
        assert_eq!(metadata.dimensions, [4, 3, 2, 1]);
        assert_eq!(metadata.spacing, [2.0, 3.0, 4.0]);
        assert_eq!(metadata.spatial_units, SpatialUnits::Millimeter);
        assert_eq!(metadata.transform, None);
        assert_eq!(
            metadata.data,
            DataFile {
                path: PathBuf::from("test.nii"),
                voxel_offset: 352,
                byte_count: 4 * 3 * 2 * 2,
                gzip_compressed: false,
                stream_offset: 0,
            }
        );
    }

    #[test]
    fn keeps_byte_order() {
        let metadata = convert(|h| h.endianness = Endianness::Big).unwrap();
        assert_eq!(metadata.endian, ByteOrder::BigEndian);
    }

    #[test]
    fn finds_image_of_pair() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("brain.hdr");
        let mut raw = test_header();
        raw.magic = *b"ni1\0";
        raw.vox_offset = 0.0;
        assert!(convert_header(&raw, false, &path).is_err());

        let image = directory.path().join("brain.img");
        std::fs::write(&image, []).unwrap();
        let metadata = convert_header(&raw, false, &path).unwrap();
        assert_eq!(metadata.format, Format::Nifti1Pair);
        assert_eq!(metadata.data.path, image);
        assert_eq!(metadata.data.voxel_offset, 0);
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
        let metadata = convert(|h| h.dim = [3, 4, 3, 2, 0, 0, 0, 0]).unwrap();
        assert_eq!(metadata.timepoints(), 1);

        let metadata = convert(|h| h.dim = [4, 4, 3, 2, 5, 1, 1, 1]).unwrap();
        assert_eq!(metadata.timepoints(), 5);
        assert_eq!(metadata.data.byte_count, 4 * 3 * 2 * 5 * 2);
    }

    #[test]
    fn non_finite_scaling_is_ignored() {
        let metadata = convert(|h| {
            h.scl_slope = f32::NAN;
            h.scl_inter = 5.0;
        })
        .unwrap();
        assert_eq!(metadata.scaling, Scaling::default());

        let metadata = convert(|h| {
            h.scl_slope = 2.0;
            h.scl_inter = f32::INFINITY;
        })
        .unwrap();
        assert_eq!(
            metadata.scaling,
            Scaling {
                slope: 2.0,
                intercept: 0.0
            }
        );
    }

    #[test]
    fn qform_from_quaternion() {
        let metadata = convert(|h| {
            h.qform_code = 1;
            h.quatern_x = 10.0;
            h.quatern_y = 20.0;
            h.quatern_z = 30.0;
        })
        .unwrap();
        assert_eq!(
            metadata.transform,
            Some(Transform {
                name: "qform",
                matrix: [
                    [2.0, 0.0, 0.0, 10.0],
                    [0.0, 3.0, 0.0, 20.0],
                    [0.0, 0.0, 4.0, 30.0]
                ],
            })
        );

        let metadata = convert(|h| {
            h.qform_code = 1;
            h.quatern_d = 1.0;
            h.pixdim[0] = -1.0;
        })
        .unwrap();
        assert_eq!(
            metadata.transform.unwrap().matrix,
            [
                [-2.0, 0.0, 0.0, 0.0],
                [0.0, -3.0, 0.0, 0.0],
                [0.0, 0.0, -4.0, 0.0]
            ]
        );

        let metadata = convert(|h| {
            h.qform_code = 1;
            h.pixdim[1] = -2.0;
        })
        .unwrap();
        assert_eq!(metadata.transform.unwrap().matrix[0][0], 1.0);

        assert_eq!(convert(|h| h.qform_code = -1).unwrap().transform, None);
    }

    #[test]
    fn prefers_sform_to_qform() {
        let metadata = convert(|h| {
            h.qform_code = 1;
            h.sform_code = 2;
            h.srow_x = [1.0, 0.0, 0.0, -90.0];
            h.srow_y = [0.0, 1.0, 0.0, -126.0];
            h.srow_z = [0.0, 0.0, 1.0, -72.0];
        })
        .unwrap();
        assert_eq!(
            metadata.transform,
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
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("brain.hdr");
        let analyze = Fixture {
            analyze_format: true,
            intercept: 5.0,
            ..Fixture::default()
        };
        analyze.write(&path);
        let metadata = read(&path).unwrap();
        assert_eq!(metadata.format, Format::Analyze75);
        assert_eq!(metadata.transform, None);
        assert_eq!(metadata.scaling, Scaling::default());
        assert_eq!(metadata.spatial_units, SpatialUnits::Millimeter);
        assert_eq!(metadata.data.path, directory.path().join("brain.img"));
    }

    #[test]
    fn opens_pairs_from_either_file() {
        let directory = tempfile::tempdir().unwrap();
        let header = directory.path().join("pair.hdr");
        Fixture::default().write(&header);
        let from_header = read(&header).unwrap();
        assert_eq!(from_header.format, Format::Nifti1Pair);
        assert_eq!(
            read(&directory.path().join("pair.img")).unwrap(),
            from_header
        );
        assert!(read(&directory.path().join("lonely.img")).is_err());
    }

    #[test]
    fn opens_pairs_with_upper_case_extensions() {
        let directory = tempfile::tempdir().unwrap();
        Fixture::default().write(&directory.path().join("pair.hdr"));
        std::fs::rename(
            directory.path().join("pair.hdr"),
            directory.path().join("PAIR.HDR"),
        )
        .unwrap();
        std::fs::rename(
            directory.path().join("pair.img"),
            directory.path().join("PAIR.IMG"),
        )
        .unwrap();
        assert!(read(&directory.path().join("PAIR.HDR")).is_ok());
        assert!(read(&directory.path().join("PAIR.IMG")).is_ok());
    }
}
