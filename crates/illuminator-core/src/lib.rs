//! Illuminator core. Reading, slicing and measuring scan volumes.
//!
//! - [`metadata`] reads a file's header and [`volume`] maps its voxels into memory. Gzipped data
//!   is decompressed into the [`cache`] first.
//! - [`loader`] loads volumes on background threads.
//! - [`geometry`] places voxels in world space, [`display`] holds how a volume is shown,
//!   and [`slicer`] turns a plane into an image.
//! - [`labels`], [`statistics`], [`histogram`] and [`suv`] measure what is in a volume.

pub mod cache;
pub mod colormap;
mod colormap_definitions;
pub mod display;
pub mod error;
mod formats;
pub mod geometry;
pub mod histogram;
pub mod labels;
pub mod loader;
pub mod metadata;
pub mod progress;
mod projection;
pub mod slicer;
pub mod statistics;
pub mod suv;
pub mod time_series;
pub mod volume;
pub mod voxel;

#[cfg(test)]
mod test_fixtures;

pub use error::{Error, Result};
