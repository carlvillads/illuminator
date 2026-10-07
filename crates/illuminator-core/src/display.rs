//! display state for one volume

use std::sync::Arc;

use crate::error::{Error, Result};
use crate::geometry::{GeometryOptions, Point3D, SlicePlane, ViewPlane, VolumeGeometry};
use crate::header::{Header, SpatialUnits};
use crate::labels::LabelTable;
use crate::loader::LoadedVolume;
use crate::progress::Progress;
use crate::slicer::{Coloring, Colormap, SampledSlice, Slice};
use crate::volume::{NiiVolume, Range};

const LABEL_LIMIT: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq)]
pub struct SampleKey {
    pub geometry: VolumeGeometry,
    pub plane: SlicePlane,
    pub frame: usize,
    pub labels: bool,
    pub width: usize,
    pub height: usize,
}

pub struct SliceJob {
    pub volume: Arc<NiiVolume>,
    pub key: SampleKey,
}

impl SliceJob {
    pub fn sample(&self) -> SampledSlice {
        let key = &self.key;
        Slice {
            volume: &self.volume,
            geometry: &key.geometry,
            plane: key.plane,
            frame: key.frame,
            labels: key.labels,
        }
        .sample(key.width, key.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntensityWindow {
    pub level: f64,
    pub width: f64,
    pub min_width: f64,
    pub step: f64,
}

impl IntensityWindow {
    pub fn spanning(low: f64, high: f64) -> Result<Self> {
        let span = high - low;
        if !span.is_finite() {
            return Err(Error::invalid("Intensity range cannot be represented"));
        }
        let magnitude = low.abs().max(high.abs());
        let width = if span > 0.0 {
            span
        } else {
            (magnitude * 0.01).max(1.0)
        };
        let min_width = (span * 1e-6)
            .max(magnitude * f64::EPSILON * 8.0)
            .max(1e-300);
        Ok(Self {
            level: low * 0.5 + high * 0.5,
            width,
            min_width,
            step: (width * 0.005).max(min_width),
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
    volume: Arc<NiiVolume>,
    geometry: VolumeGeometry,
    options: GeometryOptions,
    cursor: Point3D,
    frame: usize,
    range: Option<Range>,
    labels: LabelTable,
    label_mode: bool,
    pub window: IntensityWindow,
    pub colormap: Colormap,
    pub clip_low: bool,
    pub outline: bool,
    pub link_world: bool,
}

impl VolumeDisplay {
    pub fn new(loaded: LoadedVolume) -> Result<Self> {
        let options = GeometryOptions::default();
        let geometry = loaded.volume.geometry(options)?;
        let window = IntensityWindow::initial(loaded.volume.header(), loaded.range)?;
        let mut display = Self {
            volume: Arc::new(loaded.volume),
            geometry,
            options,
            cursor: Point3D::default(),
            frame: 0,
            range: loaded.range,
            labels: LabelTable::default(),
            label_mode: false,
            window,
            colormap: Colormap::default(),
            clip_low: false,
            outline: false,
            link_world: true,
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

    pub fn options(&self) -> GeometryOptions {
        self.options
    }

    pub fn cursor(&self) -> Point3D {
        self.cursor
    }

    pub fn frame(&self) -> usize {
        self.frame
    }

    pub fn range(&self) -> Option<Range> {
        self.range
    }

    pub fn label_mode(&self) -> bool {
        self.label_mode
    }

    pub fn label_count(&self) -> usize {
        self.labels.labels().len()
    }

    pub fn name(&self) -> String {
        self.volume
            .path()
            .file_name()
            .unwrap_or(self.volume.path().as_os_str())
            .to_string_lossy()
            .into_owned()
    }

    pub fn units_unknown(&self) -> bool {
        self.volume.header().units == SpatialUnits::Unknown
    }

    pub fn can_be_anatomical(&self) -> bool {
        let h = self.volume.header();
        (h.sform.is_some() || h.qform.is_some()) && self.geometry.units_known
    }

    pub fn world_linked(&self) -> bool {
        self.geometry.anatomical && self.link_world
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

    pub fn set_native(&mut self, native: bool) -> Result<()> {
        self.set_options(GeometryOptions {
            native,
            ..self.options
        })
    }

    pub fn set_assume_mm(&mut self, assume_mm: bool) -> Result<()> {
        self.set_options(GeometryOptions {
            assume_mm,
            ..self.options
        })
    }

    /// Rebuilds the geometry while keeping the cursor on the same voxel
    fn set_options(&mut self, options: GeometryOptions) -> Result<()> {
        let geometry = self.volume.geometry(options)?;
        let voxel = self.geometry.world_to_voxel.point(self.cursor);
        self.cursor = geometry.voxel_to_world.point(voxel);
        self.geometry = geometry;
        self.options = options;
        Ok(())
    }

    pub fn fractional_cursor(&self) -> Point3D {
        let bounds = &self.geometry.cursor_bounds;
        let extent = bounds.extent();
        let mut fraction = Point3D::default();
        for axis in 0..3 {
            fraction[axis] = if extent[axis] > 0.0 {
                (self.cursor[axis] - bounds.min[axis]) / extent[axis]
            } else {
                0.5
            };
        }
        fraction
    }

    pub fn set_fractional_cursor(&mut self, fraction: Point3D) {
        if !fraction.is_finite() {
            return;
        }
        let bounds = self.geometry.cursor_bounds;
        let extent = bounds.extent();
        for axis in 0..3 {
            self.cursor[axis] = bounds.min[axis] + fraction[axis].clamp(0.0, 1.0) * extent[axis];
        }
        self.snap_to_voxel();
    }

    pub fn slice_axis(&self, axis: ViewPlane) -> (usize, f64) {
        let world_axis = match axis {
            ViewPlane::Axial => 2,
            ViewPlane::Coronal => 1,
            ViewPlane::Sagittal => 0,
        };
        let row = self.geometry.voxel_to_world.m[world_axis];
        let step = row[..3]
            .iter()
            .fold(0.0, |most: f64, value| most.max(value.abs()));
        (world_axis, step)
    }

    pub fn slice_position(&self, axis: ViewPlane) -> (usize, usize) {
        let (world_axis, step) = self.slice_axis(axis);
        let bounds = &self.geometry.cursor_bounds;
        if step <= 0.0 {
            return (0, 1);
        }
        let count = ((bounds.max[world_axis] - bounds.min[world_axis]) / step).round() as usize + 1;
        let index = ((self.cursor[world_axis] - bounds.min[world_axis]) / step).round() as usize;
        (index.min(count - 1), count)
    }

    pub fn step_slice(&mut self, axis: ViewPlane, steps: f64) -> bool {
        let (world_axis, step) = self.slice_axis(axis);
        let bounds = &self.geometry.cursor_bounds;
        let before = self.cursor;
        self.cursor[world_axis] = (self.cursor[world_axis] + steps * step)
            .clamp(bounds.min[world_axis], bounds.max[world_axis]);
        self.snap_to_voxel();
        self.cursor != before
    }

    fn snap_to_voxel(&mut self) {
        if self.geometry.anatomical {
            return;
        }
        let mut voxel = self.geometry.world_to_voxel.point(self.cursor);
        for axis in 0..3 {
            let last = (self.geometry.dimensions[axis] - 1) as f64;
            voxel[axis] = (voxel[axis] + 0.5).floor().clamp(0.0, last);
        }
        self.cursor = self.geometry.voxel_to_world.point(voxel);
    }

    pub fn set_frame(&mut self, t: usize) -> Result<()> {
        let frame = self.volume.clamp_frame(t);
        if frame == self.frame {
            return Ok(());
        }
        if self.label_mode {
            self.labels
                .encode(&self.volume, frame, LABEL_LIMIT, &Progress::default())?;
        }
        self.frame = frame;
        Ok(())
    }

    pub fn set_label_mode(&mut self, enabled: bool) -> Result<()> {
        if enabled && !self.label_mode {
            self.labels
                .encode(&self.volume, self.frame, LABEL_LIMIT, &Progress::default())?;
        }
        self.label_mode = enabled;
        Ok(())
    }

    pub fn auto_window(&mut self) -> Result<()> {
        let range = self.volume.frame_range(self.frame, &Progress::default())?;
        let (low, high) = range.map_or((0.0, 1.0), |range| (range.min, range.max));
        self.window = IntensityWindow::spanning(low, high)?;
        self.range = range;
        Ok(())
    }

    pub fn drag_window(&mut self, dx: f64, dy: f64) {
        let window = &mut self.window;
        let width = (window.width + dx * window.step).max(window.min_width);
        let level = window.level - dy * window.step;
        if width.is_finite() && level.is_finite() {
            window.width = width;
            window.level = level;
        }
    }

    /// Values at the top and bottom of the window
    pub fn window_limits(&self) -> (f64, f64) {
        let half = self.window.width / 2.0;
        (self.window.level - half, self.window.level + half)
    }

    pub fn set_window_limits(&mut self, low: f64, high: f64) {
        if low < high
            && let Ok(window) = IntensityWindow::spanning(low, high)
        {
            self.window = window;
        }
    }

    pub fn shared_volume(&self) -> Arc<NiiVolume> {
        Arc::clone(&self.volume)
    }

    pub fn coloring(&self) -> Coloring {
        if self.label_mode {
            Coloring::Labels {
                outline: self.outline,
            }
        } else {
            Coloring::Intensity {
                level: self.window.level,
                width: self.window.width,
                colormap: self.colormap,
                clip_low: self.clip_low,
            }
        }
    }

    pub fn plane(&self, axis: ViewPlane, radiological: bool) -> SlicePlane {
        self.geometry.plane(axis, self.cursor, radiological)
    }

    /// What to render for one panel of w x h pixels
    pub fn job(
        &self,
        axis: ViewPlane,
        radiological: bool,
        width: usize,
        height: usize,
    ) -> SliceJob {
        self.job_on(self.plane(axis, radiological), width, height)
    }

    pub fn job_on(&self, plane: SlicePlane, width: usize, height: usize) -> SliceJob {
        SliceJob {
            volume: Arc::clone(&self.volume),
            key: SampleKey {
                geometry: self.geometry.clone(),
                plane,
                frame: self.frame,
                labels: self.label_mode,
                width,
                height,
            },
        }
    }

    pub fn pick(&mut self, axis: ViewPlane, radiological: bool, s: f64, t: f64) -> bool {
        if !(0.0..1.0).contains(&s) || !(0.0..1.0).contains(&t) {
            return false;
        }
        let selected = self.plane(axis, radiological).point(s, t);
        if !self.geometry.anatomical
            && !self
                .geometry
                .contains_voxel(self.geometry.world_to_voxel.point(selected))
        {
            return false;
        }
        self.cursor = selected;
        self.snap_to_voxel();
        true
    }

    pub fn cursor_sample(&self) -> Option<([usize; 3], f64)> {
        self.sample_world(self.cursor)
    }

    pub fn sample_world(&self, p: Point3D) -> Option<([usize; 3], f64)> {
        let voxel = self.geometry.world_to_voxel.point(p);
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
    use crate::test_util::Fixture;
    use crate::voxel::DataType;

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
        assert_eq!(display.cursor(), Point3D::new(1.0, 1.0, 1.0));
        assert_eq!(display.cursor_sample(), Some(([1, 1, 1], 17.0)));
        assert_eq!(
            display.coloring(),
            Coloring::Intensity {
                level: 23.5,
                width: 47.0,
                colormap: Colormap::Grayscale,
                clip_low: false,
            }
        );
    }

    #[test]
    fn sets_the_window_by_its_limits() {
        let dir = tempfile::tempdir().unwrap();
        let mut display = display(&dir, &Fixture::default());
        display.set_window_limits(10.0, 30.0);
        assert_eq!(display.window_limits(), (10.0, 30.0));
        display.set_window_limits(30.0, 10.0);
        assert_eq!(display.window_limits(), (10.0, 30.0));
    }

    #[test]
    fn picking_moves_the_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let mut display = display(&dir, &Fixture::default());
        assert!(display.pick(ViewPlane::Axial, false, 0.125, 0.5));
        assert_eq!(display.cursor(), Point3D::new(0.0, 1.0, 1.0));
        assert!(!display.pick(ViewPlane::Axial, false, 1.5, 0.5));
        assert_eq!(display.cursor(), Point3D::new(0.0, 1.0, 1.0));
        assert!(display.pick(ViewPlane::Axial, true, 0.125, 0.5));
        assert_eq!(display.cursor(), Point3D::new(3.0, 1.0, 1.0));
    }

    #[test]
    fn native_view_snaps_to_voxels_and_keeps_the_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let flipped = Fixture {
            sform: [
                [-2.0, 0.0, 0.0, 30.0],
                [0.0, 3.0, 0.0, -20.0],
                [0.0, 0.0, 4.0, 10.0],
            ],
            ..Fixture::default()
        };
        let mut display = display(&dir, &flipped);
        assert!(display.world_linked());
        display.set_native(true).unwrap();
        assert!(!display.geometry().anatomical && !display.world_linked());
        assert_eq!(display.cursor(), Point3D::new(1.0, 1.0, 1.0));
        assert!(display.pick(ViewPlane::Axial, false, 0.3, 0.5));
        assert_eq!(display.cursor(), Point3D::new(1.0, 1.0, 1.0));
    }

    #[test]
    fn proportional_cursor_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut display = display(&dir, &Fixture::default());
        assert_eq!(
            display.fractional_cursor(),
            Point3D::new(1.0 / 3.0, 0.5, 1.0 / 3.0)
        );
        display.set_fractional_cursor(Point3D::new(1.0, 1.0, 1.0));
        assert_eq!(display.cursor(), Point3D::new(3.0, 2.0, 3.0));
        assert!(display.geometry().contains_world(display.cursor()));
    }

    #[test]
    fn steps_through_slices() {
        let dir = tempfile::tempdir().unwrap();
        let mut display = display(&dir, &Fixture::default());
        assert!(display.step_slice(ViewPlane::Axial, 1.0));
        assert_eq!(display.cursor().z, 2.0);
        display.step_slice(ViewPlane::Axial, 10.0);
        assert_eq!(display.cursor().z, 3.0);
        assert!(!display.step_slice(ViewPlane::Axial, 1.0));
        assert!(display.step_slice(ViewPlane::Sagittal, -1.0));
        assert_eq!(display.cursor().x, 0.0);
    }

    #[test]
    fn frames_and_labels() {
        let dir = tempfile::tempdir().unwrap();
        let four_d = Fixture {
            dims: vec![4, 3, 4, 2],
            datatype: DataType::I32,
            value: |i| (i % 3) as f64,
            ..Fixture::default()
        };
        let mut labels = display(&dir, &four_d);
        labels.set_frame(99).unwrap();
        assert_eq!(labels.frame(), 1);
        labels.set_label_mode(true).unwrap();
        assert_eq!(labels.coloring(), Coloring::Labels { outline: false });
        assert_eq!(labels.label_count(), 3);

        let other_dir = tempfile::tempdir().unwrap();
        let fractional = Fixture {
            datatype: DataType::F32,
            value: |i| i as f64 + 0.5,
            ..Fixture::default()
        };
        let mut floats = display(&other_dir, &fractional);
        assert!(floats.set_label_mode(true).is_err());
        assert!(!floats.label_mode());
    }

    #[test]
    fn contrast_follows_the_mouse_and_the_frame() {
        let dir = tempfile::tempdir().unwrap();
        let mut display = display(&dir, &Fixture::default());
        let before = display.window;
        display.drag_window(10.0, -10.0);
        assert!(display.window.width > before.width && display.window.level > before.level);
        display.drag_window(-1e9, 0.0);
        assert_eq!(display.window.width, display.window.min_width);
        display.auto_window().unwrap();
        assert_eq!((display.window.level, display.window.width), (23.5, 47.0));
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
