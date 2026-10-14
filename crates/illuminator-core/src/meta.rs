//! meta.rs
//!
//! A volume header in a single form for every file format and picking the correct reader for each file.
//!

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::voxel::{DataType, Endian, Scaling};
use crate::{nifti1, nrrd};

pub(crate) type Mat3x4 = [[f64; 4]; 3];

/// The file extensions illuminator supports
const EXTENSIONS: [&str; 8] = [
    ".nii.gz", ".nii", ".nrrd", ".nhdr", ".hdr.gz", ".hdr", ".img.gz", ".img",
];

/// Removes file extension from a string.
pub fn strip_extension(name: &str) -> &str {
    let lower = name.to_ascii_lowercase();
    EXTENSIONS
        .iter()
        .find(|extension| lower.ends_with(*extension))
        .map_or(name, |extension| &name[..name.len() - extension.len()])
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
    Micron,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Transform {
    pub(crate) name: &'static str,
    pub(crate) matrix: Mat3x4,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DataFile {
    pub(crate) path: PathBuf,
    pub(crate) start: u64,
    pub(crate) offset: u64,
    pub(crate) bytes: usize,
    pub(crate) gzip: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VolumeMeta {
    pub format: Format,
    pub(crate) dims: [usize; 4],
    pub spacing: [f64; 3],
    pub(crate) units: SpatialUnits,
    pub(crate) transform: Option<Transform>,
    pub datatype: DataType,
    pub(crate) endian: Endian,
    pub(crate) scaling: Scaling,
    pub(crate) display_range: Option<(f64, f64)>,
    pub(crate) data: DataFile,
}

impl VolumeMeta {
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
        self.dims[0]
    }

    pub fn height(&self) -> usize {
        self.dims[1]
    }

    pub fn depth(&self) -> usize {
        self.dims[2]
    }

    pub fn timepoints(&self) -> usize {
        self.dims[3]
    }

    pub(crate) fn frame_voxels(&self) -> usize {
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
