//! How one volume is shown (cursor, frame, contrast window, colormap, labels).

use std::sync::Arc;

use crate::colormap::Colormap;
use crate::error::{Error, Result};
use crate::geometry::{GeometryOptions, Point3D, SlicePlane, ViewPlane, VolumeGeometry};
use crate::labels;
use crate::loader::LoadedVolume;
use crate::metadata::{SpatialUnits, VolumeMetadata};
use crate::progress::Progress;
use crate::projection;
use crate::slicer::{Coloring, SampledSlice, Slice};
use crate::volume::{ValueRange, Volume};

const LABEL_LIMIT: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq)]
pub struct SampleKey {
    pub(crate) volume: u64,
    pub(crate) geometry: VolumeGeometry,
    pub(crate) plane: SlicePlane,
    pub frame: usize,
    pub(crate) labels: bool,
    pub(crate) mip: bool,
    pub(crate) width: usize,
    pub(crate) height: usize,
}

pub struct SliceJob {
    pub(crate) volume: Arc<Volume>,
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
    pub(crate) minimum_width: f64,
    pub(crate) step: f64,
}

impl IntensityWindow {
    pub(crate) fn from_limits(low: f64, high: f64) -> Result<Self> {
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
        let minimum_width = (span * 1e-6)
            .max(magnitude * f64::EPSILON * 8.0)
            .max(1e-300);
        Ok(Self {
            level: low * 0.5 + high * 0.5,
            width,
            minimum_width,
            step: (width * 0.005).max(minimum_width),
        })
    }

