//! display state for one volume

use std::sync::Arc;

use crate::error::{Error, Result};
use crate::geometry::{GeometryOptions, Point3D, SlicePlane, ViewPlane, VolumeGeometry};
use crate::labels;
use crate::loader::LoadedVolume;
use crate::meta::{SpatialUnits, VolumeMeta};
use crate::progress::Progress;
use crate::projection;
use crate::slicer::{Coloring, Colormap, SampledSlice, Slice};
use crate::volume::{Range, Volume};

const LABEL_LIMIT: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq)]
pub struct SampleKey {
    pub volume: u64,
    pub geometry: VolumeGeometry,
    pub plane: SlicePlane,
    pub frame: usize,
    pub labels: bool,
    pub mip: bool,
    pub width: usize,
    pub height: usize,
}

pub struct SliceJob {
    pub volume: Arc<Volume>,
    pub key: SampleKey,
}

impl SliceJob {
    pub fn sample(&self) -> SampledSlice {
        let key = &self.key;
        let slice = Slice {
            volume: &self.volume,
            geometry: &key.geometry,
            plane: key.plane,
            frame: key.frame,
            labels: key.labels,
        };
        if key.mip {
            slice.sample_mip(key.width, key.height)
        } else {
            slice.sample(key.width, key.height)
        }
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

    fn initial(meta: &VolumeMeta, range: Option<Range>) -> Result<Self> {
        let (low, high) = if let Some(suggested) = meta.display_range {
            suggested
        } else if let Some(range) = range {
            (range.min, range.max)
        } else {
            (0.0, 1.0)
        };
        Self::spanning(low, high)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrientationLabels {
    pub title: &'static str,
    pub left: &'static str,
    pub right: &'static str,
    pub top: &'static str,
    pub bottom: &'static str,
}

pub struct VolumeDisplay {
    raw: Arc<Volume>,
    volume: Arc<Volume>,
    scale: f64,
    geometry: VolumeGeometry,
    options: GeometryOptions,
    cursor: Point3D,
    frame: usize,
    range: Option<Range>,
    label_count: usize,
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
        let window = IntensityWindow::initial(loaded.volume.meta(), loaded.range)?;
        let raw = Arc::new(loaded.volume);
        let mut display = Self {
            volume: Arc::clone(&raw),
            raw,
            scale: 1.0,
            geometry,
            options,
            cursor: Point3D::default(),
            frame: 0,
            range: loaded.range,
            label_count: 0,
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

    pub fn volume(&self) -> &Volume {
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
        self.label_count
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
        self.volume.meta().units == SpatialUnits::Unknown
    }

    pub fn can_be_anatomical(&self) -> bool {
        self.volume.meta().transform.is_some() && self.geometry.units_known
    }

    pub fn world_linked(&self) -> bool {
        self.geometry.anatomical && self.link_world
    }

    pub fn reset_cursor(&mut self) {
        let meta = self.volume.meta();
        let center = Point3D::new(
            ((meta.width() - 1) / 2) as f64,
            ((meta.height() - 1) / 2) as f64,
            ((meta.depth() - 1) / 2) as f64,
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

    pub fn keep_settings_of(&mut self, old: &VolumeDisplay) -> Result<()> {
        self.set_scale(old.scale)?;
        self.window = old.window;
        self.colormap = old.colormap;
        self.clip_low = old.clip_low;
        self.outline = old.outline;
        self.link_world = old.link_world;
        self.set_options(old.options)?;
        self.set_cursor(old.cursor);
        self.set_frame(old.frame)?;
        self.set_label_mode(old.label_mode)
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
            self.label_count =
                labels::count_labels(&self.volume, frame, LABEL_LIMIT, &Progress::default())?;
        }
        self.frame = frame;
        Ok(())
    }

    pub fn set_label_mode(&mut self, enabled: bool) -> Result<()> {
        if enabled && !self.label_mode {
            self.label_count =
                labels::count_labels(&self.volume, self.frame, LABEL_LIMIT, &Progress::default())?;
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

    pub fn scale(&self) -> f64 {
        self.scale
    }

    pub fn set_scale(&mut self, scale: f64) -> Result<()> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(Error::invalid("The scale must be a positive number"));
        }
        if self.label_mode {
            return Err(Error::invalid("Label maps keep their values"));
        }
        let ratio = scale / self.scale;
        self.volume = if scale == 1.0 {
            Arc::clone(&self.raw)
        } else {
            Arc::new(self.raw.rescaled(scale))
        };
        let (low, high) = self.window_limits();
        self.window = IntensityWindow::spanning(low * ratio, high * ratio)?;
        self.range = self.range.map(|range| Range {
            min: range.min * ratio,
            max: range.max * ratio,
        });
        self.scale = scale;
        Ok(())
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

    pub fn shared_volume(&self) -> Arc<Volume> {
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

    pub fn slice_plane(&self, axis: ViewPlane, index: usize, radiological: bool) -> SlicePlane {
        let (world_axis, step) = self.slice_axis(axis);
        let mut through = self.cursor;
        through[world_axis] = self.geometry.cursor_bounds.min[world_axis] + index as f64 * step;
        self.geometry.plane(axis, through, radiological)
    }

    /// What to render for one panel: `plane` at `width` x `height` pixels, as a MIP if `mip`
    pub fn job(&self, plane: SlicePlane, width: usize, height: usize, mip: bool) -> SliceJob {
        SliceJob {
            volume: Arc::clone(&self.volume),
            key: SampleKey {
                volume: self.volume.id(),
                geometry: self.geometry.clone(),
                plane,
                frame: self.frame,
                labels: self.label_mode,
                mip,
                width,
                height,
            },
        }
    }

    pub fn pick_brightest(&mut self, axis: ViewPlane, radiological: bool, s: f64, t: f64) -> bool {
        if !(0.0..1.0).contains(&s) || !(0.0..1.0).contains(&t) {
            return false;
        }
        let plane = self.plane(axis, radiological);
        let depth = projection::depth_axis(&self.geometry, &plane);
        let voxel = self.geometry.world_to_voxel.point(plane.point(s, t));
        let Some(brightest) = self.volume.projection(self.frame, depth).brightest(voxel) else {
            return false;
        };
        let brightest = Point3D::from_array(brightest.map(|index| index as f64));
        self.cursor = self.geometry.voxel_to_world.point(brightest);
        true
    }

    pub fn pick(&mut self, axis: ViewPlane, radiological: bool, s: f64, t: f64) -> bool {
        self.pick_on(self.plane(axis, radiological), s, t)
    }

    pub fn pick_on(&mut self, plane: SlicePlane, s: f64, t: f64) -> bool {
        if !(0.0..1.0).contains(&s) || !(0.0..1.0).contains(&t) {
            return false;
        }
        let selected = plane.point(s, t);
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
        let nearest = self.geometry.nearest_voxel(p)?;
        let value = self.volume.sample_value(nearest, self.frame)?;
        Some((nearest, value))
    }

    pub fn orientation_labels(&self, axis: ViewPlane, radiological: bool) -> OrientationLabels {
        if !self.geometry.anatomical {
            return native_labels(axis);
        }
        let (title, mut left, mut right, top, bottom) = match axis {
            ViewPlane::Axial => ("Axial", "L", "R", "A", "P"),
            ViewPlane::Coronal => ("Coronal", "L", "R", "S", "I"),
            ViewPlane::Sagittal => ("Sagittal", "P", "A", "S", "I"),
        };
        if radiological && axis != ViewPlane::Sagittal {
            (left, right) = (right, left);
        }
        OrientationLabels {
            title,
            left,
            right,
            top,
            bottom,
        }
    }
}

/// Labels for images without an orientation. Uses voxel axes (I, J, K)
fn native_labels(axis: ViewPlane) -> OrientationLabels {
    let (title, left, right, top, bottom) = match axis {
        ViewPlane::Axial => ("Native K", "I-", "I+", "J+", "J-"),
        ViewPlane::Coronal => ("Native J", "I-", "I+", "K+", "K-"),
        ViewPlane::Sagittal => ("Native I", "J-", "J+", "K+", "K-"),
    };
    OrientationLabels {
        title,
        left,
        right,
        top,
        bottom,
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::test_util::{Fixture, load};
    use crate::voxel::DataType;

    fn display(dir: &TempDir, fixture: &Fixture) -> VolumeDisplay {
        let volume = load(dir, fixture, "display.nii");
        let range = volume.frame_range(0, &Progress::default()).unwrap();
        VolumeDisplay::new(LoadedVolume {
            load_id: 1,
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
        let mut meta = display.volume().meta().clone();
        let range = Some(Range {
            min: 0.0,
            max: 47.0,
        });
        assert_eq!(IntensityWindow::initial(&meta, range).unwrap().level, 23.5);
        meta.display_range = Some((10.0, 20.0));
        assert_eq!(IntensityWindow::initial(&meta, range).unwrap().level, 15.0);
    }

    #[test]
    fn rescaling_follows_values_window_and_range() {
        let dir = tempfile::tempdir().unwrap();
        let mut display = display(&dir, &Fixture::default());
        let (low, high) = display.window_limits();
        display.set_scale(0.5).unwrap();
        assert_eq!(display.cursor_sample(), Some(([1, 1, 1], 8.5)));
        assert_eq!(display.window_limits(), (low * 0.5, high * 0.5));
        assert_eq!(display.range().unwrap().max, 23.5);
        display.set_scale(1.0).unwrap();
        assert_eq!(display.cursor_sample(), Some(([1, 1, 1], 17.0)));
        assert_eq!(display.window_limits(), (low, high));
        assert!(display.set_scale(0.0).is_err());
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
    fn picks_on_other_slices() {
        let dir = tempfile::tempdir().unwrap();
        let mut display = display(&dir, &Fixture::default());
        let (slice, count) = display.slice_position(ViewPlane::Axial);
        assert_eq!((slice, count), (1, 4));
        let here = display.plane(ViewPlane::Axial, false);
        assert_eq!(display.slice_plane(ViewPlane::Axial, slice, false), here);
        let last = display.slice_plane(ViewPlane::Axial, 3, false);
        assert_eq!(last.origin.z, 3.0);
        assert!(display.pick_on(last, 0.125, 0.5));
        assert_eq!(display.cursor(), Point3D::new(0.0, 1.0, 3.0));
        assert_eq!(display.slice_position(ViewPlane::Axial), (3, 4));
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
        assert_eq!(labels.label_count(), 2);

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
    fn reloading_keeps_the_settings() {
        let four_d = Fixture {
            dims: vec![4, 3, 4, 2],
            datatype: DataType::I32,
            value: |i| (i % 3) as f64,
            ..Fixture::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let mut old = display(&dir, &four_d);
        old.set_frame(1).unwrap();
        old.set_label_mode(true).unwrap();
        old.outline = true;
        old.colormap = Colormap::Viridis;
        old.step_slice(ViewPlane::Axial, 1.0);

        let other_dir = tempfile::tempdir().unwrap();
        let mut new = display(&other_dir, &four_d);
        new.keep_settings_of(&old).unwrap();
        assert_eq!(new.frame(), 1);
        assert_eq!(new.coloring(), Coloring::Labels { outline: true });
        assert_eq!(new.colormap, Colormap::Viridis);
        assert_eq!(new.cursor(), old.cursor());
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
    fn orientation_labels() {
        let dir = tempfile::tempdir().unwrap();
        let display = display(&dir, &Fixture::default());
        let axial = display.orientation_labels(ViewPlane::Axial, false);
        assert_eq!((axial.title, axial.left, axial.top), ("Axial", "L", "A"));
        assert_eq!(display.orientation_labels(ViewPlane::Axial, true).left, "R");
        assert_eq!(
            display.orientation_labels(ViewPlane::Sagittal, true).left,
            "P"
        );
    }
}
