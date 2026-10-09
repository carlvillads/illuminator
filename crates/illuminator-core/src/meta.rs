use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::voxel::{DataType, Endian, Scaling};
use crate::{nifti1, nrrd};

pub type Mat3x4 = [[f64; 4]; 3];

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
pub enum SpatialUnits {
    Unknown,
    Meter,
    Millimeter,
    Micron,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transform {
    pub name: &'static str,
    pub matrix: Mat3x4,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DataFile {
    pub path: PathBuf,
    pub start: u64,
    pub offset: u64,
    pub bytes: usize,
    pub gzip: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VolumeMeta {
    pub format: Format,
    pub dims: [usize; 4],
    pub spacing: [f64; 3],
    pub units: SpatialUnits,
    pub transform: Option<Transform>,
    pub datatype: DataType,
    pub endian: Endian,
    pub scaling: Scaling,
    pub display_range: Option<(f64, f64)>,
    pub data: DataFile,
}

impl VolumeMeta {
    pub fn read(path: &Path) -> Result<Self> {
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

    pub fn frame_voxels(&self) -> usize {
        self.width() * self.height() * self.depth()
    }
}