    fn initial(metadata: &VolumeMetadata, range: Option<ValueRange>) -> Result<Self> {
        let (low, high) = if let Some(suggested) = metadata.display_range {
            suggested
        } else if let Some(range) = range {
            (range.minimum, range.maximum)
        } else {
            (0.0, 1.0)
        };
        Self::from_limits(low, high)
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
    unscaled_volume: Arc<Volume>,
    volume: Arc<Volume>,
    scale: f64,
    geometry: VolumeGeometry,
    options: GeometryOptions,
    cursor: Point3D,
    frame: usize,
    range: Option<ValueRange>,
    labels: Vec<i64>,
    label_mode: bool,
    pub window: IntensityWindow,
    pub colormap: Colormap,
    pub transparent_below_window: bool,
    pub outline: bool,
    pub world_link_enabled: bool,
}

impl VolumeDisplay {
    pub fn new(loaded: LoadedVolume) -> Result<Self> {
        let options = GeometryOptions {
            assume_millimeters: true, // units are often left out and implied to be mm
            ..GeometryOptions::default()
        };
        let geometry = loaded.volume.geometry(options)?;
        let window = IntensityWindow::initial(loaded.volume.metadata(), loaded.range)?;
        let unscaled_volume = Arc::new(loaded.volume);
        let mut display = Self {
            volume: Arc::clone(&unscaled_volume),
            unscaled_volume,
            scale: 1.0,
            geometry,
            options,
            cursor: Point3D::default(),
            frame: 0,
            range: loaded.range,
            labels: Vec::new(),
            label_mode: false,
            window,
            colormap: Colormap::default(),
            transparent_below_window: false,
            outline: false,
            world_link_enabled: true,
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

    pub fn range(&self) -> Option<ValueRange> {
        self.range
    }

    pub fn label_mode(&self) -> bool {
        self.label_mode
    }

    pub fn label_count(&self) -> usize {
        self.labels.len()
    }

    /// Labels in the current frame of a label-map.
    pub fn labels(&self) -> &[i64] {
        &self.labels
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
        self.volume.metadata().spatial_units == SpatialUnits::Unknown
    }

    pub fn has_anatomical_transform(&self) -> bool {
        self.volume.metadata().transform.is_some() && self.geometry.units_known
    }

    pub fn world_linked(&self) -> bool {
        self.geometry.anatomical && self.world_link_enabled
    }

    pub fn reset_cursor(&mut self) {
        let metadata = self.volume.metadata();
        let center = Point3D::new(
            ((metadata.width() - 1) / 2) as f64,
            ((metadata.height() - 1) / 2) as f64,
            ((metadata.depth() - 1) / 2) as f64,
        );
        self.cursor = self.geometry.voxel_to_world.transform_point(center);
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

    pub fn set_assume_millimeters(&mut self, assume_mm: bool) -> Result<()> {
        self.set_options(GeometryOptions {
            assume_millimeters: assume_mm,
            ..self.options
        })
    }

    pub fn copy_settings_from(&mut self, previous: &VolumeDisplay) -> Result<()> {
        self.set_scale(previous.scale)?;
        self.window = previous.window;
        self.colormap = previous.colormap;
        self.transparent_below_window = previous.transparent_below_window;
        self.outline = previous.outline;
        self.world_link_enabled = previous.world_link_enabled;
        self.set_options(previous.options)?;
        self.set_cursor(previous.cursor);
        self.set_frame(previous.frame)?;
        self.set_label_mode(previous.label_mode)
    }

    /// Rebuilds the geometry while keeping the cursor on the same voxel
    fn set_options(&mut self, options: GeometryOptions) -> Result<()> {
        let geometry = self.volume.geometry(options)?;
        let voxel = self.geometry.world_to_voxel.transform_point(self.cursor);
        self.cursor = geometry.voxel_to_world.transform_point(voxel);
        self.geometry = geometry;
        self.options = options;
        Ok(())
    }

    pub fn normalized_cursor(&self) -> Point3D {
        let bounds = &self.geometry.cursor_bounds;
        let extent = bounds.extent();
        let mut fraction = Point3D::default();
        for axis in 0..3 {
            fraction[axis] = if extent[axis] > 0.0 {
                (self.cursor[axis] - bounds.minimum[axis]) / extent[axis]
            } else {
                0.5
            };
        }
        fraction
    }

    pub fn set_normalized_cursor(&mut self, fraction: Point3D) {
        if !fraction.is_finite() {
            return;
        }
        let bounds = self.geometry.cursor_bounds;
        let extent = bounds.extent();
        for axis in 0..3 {
            self.cursor[axis] =
                bounds.minimum[axis] + fraction[axis].clamp(0.0, 1.0) * extent[axis];
        }
        self.round_cursor_to_voxel();
    }

    pub(crate) fn slice_axis(&self, axis: ViewPlane) -> (usize, f64) {
        let world_axis = match axis {
            ViewPlane::Axial => 2,
            ViewPlane::Coronal => 1,
            ViewPlane::Sagittal => 0,
        };
        let row = self.geometry.voxel_to_world.matrix[world_axis];
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
        let count =
            ((bounds.maximum[world_axis] - bounds.minimum[world_axis]) / step).round() as usize + 1;
        let index =
            ((self.cursor[world_axis] - bounds.minimum[world_axis]) / step).round() as usize;
        (index.min(count - 1), count)
    }

    pub fn step_slice(&mut self, axis: ViewPlane, steps: f64) -> bool {
        let (world_axis, step) = self.slice_axis(axis);
        let bounds = &self.geometry.cursor_bounds;
        let before = self.cursor;
        self.cursor[world_axis] = (self.cursor[world_axis] + steps * step)
            .clamp(bounds.minimum[world_axis], bounds.maximum[world_axis]);
        self.round_cursor_to_voxel();
        self.cursor != before
    }

    fn round_cursor_to_voxel(&mut self) {
        if self.geometry.anatomical {
            return;
        }
        let mut voxel = self.geometry.world_to_voxel.transform_point(self.cursor);
        for axis in 0..3 {
            let last = (self.geometry.dimensions[axis] - 1) as f64;
            voxel[axis] = (voxel[axis] + 0.5).floor().clamp(0.0, last);
        }
        self.cursor = self.geometry.voxel_to_world.transform_point(voxel);
    }

    pub fn set_frame(&mut self, requested_frame: usize) -> Result<()> {
        let frame = self.volume.clamp_frame(requested_frame);
        if frame == self.frame {
            return Ok(());
        }
        if self.label_mode {
            self.labels =
                labels::find_labels(&self.volume, frame, LABEL_LIMIT, &Progress::default())?;
        }
        self.frame = frame;
        Ok(())
    }

    pub fn set_label_mode(&mut self, enabled: bool) -> Result<()> {
        if enabled && !self.label_mode {
            self.labels =
                labels::find_labels(&self.volume, self.frame, LABEL_LIMIT, &Progress::default())?;
        }
        self.label_mode = enabled;
        Ok(())
    }

    pub fn fit_window_to_range(&mut self) -> Result<()> {
        let range = self
            .volume
            .frame_value_range(self.frame, &Progress::default())?;
        let (low, high) = range.map_or((0.0, 1.0), |range| (range.minimum, range.maximum));
        self.window = IntensityWindow::from_limits(low, high)?;
        self.range = range;
        Ok(())
    }

    pub fn adjust_window(&mut self, width_steps: f64, level_steps: f64) {
        let window = &mut self.window;
        let width = (window.width + width_steps * window.step).max(window.minimum_width);
        let level = window.level - level_steps * window.step;
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
            Arc::clone(&self.unscaled_volume)
        } else {
            Arc::new(self.unscaled_volume.rescaled(scale))
        };
        let (low, high) = self.window_limits();
        self.window = IntensityWindow::from_limits(low * ratio, high * ratio)?;
        self.range = self.range.map(|range| ValueRange {
            minimum: range.minimum * ratio,
            maximum: range.maximum * ratio,
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
            && let Ok(window) = IntensityWindow::from_limits(low, high)
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
                clip_low: self.transparent_below_window,
            }
        }
    }

    pub fn plane(&self, axis: ViewPlane, radiological: bool) -> SlicePlane {
        self.geometry.plane(axis, self.cursor, radiological)
    }

    pub fn slice_plane(&self, axis: ViewPlane, index: usize, radiological: bool) -> SlicePlane {
        let (world_axis, step) = self.slice_axis(axis);
        let mut plane_point = self.cursor;
        plane_point[world_axis] =
            self.geometry.cursor_bounds.minimum[world_axis] + index as f64 * step;
        self.geometry.plane(axis, plane_point, radiological)
    }

    /// What to render for one panel: `plane` at `width` x `height` pixels, as a MIP if `mip`
    pub fn slice_job(&self, plane: SlicePlane, width: usize, height: usize, mip: bool) -> SliceJob {
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

    pub fn select_maximum_along_view(
        &mut self,
        axis: ViewPlane,
        radiological: bool,
        s: f64,
        t: f64,
    ) -> bool {
        if !(0.0..1.0).contains(&s) || !(0.0..1.0).contains(&t) {
            return false;
        }
        let plane = self.plane(axis, radiological);
        let depth = projection::projection_axis(&self.geometry, &plane);
        let voxel = self
            .geometry
            .world_to_voxel
            .transform_point(plane.point(s, t));
        let Some(brightest) = self
            .volume
            .projection(self.frame, depth)
            .maximum_voxel(voxel)
        else {
            return false;
        };
        let brightest = Point3D::from_array(brightest.map(|index| index as f64));
        self.cursor = self.geometry.voxel_to_world.transform_point(brightest);
        true
    }

    /// Moves the cursor to (`s`, `t`) on `plane`.
    /// False if that is outside the plane or the volume.
    pub fn select_point(&mut self, plane: SlicePlane, s: f64, t: f64) -> bool {
        if !(0.0..1.0).contains(&s) || !(0.0..1.0).contains(&t) {
            return false;
        }
        let selected = plane.point(s, t);
        if !self.geometry.anatomical
            && !self
                .geometry
                .contains_voxel(self.geometry.world_to_voxel.transform_point(selected))
        {
            return false;
        }
        self.cursor = selected;
        self.round_cursor_to_voxel();
        true
    }

    pub fn value_at_cursor(&self) -> Option<([usize; 3], f64)> {
        self.value_at_world_point(self.cursor)
    }

    pub fn value_at_world_point(&self, p: Point3D) -> Option<([usize; 3], f64)> {
        let nearest = self.geometry.nearest_voxel(p)?;
        let value = self.volume.sample_value(nearest, self.frame)?;
        Some((nearest, value))
    }

    pub fn orientation_labels(&self, axis: ViewPlane, radiological: bool) -> OrientationLabels {
        let [title, mut left, mut right, top, bottom] = match (self.geometry.anatomical, axis) {
            (true, ViewPlane::Axial) => ["Axial", "L", "R", "A", "P"],
            (true, ViewPlane::Coronal) => ["Coronal", "L", "R", "S", "I"],
            (true, ViewPlane::Sagittal) => ["Sagittal", "P", "A", "S", "I"],
            // voxel axes I, J, K
            (false, ViewPlane::Axial) => ["Native K", "I-", "I+", "J+", "J-"],
            (false, ViewPlane::Coronal) => ["Native J", "I-", "I+", "K+", "K-"],
            (false, ViewPlane::Sagittal) => ["Native I", "J-", "J+", "K+", "K-"],
        };
        if self.geometry.is_mirrored(axis, radiological) {
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

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::test_fixtures::{Fixture, write_and_load};
    use crate::voxel::DataType;

    fn display(directory: &TempDir, fixture: &Fixture) -> VolumeDisplay {
        let volume = write_and_load(directory, fixture, "display.nii");
        let range = volume.frame_value_range(0, &Progress::default()).unwrap();
        VolumeDisplay::new(LoadedVolume {
            load_id: 1,
            volume,
            range,
        })
        .unwrap()
    }

    #[test]
    fn windows() {
        let window = IntensityWindow::from_limits(0.0, 30.0).unwrap();
        assert_eq!((window.level, window.width), (15.0, 30.0));
        let flat = IntensityWindow::from_limits(-5.0, -5.0).unwrap();
        assert_eq!((flat.level, flat.width), (-5.0, 1.0));
        assert!(IntensityWindow::from_limits(-f64::MAX, f64::MAX).is_err());
    }

    #[test]
    fn calibration_overrides_the_measured_range() {
        let directory = tempfile::tempdir().unwrap();
        let display = display(&directory, &Fixture::default());
        let mut metadata = display.volume().metadata().clone();
        let range = Some(ValueRange {
            minimum: 0.0,
            maximum: 47.0,
        });
        assert_eq!(
            IntensityWindow::initial(&metadata, range).unwrap().level,
            23.5
        );
        metadata.display_range = Some((10.0, 20.0));
        assert_eq!(
            IntensityWindow::initial(&metadata, range).unwrap().level,
            15.0
        );
    }

    #[test]
    fn rescaling_follows_values_window_and_range() {
        let directory = tempfile::tempdir().unwrap();
        let mut display = display(&directory, &Fixture::default());
        let (low, high) = display.window_limits();
        display.set_scale(0.5).unwrap();
        assert_eq!(display.value_at_cursor(), Some(([1, 1, 1], 8.5)));
        assert_eq!(display.window_limits(), (low * 0.5, high * 0.5));
        assert_eq!(display.range().unwrap().maximum, 23.5);
        display.set_scale(1.0).unwrap();
        assert_eq!(display.value_at_cursor(), Some(([1, 1, 1], 17.0)));
        assert_eq!(display.window_limits(), (low, high));
        assert!(display.set_scale(0.0).is_err());
    }

    #[test]
    fn starts_on_the_center_voxel() {
        let directory = tempfile::tempdir().unwrap();
        let display = display(&directory, &Fixture::default());
        assert_eq!(display.cursor(), Point3D::new(1.0, 1.0, 1.0));
        assert_eq!(display.value_at_cursor(), Some(([1, 1, 1], 17.0)));
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
        let directory = tempfile::tempdir().unwrap();
        let mut display = display(&directory, &Fixture::default());
        display.set_window_limits(10.0, 30.0);
        assert_eq!(display.window_limits(), (10.0, 30.0));
        display.set_window_limits(30.0, 10.0);
        assert_eq!(display.window_limits(), (10.0, 30.0));
    }

    #[test]
    fn picking_moves_the_cursor() {
        let directory = tempfile::tempdir().unwrap();
        let mut display = display(&directory, &Fixture::default());
        assert!(display.select_point(display.plane(ViewPlane::Axial, false), 0.125, 0.5));
        assert_eq!(display.cursor(), Point3D::new(0.0, 1.0, 1.0));
        assert!(!display.select_point(display.plane(ViewPlane::Axial, false), 1.5, 0.5));
        assert_eq!(display.cursor(), Point3D::new(0.0, 1.0, 1.0));
        assert!(display.select_point(display.plane(ViewPlane::Axial, true), 0.125, 0.5));
        assert_eq!(display.cursor(), Point3D::new(3.0, 1.0, 1.0));
    }

    #[test]
    fn native_view_snaps_to_voxels_and_keeps_the_cursor() {
        let directory = tempfile::tempdir().unwrap();
        let flipped = Fixture {
            sform: [
                [-2.0, 0.0, 0.0, 30.0],
                [0.0, 3.0, 0.0, -20.0],
                [0.0, 0.0, 4.0, 10.0],
            ],
            ..Fixture::default()
        };
        let mut display = display(&directory, &flipped);
        assert!(display.world_linked());
        display.set_native(true).unwrap();
        assert!(!display.geometry().anatomical && !display.world_linked());
        assert_eq!(display.cursor(), Point3D::new(1.0, 1.0, 1.0));
        assert!(display.select_point(display.plane(ViewPlane::Axial, false), 0.3, 0.5));
        assert_eq!(display.cursor(), Point3D::new(1.0, 1.0, 1.0));
    }

    #[test]
    fn proportional_cursor_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let mut display = display(&directory, &Fixture::default());
        assert_eq!(
            display.normalized_cursor(),
            Point3D::new(1.0 / 3.0, 0.5, 1.0 / 3.0)
        );
        display.set_normalized_cursor(Point3D::new(1.0, 1.0, 1.0));
        assert_eq!(display.cursor(), Point3D::new(3.0, 2.0, 3.0));
        assert!(display.geometry().contains_world(display.cursor()));
    }

    #[test]
    fn steps_through_slices() {
        let directory = tempfile::tempdir().unwrap();
        let mut display = display(&directory, &Fixture::default());
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
        let directory = tempfile::tempdir().unwrap();
        let mut display = display(&directory, &Fixture::default());
        let (slice, count) = display.slice_position(ViewPlane::Axial);
        assert_eq!((slice, count), (1, 4));
        let here = display.plane(ViewPlane::Axial, false);
        assert_eq!(display.slice_plane(ViewPlane::Axial, slice, false), here);
        let last = display.slice_plane(ViewPlane::Axial, 3, false);
        assert_eq!(last.origin.z, 3.0);
        assert!(display.select_point(last, 0.125, 0.5));
        assert_eq!(display.cursor(), Point3D::new(0.0, 1.0, 3.0));
        assert_eq!(display.slice_position(ViewPlane::Axial), (3, 4));
    }

    #[test]
    fn frames_and_labels() {
        let directory = tempfile::tempdir().unwrap();
        let four_d = Fixture {
            dimensions: vec![4, 3, 4, 2],
            datatype: DataType::I32,
            value: |i| (i % 3) as f64,
            ..Fixture::default()
        };
        let mut labels = display(&directory, &four_d);
        labels.set_frame(99).unwrap();
        assert_eq!(labels.frame(), 1);
        labels.set_label_mode(true).unwrap();
        assert_eq!(labels.coloring(), Coloring::Labels { outline: false });
        assert_eq!(labels.label_count(), 2);

        let other_directory = tempfile::tempdir().unwrap();
        let fractional = Fixture {
            datatype: DataType::F32,
            value: |i| i as f64 + 0.5,
            ..Fixture::default()
        };
        let mut floats = display(&other_directory, &fractional);
        assert!(floats.set_label_mode(true).is_err());
        assert!(!floats.label_mode());
    }

    #[test]
    fn reloading_keeps_the_settings() {
        let four_d = Fixture {
            dimensions: vec![4, 3, 4, 2],
            datatype: DataType::I32,
            value: |i| (i % 3) as f64,
            ..Fixture::default()
        };
        let directory = tempfile::tempdir().unwrap();
        let mut old = display(&directory, &four_d);
        old.set_frame(1).unwrap();
        old.set_label_mode(true).unwrap();
        old.outline = true;
        old.colormap = Colormap::Viridis;
        old.step_slice(ViewPlane::Axial, 1.0);

        let other_directory = tempfile::tempdir().unwrap();
        let mut new = display(&other_directory, &four_d);
        new.copy_settings_from(&old).unwrap();
        assert_eq!(new.frame(), 1);
        assert_eq!(new.coloring(), Coloring::Labels { outline: true });
        assert_eq!(new.colormap, Colormap::Viridis);
        assert_eq!(new.cursor(), old.cursor());
    }

    #[test]
    fn contrast_follows_the_mouse_and_the_frame() {
        let directory = tempfile::tempdir().unwrap();
        let mut display = display(&directory, &Fixture::default());
        let before = display.window;
        display.adjust_window(10.0, -10.0);
        assert!(display.window.width > before.width && display.window.level > before.level);
        display.adjust_window(-1e9, 0.0);
        assert_eq!(display.window.width, display.window.minimum_width);
        display.fit_window_to_range().unwrap();
        assert_eq!((display.window.level, display.window.width), (23.5, 47.0));
    }

    #[test]
    fn orientation_labels() {
        let directory = tempfile::tempdir().unwrap();
        let display = display(&directory, &Fixture::default());
        let axial = display.orientation_labels(ViewPlane::Axial, false);
        assert_eq!((axial.title, axial.left, axial.top), ("Axial", "L", "A"));
        assert_eq!(display.orientation_labels(ViewPlane::Axial, true).left, "R");
        assert_eq!(
            display.orientation_labels(ViewPlane::Sagittal, true).left,
            "P"
        );
    }
}
