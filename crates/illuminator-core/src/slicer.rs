//! Samples a volume along a plane and colors the samples into an RGBA image.

use ::rayon::prelude::*;
use std::marker::PhantomData;

use crate::colormap::{ColorTable, Colormap};
use crate::geometry::{Point3D, SlicePlane, VolumeGeometry, aspect_scale};
use crate::labels::{label_color, label_id};
use crate::projection;
use crate::volume::Volume;
use crate::voxel::{ByteOrder, Scalar, dispatch_data_type};

const INVALID_LABEL_COLOR: [u8; 4] = [255, 0, 255, 255];
const OUTLINE_WIDTH: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Coloring {
    Intensity {
        level: f64,
        width: f64,
        colormap: Colormap,
        /// Values below the window are transparent
        clip_low: bool,
    },
    Labels {
        /// Only outlines
        outline: bool,
    },
}

impl Coloring {
    /// Where `value` falls on the colormap: `0` at the bottom of the window, `1` at the top.
    /// `None` if the value is drawn transparent or if this coloring is for labels.
    pub fn window_position(self, value: f64) -> Option<f64> {
        let Self::Intensity {
            level,
            width,
            clip_low,
            ..
        } = self
        else {
            return None;
        };
        let t = (value - level) / width + 0.5;
        (!value.is_nan() && !(clip_low && t <= 0.0)).then_some(t)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SliceImage {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Copy)]
pub(crate) struct Slice<'a> {
    pub(crate) volume: &'a Volume,
    pub(crate) geometry: &'a VolumeGeometry,
    pub(crate) plane: SlicePlane,
    pub(crate) frame: usize,
    pub(crate) labels: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SampledSlice {
    pub(crate) width: usize,
    pub(crate) height: usize,
    samples: Samples,
}

#[derive(Debug, Clone, PartialEq)]
enum Samples {
    Intensity(Vec<f32>),
    Labels(Vec<LabelSample>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum LabelSample {
    Outside,
    Invalid,
    Id(i64),
}

impl LabelSample {
    /// The sample of voxel `value` inside the volume.
    fn of(value: f64) -> Self {
        label_id(value).map_or(Self::Invalid, Self::Id)
    }
}

impl Slice<'_> {
    pub(crate) fn sample(&self, width: usize, height: usize) -> SampledSlice {
        let samples = dispatch_data_type!(self.volume.metadata().data_type, |T| {
            self.sample_typed::<T>(width, height)
        });
        SampledSlice {
            width,
            height,
            samples,
        }
    }

    pub(crate) fn sample_mip(&self, width: usize, height: usize) -> SampledSlice {
        let axis = projection::projection_axis(self.geometry, &self.plane);
        let mip = self.volume.projection(self.frame, axis);
        let samples = if self.labels {
            Samples::Labels(
                self.sample_pixels(width, height, LabelSample::Outside, |voxel| {
                    LabelSample::of(f64::from(mip.sample_nearest(voxel)))
                }),
            )
        } else {
            Samples::Intensity(
                self.sample_pixels(width, height, f32::NAN, |voxel| mip.sample_bilinear(voxel)),
            )
        };
        SampledSlice {
            width,
            height,
            samples,
        }
    }

    fn sample_typed<T: Scalar>(&self, width: usize, height: usize) -> Samples {
        let voxels = TypedVoxelReader::<T>::new(self.volume, self.frame);
        let scaling = self.volume.metadata().scaling;
        if self.labels {
            Samples::Labels(
                self.sample_pixels(width, height, LabelSample::Outside, |voxel| {
                    LabelSample::of(scaling.apply(voxels.sample_nearest(voxel)))
                }),
            )
        } else {
            Samples::Intensity(self.sample_pixels(width, height, f32::NAN, |voxel| {
                let raw = voxels.sample_trilinear(voxel);
                if raw.is_finite() {
                    scaling.apply(raw) as f32
                } else {
                    f32::NAN
                }
            }))
        }
    }

    fn sample_pixels<P: Copy + Send + Sync>(
        &self,
        width: usize,
        height: usize,
        outside: P,
        sample: impl Fn(Point3D) -> P + Sync,
    ) -> Vec<P> {
        let mut pixels = vec![outside; width * height];
        if width == 0 || height == 0 {
            return pixels;
        }
        let to_voxel = &self.geometry.world_to_voxel;
        let origin = to_voxel.transform_point(self.plane.origin);
        let step_s = to_voxel.transform_point(self.plane.origin + self.plane.u) - origin;
        let step_t = to_voxel.transform_point(self.plane.origin + self.plane.v) - origin;

        pixels
            .par_chunks_exact_mut(width)
            .enumerate()
            .for_each(|(row, line)| {
                // t=1 at row 0
                let t = 1.0 - (row as f64 + 0.5) / height as f64;
                for (col, pixel) in line.iter_mut().enumerate() {
                    let s = (col as f64 + 0.5) / width as f64;
                    let voxel = origin + step_s * s + step_t * t;
                    if self.geometry.contains_voxel(voxel) {
                        *pixel = sample(voxel);
                    }
                }
            });
        pixels
    }
}

impl SampledSlice {
    pub fn colorize(&self, coloring: Coloring) -> SliceImage {
        let mut pixels = vec![0; self.width * self.height * 4];
        let rgba_pixels = pixels.as_chunks_mut::<4>().0;
        let row = self.width.max(1);
        match (&self.samples, coloring) {
            (Samples::Intensity(values), Coloring::Intensity { colormap, .. }) => {
                let table = ColorTable::new(colormap);
                rgba_pixels
                    .par_chunks_mut(row)
                    .zip(values.par_chunks(row))
                    .for_each(|(line, values)| {
                        for (pixel, &value) in line.iter_mut().zip(values) {
                            if let Some(t) = coloring.window_position(f64::from(value)) {
                                let [r, g, b] = table.color(t);
                                *pixel = [r, g, b, 255];
                            }
                        }
                    });
            }
            (Samples::Labels(labels), coloring) => {
                let outline = matches!(coloring, Coloring::Labels { outline: true });
                let (width, height) = (self.width, self.height);
                rgba_pixels
                    .par_chunks_mut(row)
                    .enumerate()
                    .for_each(|(y, line)| {
                        for (x, pixel) in line.iter_mut().enumerate() {
                            if outline && !is_label_boundary(labels, width, height, x, y) {
                                continue;
                            }
                            *pixel = match labels[y * width + x] {
                                LabelSample::Outside => [0; 4],
                                LabelSample::Invalid => INVALID_LABEL_COLOR,
                                LabelSample::Id(id) => label_color(id),
                            };
                        }
                    });
            }
            (Samples::Intensity(_), Coloring::Labels { .. }) => {}
        }
        SliceImage {
            width: self.width,
            height: self.height,
            pixels,
        }
    }
}

pub fn fit_size(plane: &SlicePlane, panel_width: usize, panel_height: usize) -> (usize, usize) {
    if panel_width == 0 || panel_height == 0 {
        return (0, 0);
    }
    let (scale_x, scale_y) = aspect_scale(
        panel_width as f32,
        panel_height as f32,
        plane.u.length(),
        plane.v.length(),
    );
    let fit = |size: usize, scale: f32| ((size as f32 * scale).round() as usize).max(1);
    (fit(panel_width, scale_x), fit(panel_height, scale_y))
}

struct TypedVoxelReader<'a, T> {
    bytes: &'a [u8],
    endian: ByteOrder,
    dimensions: [usize; 3],
    _scalar_type: PhantomData<T>,
}

impl<'a, T: Scalar> TypedVoxelReader<'a, T> {
    fn new(volume: &'a Volume, frame: usize) -> Self {
        let metadata = volume.metadata();
        Self {
            bytes: volume.frame_bytes(frame),
            endian: metadata.endian,
            dimensions: [metadata.width(), metadata.height(), metadata.depth()],
            _scalar_type: PhantomData,
        }
    }

