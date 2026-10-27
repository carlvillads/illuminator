//! A volume's header in the same form for every file format.
//!
//! `VolumeMetadata::read` picks the reader from the file name and [`strip_extension`] removes
//! the extensions of volume files.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::formats::{nifti1, nrrd};
use crate::voxel::{ByteOrder, DataType, Scaling};

pub(crate) type Matrix3x4 = [[f64; 4]; 3];

const SUPPORTED_EXTENSIONS: [&str; 8] = [
    ".nii.gz", ".nii", ".nrrd", ".nhdr", ".hdr.gz", ".hdr", ".img.gz", ".img",
];

/// Removes file extension from a string.
pub fn strip_extension(name: &str) -> &str {
    strip_suffix_ignore_case(name, &SUPPORTED_EXTENSIONS).unwrap_or(name)
}

/// `name` without the first of the lowercase `suffixes`.
pub(crate) fn strip_suffix_ignore_case<'a>(name: &'a str, suffixes: &[&str]) -> Option<&'a str> {
    let lower = name.to_ascii_lowercase();
    let suffix = suffixes.iter().find(|suffix| lower.ends_with(*suffix))?;
    Some(&name[..name.len() - suffix.len()])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Nifti1,
    Nifti1Pair,
    Analyze75,
    Nrrd,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Self::Nifti1 => "NIfTI-1",
            Self::Nifti1Pair => "NIfTI-1 pair",
            Self::Analyze75 => "Analyze 7.5",
            Self::Nrrd => "NRRD",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpatialUnits {
    Unknown,
    Meter,
    Millimeter,
    Micrometer,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Transform {
    pub(crate) name: &'static str,
    pub(crate) matrix: Matrix3x4,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DataFile {
    pub(crate) path: PathBuf,
    pub(crate) stream_offset: u64,
    pub(crate) voxel_offset: u64,
    pub(crate) byte_count: usize,
    pub(crate) gzip_compressed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VolumeMetadata {
    pub format: Format,
    pub(crate) dimensions: [usize; 4],
    pub spacing: [f64; 3],
    pub(crate) spatial_units: SpatialUnits,
    pub(crate) transform: Option<Transform>,
    pub data_type: DataType,
    pub(crate) endian: ByteOrder,
    pub(crate) scaling: Scaling,
    pub(crate) display_range: Option<(f64, f64)>,
    pub(crate) data: DataFile,
}

impl VolumeMetadata {
    pub(crate) fn read(path: &Path) -> Result<Self> {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if name.ends_with(".nrrd") || name.ends_with(".nhdr") {
            nrrd::read(path)
        } else {
            nifti1::read(path)
        }
    }

    pub fn width(&self) -> usize {
        self.dimensions[0]
    }

    pub fn height(&self) -> usize {
        self.dimensions[1]
    }

    pub fn depth(&self) -> usize {
        self.dimensions[2]
    }

    pub fn timepoints(&self) -> usize {
        self.dimensions[3]
    }

    pub(crate) fn voxels_per_frame(&self) -> usize {
        self.width() * self.height() * self.depth()
    }
}

#[cfg(test)]
mod tests {
    use super::strip_extension;

    #[test]
    fn strips_only_volume_extensions() {
        assert_eq!(strip_extension("ct.nii.gz"), "ct");
        assert_eq!(strip_extension("patient.01.NII"), "patient.01");
        assert_eq!(strip_extension("seg.nrrd"), "seg");
        assert_eq!(strip_extension("notes.txt"), "notes.txt");
    }
}
