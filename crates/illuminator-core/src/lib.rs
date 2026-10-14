//! Illuminator core. Reading, slicing and measuring scan volumes.
//!
//! - [`meta`] reads a file's header and [`volume`] maps its voxels into memory. Gzipped data
//!   is decompressed into the [`cache`] first.
//! - [`loader`] loads volumes on background threads.
//! - [`geometry`] places voxels in world space, [`display`] holds how a volume is shown,
//!   and [`slicer`] turns a plane into an image.
//! - [`labels`], [`stats`], [`histogram`] and [`suv`] measure what is in a volume.

pub mod cache;
pub mod colormap;
pub mod display;
pub mod error;
pub mod geometry;
pub mod histogram;
pub mod labels;
pub mod loader;
pub mod meta;
mod nifti1;
mod nrrd;
pub mod progress;
mod projection;
pub mod slicer;
pub mod stats;
pub mod suv;
pub mod volume;
pub mod voxel;

#[cfg(test)]
mod test_util;

pub use error::{Error, Result};