    fn value_at(&self, [x, y, z]: [usize; 3]) -> f64 {
        let index = x + self.dimensions[0] * (y + self.dimensions[1] * z);
        T::read(
            &self.bytes[index * T::SIZE..(index + 1) * T::SIZE],
            self.endian,
        )
    }

    fn sample_nearest(&self, p: Point3D) -> f64 {
        self.value_at(std::array::from_fn(|axis| {
            clamp_index((p[axis] + 0.5).floor(), self.dimensions[axis])
        }))
    }

    fn sample_trilinear(&self, p: Point3D) -> f64 {
        let base: [f64; 3] = std::array::from_fn(|axis| p[axis].floor());
        let fraction: [f64; 3] = std::array::from_fn(|axis| p[axis] - base[axis]);
        let low: [usize; 3] =
            std::array::from_fn(|axis| clamp_index(base[axis], self.dimensions[axis]));
        let high: [usize; 3] =
            std::array::from_fn(|axis| clamp_index(base[axis] + 1.0, self.dimensions[axis]));

        let mut sum = 0.0;
        for corner in 0..8 {
            let mut weight = 1.0;
            let index = std::array::from_fn(|axis| {
                if corner & (1 << axis) != 0 {
                    weight *= fraction[axis];
                    high[axis]
                } else {
                    weight *= 1.0 - fraction[axis];
                    low[axis]
                }
            });
            if weight != 0.0 {
                sum += weight * self.value_at(index);
            }
        }
        sum
    }
}

/// voxel coordinate to index from 0..size-1
fn clamp_index(coordinate: f64, size: usize) -> usize {
    (coordinate as usize).min(size - 1)
}

fn is_label_boundary(
    labels: &[LabelSample],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
) -> bool {
    let label = labels[y * width + x];
    let differs = |x: usize, y: usize| labels[y * width + x] != label;
    (1..=OUTLINE_WIDTH).any(|d| {
        (x >= d && differs(x - d, y))
            || (x + d < width && differs(x + d, y))
            || (y >= d && differs(x, y - d))
            || (y + d < height && differs(x, y + d))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{AffineTransform, GeometryOptions, ViewPlane};
    use crate::metadata::Matrix3x4;
    use crate::test_fixtures::{Fixture, write_and_load};
    use crate::voxel::DataType;

    /// The value at world point `p` interpolated.
    fn sample_linear(
        volume: &Volume,
        geometry: &VolumeGeometry,
        frame: usize,
        p: Point3D,
    ) -> Option<f64> {
        let voxel = geometry.world_to_voxel.transform_point(p);
        if !geometry.contains_voxel(voxel) {
            return None;
        }
        let raw = dispatch_data_type!(volume.metadata().data_type, |T| {
            TypedVoxelReader::<T>::new(volume, frame).sample_trilinear(voxel)
        });
        raw.is_finite()
            .then(|| volume.metadata().scaling.apply(raw))
    }

    fn geometry_of(volume: &Volume) -> VolumeGeometry {
        volume.geometry(GeometryOptions::default()).unwrap()
    }

    fn plane_at(geometry: &VolumeGeometry, axis: ViewPlane, voxel: Point3D) -> SlicePlane {
        geometry.plane(axis, geometry.voxel_to_world.transform_point(voxel), false)
    }

    fn grayscale(level: f64, width: f64) -> Coloring {
        Coloring::Intensity {
            level,
            width,
            colormap: Colormap::Grayscale,
            clip_low: false,
        }
    }

    fn pixel(image: &SliceImage, col: usize, row: usize) -> [u8; 4] {
        let start = 4 * (col + row * image.width);
        image.pixels[start..start + 4].try_into().unwrap()
    }

    fn render_axial(volume: &Volume, coloring: Coloring, frame: usize) -> SliceImage {
        let geometry = geometry_of(volume);
        let slice = Slice {
            volume,
            geometry: &geometry,
            plane: plane_at(&geometry, ViewPlane::Axial, Point3D::new(1.0, 1.0, 1.0)),
            frame,
            labels: matches!(coloring, Coloring::Labels { .. }),
        };
        slice.sample(4, 3).colorize(coloring)
    }

    #[test]
    fn samples_voxel_centers_exactly() {
        let directory = tempfile::tempdir().unwrap();
        let ramp = Fixture {
            datatype: DataType::F32,
            value: |i| (i / 12) as f64 * 10.0,
            ..Fixture::default()
        };
        let volume = write_and_load(&directory, &ramp, "ramp.nii");
        let geometry = geometry_of(&volume);
        let at = |z| sample_linear(&volume, &geometry, 0, Point3D::new(1.0, 1.0, z));
        assert_eq!(at(1.0), Some(10.0));
        assert_eq!(at(1.5), Some(15.0));
        assert_eq!(at(3.4), Some(30.0));
        assert_eq!(at(3.6), None);

        let image = render_axial(&volume, grayscale(15.0, 30.0), 0);
        assert!(image.pixels.chunks(4).all(|p| p == [85, 85, 85, 255]));
    }

    #[test]
    fn applies_intensity_scaling() {
        let directory = tempfile::tempdir().unwrap();
        for (slope, expected) in [(0.0, 10.0), (-2.0, 80.0), (0.001, 100.01)] {
            let scaled = Fixture {
                datatype: DataType::F32,
                value: |i| (i / 12) as f64 * 10.0,
                slope,
                intercept: 100.0,
                ..Fixture::default()
            };
            let volume = write_and_load(&directory, &scaled, &format!("scaled-{slope}.nii"));
            let geometry = geometry_of(&volume);
            let value = sample_linear(&volume, &geometry, 0, Point3D::new(1.0, 1.0, 1.0)).unwrap();
            assert!((value - expected).abs() < 1e-4, "slope {slope}: {value}");
        }
    }

    #[test]
    fn hides_non_finite_values_and_empty_space() {
        let directory = tempfile::tempdir().unwrap();
        let all_nan = Fixture {
            datatype: DataType::F32,
            value: |_| f64::NAN,
            ..Fixture::default()
        };
        let image = render_axial(
            &write_and_load(&directory, &all_nan, "nan.nii"),
            grayscale(0.0, 1.0),
            0,
        );
        assert!(image.pixels.chunks(4).all(|p| p[3] == 0));

        let first_slice_nan = Fixture {
            datatype: DataType::F32,
            value: |i| if i < 12 { f64::NAN } else { 1.0 },
            ..Fixture::default()
        };
        let volume = write_and_load(&directory, &first_slice_nan, "partial.nii");
        let geometry = geometry_of(&volume);
        let at = |z| sample_linear(&volume, &geometry, 0, Point3D::new(1.0, 1.0, z));
        assert_eq!(at(1.0), Some(1.0));
        assert_eq!(at(0.5), None);

        let volume = write_and_load(&directory, &Fixture::default(), "outside.nii");
        let geometry = geometry_of(&volume);
        let slice = Slice {
            volume: &volume,
            geometry: &geometry,
            plane: geometry.plane(ViewPlane::Axial, Point3D::new(1e4, 1e4, 1e4), false),
            frame: 0,
            labels: false,
        };
        let image = slice.sample(8, 8).colorize(grayscale(0.0, 1.0));
        assert!(image.pixels.chunks(4).all(|p| p[3] == 0));
    }

    fn field(p: Point3D) -> f64 {
        20.0 + p.x + 2.0 * p.y + 3.0 * p.z
    }

    fn orientation(case: usize) -> Matrix3x4 {
        let (sin, cos) = (f64::from(0.4f32.sin()), f64::from(0.4f32.cos()));
        match case {
            0 => [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
            1 => [
                [-1.0, 0.0, 0.0, 6.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
            2 => [
                [0.0, 0.0, -1.0, 4.0],
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
            ],
            _ => [
                [cos, -sin, 0.0, 2.0],
                [sin, cos, 0.0, -2.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
        }
    }

    fn phantom(case: usize, i: usize) -> f64 {
        let voxel = Point3D::new((i % 7) as f64, ((i / 7) % 9) as f64, (i / 63) as f64);
        field(
            AffineTransform {
                matrix: orientation(case),
            }
            .transform_point(voxel),
        )
    }

    #[test]
    fn reslices_every_orientation_exactly() {
        let directory = tempfile::tempdir().unwrap();
        let values: [fn(usize) -> f64; 4] = [
            |i| phantom(0, i),
            |i| phantom(1, i),
            |i| phantom(2, i),
            |i| phantom(3, i),
        ];
        let (mut sampled, mut rendered) = (0, 0);
        for (case, value) in values.into_iter().enumerate() {
            let fixture = Fixture {
                dimensions: vec![7, 9, 5],
                datatype: DataType::F32,
                sform: orientation(case),
                value,
                ..Fixture::default()
            };
            let volume = write_and_load(&directory, &fixture, &format!("orientation-{case}.nii"));
            let geometry = geometry_of(&volume);
            let cursor = geometry
                .voxel_to_world
                .transform_point(Point3D::new(3.0, 4.0, 2.0));
            let interior = |world: Point3D| {
                let voxel = geometry.world_to_voxel.transform_point(world);
                (0..3).all(|a| voxel[a] >= 0.0 && voxel[a] <= geometry.dimensions[a] as f64 - 1.0)
            };

            for radiological in [false, true] {
                for axis in ViewPlane::ALL {
                    let plane = geometry.plane(axis, cursor, radiological);
                    let context = format!("case {case}, {axis:?}, radiological {radiological}");

                    for i in 1..20 {
                        for j in 1..20 {
                            let world = plane.point(i as f64 / 20.0, j as f64 / 20.0);
                            if !interior(world) {
                                continue;
                            }
                            let value = sample_linear(&volume, &geometry, 0, world).unwrap();
                            assert!((value - field(world)).abs() < 1e-4, "{context}");
                            sampled += 1;
                        }
                    }

                    let slice = Slice {
                        volume: &volume,
                        geometry: &geometry,
                        plane,
                        frame: 0,
                        labels: false,
                    };
                    let (width, height) = (40, 40);
                    let image = slice.sample(width, height).colorize(grayscale(50.0, 100.0));
                    for row in 0..height {
                        for col in 0..width {
                            let s = (col as f64 + 0.5) / width as f64;
                            let t = 1.0 - (row as f64 + 0.5) / height as f64;
                            let world = plane.point(s, t);
                            if !interior(world) {
                                continue;
                            }
                            let expected = field(world) / 100.0 * 255.0;
                            let actual = f64::from(pixel(&image, col, row)[0]);
                            assert!((actual - expected).abs() <= 0.5 + 1e-6, "{context}");
                            rendered += 1;
                        }
                    }
                }
            }
        }
        assert!(
            sampled > 300 && rendered > 300,
            "{sampled} / {rendered} checked"
        );
    }

    #[test]
    fn labels_keep_their_colors() {
        let directory = tempfile::tempdir().unwrap();
        let labels = Fixture {
            datatype: DataType::I32,
            dimensions: vec![4, 3, 4, 2],
            value: |i| {
                const IDS: [f64; 4] = [0.0, 300.0, 1000.0, 16_777_217.0];
                IDS[(i + usize::from(i >= 48)) % 4]
            },
            ..Fixture::default()
        };
        let volume = write_and_load(&directory, &labels, "labels.nii");
        let frame0 = render_axial(&volume, Coloring::Labels { outline: false }, 0);
        let row = 2;
        assert_eq!(pixel(&frame0, 0, row), [0, 0, 0, 0]);
        assert_eq!(pixel(&frame0, 1, row), label_color(300));
        assert_eq!(pixel(&frame0, 2, row), label_color(1000));
        assert_eq!(pixel(&frame0, 3, row), label_color(16_777_217));

        let frame1 = render_axial(&volume, Coloring::Labels { outline: false }, 1);
        assert_eq!(pixel(&frame1, 0, row), label_color(300));
    }

    #[test]
    fn recolors_without_resampling() {
        let directory = tempfile::tempdir().unwrap();
        let volume = write_and_load(&directory, &Fixture::default(), "recolor.nii");
        let geometry = geometry_of(&volume);
        let slice = Slice {
            volume: &volume,
            geometry: &geometry,
            plane: plane_at(&geometry, ViewPlane::Axial, Point3D::new(1.0, 1.0, 1.0)),
            frame: 0,
            labels: false,
        };
        let samples = slice.sample(4, 3);
        for (level, width) in [(23.5, 47.0), (10.0, 5.0)] {
            let reused = samples.colorize(grayscale(level, width));
            let fresh = slice.sample(4, 3).colorize(grayscale(level, width));
            assert_eq!(reused, fresh);
        }
        assert_eq!(
            slice
                .sample(0, 3)
                .colorize(grayscale(0.0, 1.0))
                .pixels
                .len(),
            0
        );
        // labels need their own nearest-voxel samples
        assert!(
            samples
                .colorize(Coloring::Labels { outline: false })
                .pixels
                .iter()
                .all(|&b| b == 0)
        );
    }

    #[test]
    fn clips_values_below_the_window() {
        let directory = tempfile::tempdir().unwrap();
        let ramp = Fixture {
            datatype: DataType::F32,
            value: |i| (i % 4) as f64 * 10.0,
            ..Fixture::default()
        };
        let volume = write_and_load(&directory, &ramp, "clip.nii");
        let clipped = Coloring::Intensity {
            level: 15.0,
            width: 10.0,
            colormap: Colormap::Grayscale,
            clip_low: true,
        };
        let image = render_axial(&volume, clipped, 0);
        let alpha = |col| pixel(&image, col, 0)[3];
        assert_eq!([alpha(0), alpha(1), alpha(2), alpha(3)], [0, 0, 255, 255]);
    }

    #[test]
    fn fits_images_inside_panels() {
        let directory = tempfile::tempdir().unwrap();
        let volume = write_and_load(&directory, &Fixture::default(), "fit.nii");
        let geometry = geometry_of(&volume);
        let plane = plane_at(&geometry, ViewPlane::Axial, Point3D::new(1.0, 1.0, 1.0));
        assert_eq!(fit_size(&plane, 400, 400), (400, 300));
        assert_eq!(fit_size(&plane, 400, 150), (200, 150));
        assert_eq!(fit_size(&plane, 0, 150), (0, 0));
    }

    #[test]
    fn outlines_only_draw_edges() {
        let directory = tempfile::tempdir().unwrap();
        let halves = Fixture {
            datatype: DataType::I32,
            value: |i| if i % 4 == 0 { 0.0 } else { 1.0 },
            ..Fixture::default()
        };
        let volume = write_and_load(&directory, &halves, "halves.nii");
        let image = render_axial(&volume, Coloring::Labels { outline: true }, 0);
        let alpha = |col| pixel(&image, col, 1)[3];
        assert_eq!([alpha(0), alpha(1), alpha(2), alpha(3)], [0, 255, 255, 0]);
    }
}
