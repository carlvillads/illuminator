//! Illuminator viewer core

pub mod cache;
pub mod display;
pub mod error;
pub mod geometry;
pub mod histogram;
pub mod labels;
pub mod loader;
pub mod meta;
pub mod nifti1;
pub mod nrrd;
pub mod progress;
pub mod slicer;
pub mod volume;
pub mod voxel;

#[cfg(test)]
mod test_util;

pub use error::{Error, Result};
