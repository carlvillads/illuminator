//! display state for one volume

use crate::error::{Error, Result};
use crate::geometry::{GeometryOptions, Point3D, SlicePlane, ViewPlane, VolumeGeometry};
use crate::header::Header;
use crate::loader::LoadedVolume;
use crate::slicer::{Coloring, Colormap, Slice};
use crate::volume::{NiiVolume, Range};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntensityWindow {
    pub level: f64,
    pub width: f64,
    pub min_width: f64,
}

impl IntensityWindow {
    pub fn spanning(low: f64, high: f64) -> Result<Self> {
        let span = high - low;
        if !span.is_finite() {
            return Err(Error::invalid("Intensity range could not be represented"));
        }
        let magnitude = low.abs().max(high.abs());
        Ok(Self {
            level: low * 0.5 + high * 0.5,
            width: if span > 0.0 {
                span
            } else {
                (magnitude * 0.01).max(1.0)
            },
            min_width: (span * 1e-6)
                .max(magnitude * f64::EPSILON * 8.0)
                .max(1e-300),
        })
    }

    fn initial(header: &Header, range: Option<Range>) -> Result<Self> {
        let (low, high) = if header.cal_max > header.cal_min {
            (f64::from(header.cal_min), f64::from(header.cal_max))
        } else if let Some(range) = range {
            (range.min, range.max)
        } else {
            (0.0, 1.0)
        };
        Self::spanning(low, high)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelLabels {
    pub title: &'static str,
    pub left: &'static str,
    pub right: &'static str,
    pub top: &'static str,
    pub bottom: &'static str,
}

pub struct VolumeDisplay {
    volume: NiiVolume,
    geometry: VolumeGeometry,
    cursor: Point3D,
    frame: usize,
    pub window: IntensityWindow,
    pub colormap: Colormap,
    pub label_mode: bool,
}

impl VolumeDisplay {
    pub fn new(loaded: LoadedVolume) -> Result<Self> {
        let geometry = loaded.volume.geometry(GeometryOptions::default())?;
        let window = IntensityWindow::initial(loaded.volume.header(), loaded.range)?;
        let mut display = Self {
            volume: loaded.volume,
            geometry,
            cursor: Point3D::default(),
            frame: 0,
            window,
            colormap: Colormap::default(),
            label_mode: false,
        };
        display.reset_cursor();
        Ok(display)
    }

    pub fn volume(&self) -> &NiiVolume {
        &self.volume
    }

    pub fn geometry(&self) -> &VolumeGeometry {
        &self.geometry
    }

    pub fn cursor(&self) -> Point3D {
        self.cursor
    }

    pub fn frame(&self) -> usize {
        self.frame
    }

    pub fn name(&self) -> String {
        self.volume
            .path()
            .file_name()
            .unwrap_or(self.volume.path().as_os_str())
            .to_string_lossy()
            .into_owned()
    }

    pub fn reset_cursor(&mut self) {
        let h = self.volume.header();
        let center = Point3D::new(
            ((h.width() - 1) / 2) as f64,
            ((h.height() - 1) / 2) as f64,
            ((h.depth() - 1) / 2) as f64,
        );
        self.cursor = self.geometry.voxel_to_world.point(center);
    }

    pub fn set_cursor(&mut self, p: Point3D) {
        if p.is_finite() {
            self.cursor = p;
        }
    }

    /// Current coloring
    pub fn coloring(&self) -> Coloring {
        if self.label_mode {
            Coloring::Labels
        } else {
            Coloring::Intensity {
                level: self.window.level,
                width: self.window.width,
                colormap: self.colormap,
            }
        }
    }

    pub fn plane(&self, axis: ViewPlane, radiological: bool) -> SlicePlane {
        self.geometry.plane(axis, self.cursor, radiological)
    }

    pub fn slice(&self, axis: ViewPlane, radiological: bool, crosshair: bool) -> Slice<'_> {
        Slice {
            volume: &self.volume,
            geometry: &self.geometry,
            plane: self.plane(axis, radiological),
            frame: self.frame,
            coloring: self.coloring(),
            crosshair: crosshair.then_some(self.cursor()),
        }
    }

    pub fn pick(&mut self, axis: ViewPlane, radiological: bool, s: f64, t: f64) -> bool {
        if !(0.0..1.0).contains(&s) || !(0.0..1.0).contains(&t) {
            return false;
        }
        let mut selected = self.plane(axis, radiological).point(s, t);
        if !self.geometry.anatomical {
            let mut voxel = self.geometry.world_to_voxel.point(selected);
            if !self.geometry.contains_voxel(voxel) {
                return false;
            }
            for axis in 0..3 {
                voxel[axis] = (voxel[axis] + 0.5).floor();
            }
            selected = self.geometry.voxel_to_world.point(voxel);
        }
        self.cursor = selected;
        true
    }

    pub fn cursor_sample(&self) -> Option<([usize; 3], f64)> {
        let voxel = self.geometry.world_to_voxel.point(self.cursor);
        if !self.geometry.contains_voxel(voxel) {
            return None;
        }
        let nearest = [0, 1, 2].map(|axis| (voxel[axis] + 0.5).floor() as usize);
        let value = self.volume.sample_value(nearest, self.frame)?;
        Some((nearest, value))
    }

    pub fn labels(&self, axis: ViewPlane, radiological: bool) -> PanelLabels {
        let sagittal = axis == ViewPlane::Sagittal;
        let axial = axis == ViewPlane::Axial;
        if self.geometry.anatomical {
            PanelLabels {
                title: match axis {
                    ViewPlane::Axial => "Axial",
                    ViewPlane::Coronal => "Coronal",
                    ViewPlane::Sagittal => "Sagittal",
                },
                left: if sagittal {
                    "P"
                } else if radiological {
                    "R"
                } else {
                    "L"
                },
                right: if sagittal {
                    "A"
                } else if radiological {
                    "L"
                } else {
                    "R"
                },
                top: if axial { "A" } else { "S" },
                bottom: if axial { "P" } else { "I" },
            }
        } else {
            PanelLabels {
                title: match axis {
                    ViewPlane::Axial => "Native K",
                    ViewPlane::Coronal => "Native J",
                    ViewPlane::Sagittal => "Native I",
                },
                left: if sagittal { "J-" } else { "I-" },
                right: if sagittal { "J+" } else { "I+" },
                top: if axial { "J+" } else { "K+" },
                bottom: if axial { "J-" } else { "K-" },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::cache::VolumeCache;
    use crate::progress::Progress;
    use crate::test_util::Fixture;

    fn display(dir: &TempDir, fixture: &Fixture) -> VolumeDisplay {
        let path = dir.path().join("display.nii");
        fixture.write(&path);
        let cache = VolumeCache::new(dir.path().join("cache"), 1 << 20).unwrap();
        let volume = NiiVolume::load(&path, &cache, &Progress::default()).unwrap();
        let range = volume.frame_range(0, &Progress::default()).unwrap();
        VolumeDisplay::new(LoadedVolume {
            id: 1,
            volume,
            range,
        })
        .unwrap()
    }

    #[test]
    fn windows() {
        let window = IntensityWindow::spanning(0.0, 30.0).unwrap();
        assert_eq!((window.level, window.width), (15.0, 30.0));
        // A flat image gets a small window around its value (C++: "Constant volume").
        let flat = IntensityWindow::spanning(-5.0, -5.0).unwrap();
        assert_eq!((flat.level, flat.width), (-5.0, 1.0));
        assert!(IntensityWindow::spanning(-f64::MAX, f64::MAX).is_err());
    }

    #[test]
    fn calibration_overrides_the_measured_range() {
        let dir = tempfile::tempdir().unwrap();
        let display = display(&dir, &Fixture::default());
        let mut header = display.volume().header().clone();
        let range = Some(Range {
            min: 0.0,
            max: 47.0,
        });
        assert_eq!(
            IntensityWindow::initial(&header, range).unwrap().level,
            23.5
        );
        header.cal_min = 10.0;
        header.cal_max = 20.0;
        assert_eq!(
            IntensityWindow::initial(&header, range).unwrap().level,
            15.0
        );
    }

    #[test]
    fn starts_on_the_center_voxel() {
        let dir = tempfile::tempdir().unwrap();
        let display = display(&dir, &Fixture::default());
        // (4 - 1) / 2, (3 - 1) / 2, (4 - 1) / 2 with integer division, like C++.
        assert_eq!(display.cursor(), Point3D::new(1.0, 1.0, 1.0));
        // Voxel (1, 1, 1) is number 1 + 4 * (1 + 3 * 1) = 17.
        assert_eq!(display.cursor_sample(), Some(([1, 1, 1], 17.0)));
        assert_eq!(
            display.coloring(),
            Coloring::Intensity {
                level: 23.5,
                width: 47.0,
                colormap: Colormap::Grayscale,
            }
        );
    }

    #[test]
    fn picking_moves_the_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let mut display = display(&dir, &Fixture::default());
        // The axial panel spans x from -0.5 to 3.5 and y from -0.5 to 2.5.
        assert!(display.pick(ViewPlane::Axial, false, 0.125, 0.5));
        assert_eq!(display.cursor(), Point3D::new(0.0, 1.0, 1.0));
        // Outside the image: nothing changes.
        assert!(!display.pick(ViewPlane::Axial, false, 1.5, 0.5));
        assert_eq!(display.cursor(), Point3D::new(0.0, 1.0, 1.0));
        // In radiological view the same click lands on the other side.
        assert!(display.pick(ViewPlane::Axial, true, 0.125, 0.5));
        assert_eq!(display.cursor(), Point3D::new(3.0, 1.0, 1.0));
    }

    #[test]
    fn panel_labels() {
        let dir = tempfile::tempdir().unwrap();
        let display = display(&dir, &Fixture::default());
        let axial = display.labels(ViewPlane::Axial, false);
        assert_eq!((axial.title, axial.left, axial.top), ("Axial", "L", "A"));
        assert_eq!(display.labels(ViewPlane::Axial, true).left, "R");
        assert_eq!(display.labels(ViewPlane::Sagittal, true).left, "P");
    }
}
