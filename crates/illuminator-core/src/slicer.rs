//! CPU slicer. Slice to RGBA img

use ::rayon::prelude::*;
use std::marker::PhantomData;

use crate::geometry::{Point3D, SlicePlane, VolumeGeometry, aspect_scale};
use crate::labels::label_color;
use crate::volume::NiiVolume;
use crate::voxel::{self, Endian, Scalar, with_scalar};

const INVALID_LABEL: [u8; 4] = [255, 0, 255, 255];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Colormap {
    #[default]
    Grayscale,
    Viridis,
    Inferno,
    Plasma,
    Magma,
}

impl Colormap {
    pub const ALL: [Self; 5] = [
        Self::Grayscale,
        Self::Viridis,
        Self::Plasma,
        Self::Magma,
        Self::Inferno,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Grayscale => "Grayscale",
            Self::Viridis => "Viridis",
            Self::Inferno => "Inferno",
            Self::Plasma => "Plasma",
            Self::Magma => "Magma",
        }
    }

    pub fn color(self, t: f64) -> [u8; 3] {
        let t = t.clamp(0.0, 1.0);
        let rgb = match self {
            Self::Grayscale => [t; 3],
            Self::Viridis => polynomial(&VIRIDIS, t),
            Self::Inferno => polynomial(&INFERNO, t),
            Self::Plasma => polynomial(&PLASMA, t),
            Self::Magma => polynomial(&MAGMA, t),
        };
        // "as u8" turns nans into 0, making invalid values black
        rgb.map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Coloring {
    Intensity {
        level: f64,
        width: f64,
        colormap: Colormap,
        // below min are transparent
        clip_low: bool,
    },
    Labels,
}

impl Coloring {
    pub fn is_labels(self) -> bool {
        matches!(self, Self::Labels)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SliceImage {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Copy)]
pub struct Slice<'a> {
    pub volume: &'a NiiVolume,
    pub geometry: &'a VolumeGeometry,
    pub plane: SlicePlane,
    pub frame: usize,
    pub labels: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SampledSlice {
    pub width: usize,
    pub height: usize,
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

impl Slice<'_> {
    pub fn sample(&self, width: usize, height: usize) -> SampledSlice {
        let samples = with_scalar!(self.volume.header().datatype, |T| {
            self.sample_typed::<T>(width, height)
        });
        SampledSlice {
            width,
            height,
            samples,
        }
    }

    fn sample_typed<T: Scalar>(&self, width: usize, height: usize) -> Samples {
        let voxels = Voxels::<T>::new(self.volume, self.frame);
        let scaling = self.volume.header().scaling;
        if self.labels {
            Samples::Labels(
                self.sample_pixels(width, height, LabelSample::Outside, |voxel| {
                    match voxel::label_id(scaling.apply(voxels.nearest(voxel))) {
                        Ok(id) => LabelSample::Id(id),
                        Err(_) => LabelSample::Invalid,
                    }
                }),
            )
        } else {
            Samples::Intensity(self.sample_pixels(width, height, f32::NAN, |voxel| {
                let raw = voxels.linear(voxel);
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
        let origin = to_voxel.point(self.plane.origin);
        let step_s = to_voxel.point(self.plane.origin + self.plane.u) - origin;
        let step_t = to_voxel.point(self.plane.origin + self.plane.v) - origin;

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
        let out = pixels.as_chunks_mut::<4>().0;
        let row = self.width.max(1);
        match (&self.samples, coloring) {
            (
                Samples::Intensity(values),
                Coloring::Intensity {
                    level,
                    width,
                    colormap,
                    clip_low,
                },
            ) => {
                out.par_chunks_mut(row)
                    .zip(values.par_chunks(row))
                    .for_each(|(line, values)| {
                        for (pixel, &value) in line.iter_mut().zip(values) {
                            let t = (f64::from(value) - level) / width + 0.5;
                            if !value.is_nan() && !(clip_low && t <= 0.0) {
                                let [r, g, b] = colormap.color(t);
                                *pixel = [r, g, b, 255];
                            }
                        }
                    });
            }
            (Samples::Labels(labels), _) => {
                out.par_chunks_mut(row)
                    .zip(labels.par_chunks(row))
                    .for_each(|(line, labels)| {
                        for (pixel, label) in line.iter_mut().zip(labels) {
                            *pixel = match *label {
                                LabelSample::Outside => [0; 4],
                                LabelSample::Invalid => INVALID_LABEL,
                                LabelSample::Id(id) => label_color(id),
                            };
                        }
                    });
            }
            (Samples::Intensity(_), Coloring::Labels) => {}
        }
        SliceImage {
            width: self.width,
            height: self.height,
            pixels,
        }
    }
}

pub fn sample_linear(
    volume: &NiiVolume,
    geometry: &VolumeGeometry,
    frame: usize,
    p: Point3D,
) -> Option<f64> {
    let voxel = geometry.world_to_voxel.point(p);
    if !geometry.contains_voxel(voxel) {
        return None;
    }
    let raw = with_scalar!(volume.header().datatype, |T| {
        Voxels::<T>::new(volume, frame).linear(voxel)
    });
    raw.is_finite().then(|| volume.header().scaling.apply(raw))
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

struct Voxels<'a, T> {
    bytes: &'a [u8],
    endian: Endian,
    dims: [usize; 3],
    _type: PhantomData<T>,
}

impl<'a, T: Scalar> Voxels<'a, T> {
    fn new(volume: &'a NiiVolume, frame: usize) -> Self {
        let h = volume.header();
        Self {
            bytes: volume.frame(frame),
            endian: h.endian,
            dims: [h.width(), h.height(), h.depth()],
            _type: PhantomData,
        }
    }

    fn get(&self, [x, y, z]: [usize; 3]) -> f64 {
        let index = x + self.dims[0] * (y + self.dims[1] * z);
        T::read(
            &self.bytes[index * T::SIZE..(index + 1) * T::SIZE],
            self.endian,
        )
    }

    fn nearest(&self, p: Point3D) -> f64 {
        self.get(std::array::from_fn(|axis| {
            clamp_index((p[axis] + 0.5).floor(), self.dims[axis])
        }))
    }

    fn linear(&self, p: Point3D) -> f64 {
        let base: [f64; 3] = std::array::from_fn(|axis| p[axis].floor());
        let frac: [f64; 3] = std::array::from_fn(|axis| p[axis] - base[axis]);
        let low: [usize; 3] = std::array::from_fn(|axis| clamp_index(base[axis], self.dims[axis]));
        let high: [usize; 3] =
            std::array::from_fn(|axis| clamp_index(base[axis] + 1.0, self.dims[axis]));

        let mut sum = 0.0;
        for corner in 0..8 {
            let mut weight = 1.0;
            let index = std::array::from_fn(|axis| {
                if corner & (1 << axis) != 0 {
                    weight *= frac[axis];
                    high[axis]
                } else {
                    weight *= 1.0 - frac[axis];
                    low[axis]
                }
            });
            if weight != 0.0 {
                sum += weight * self.get(index);
            }
        }
        sum
    }
}

/// voxel coordinate to index from 0..size-1
fn clamp_index(coordinate: f64, size: usize) -> usize {
    (coordinate as usize).min(size - 1)
}

fn polynomial(coefficients: &[[f64; 3]; 7], t: f64) -> [f64; 3] {
    coefficients.iter().rev().fold([0.0; 3], |sum, c| {
        std::array::from_fn(|channel| c[channel] + t * sum[channel])
    })
}

/// Polynomial colormap approximations by Matt Zucker: https://www.shadertoy.com/view/WlfXRN
const VIRIDIS: [[f64; 3]; 7] = [
    [0.2777273272234177, 0.005407344544966578, 0.3340998053353061],
    [0.1050930431085774, 1.404613529898575, 1.384590162594685],
    [-0.3308618287255563, 0.214847559468213, 0.09509516302823659],
    [-4.634230498983486, -5.799100973351585, -19.33244095627987],
    [6.228269936347081, 14.17993336680509, 56.69055260068105],
    [4.776384997670288, -13.74514537774601, -65.35303263337234],
    [-5.435455855934631, 4.645852612178535, 26.3124352495832],
];
/// Polynomial colormap approximations by Matt Zucker: https://www.shadertoy.com/view/WlfXRN
const PLASMA: [[f64; 3]; 7] = [
    [0.05873234392399702, 0.02333670892565664, 0.5433401826748754],
    [2.176514634195958, 0.2383834171260182, 0.7539604599784036],
    [-2.689460476458034, -7.455851135738909, 3.110799939717086],
    [6.130348345893603, 42.3461881477227, -28.51885465332158],
    [-11.10743619062271, -82.66631109428045, 60.13984767418263],
    [10.02306557647065, 71.4136177009535, -54.07218655560067],
    [-3.658713842777788, -22.93153465461149, 18.19190778539828],
];
/// Polynomial colormap approximations by Matt Zucker: https://www.shadertoy.com/view/WlfXRN
const MAGMA: [[f64; 3]; 7] = [
    [
        -0.002136485053939582,
        -0.000749655052795221,
        -0.005386127855323933,
    ],
    [0.2516605407371642, 0.6775232436837668, 2.494026599312351],
    [8.353717279216625, -3.577719514958484, 0.3144679030132573],
    [-27.66873308576866, 14.26473078096533, -13.64921318813922],
    [52.17613981234068, -27.94360607168351, 12.94416944238394],
    [-50.76852536473588, 29.04658282127291, 4.23415299384598],
    [18.65570506591883, -11.48977351997711, -5.601961508734096],
];
/// Polynomial colormap approximations by Matt Zucker: https://www.shadertoy.com/view/WlfXRN
const INFERNO: [[f64; 3]; 7] = [
    [
        0.0002189403691192265,
        0.001651004631001012,
        -0.01948089843709184,
    ],
    [0.1065134194856116, 0.5639564367884091, 3.932712388889277],
    [11.60249308247187, -3.972853965665698, -15.9423941062914],
    [-41.70399613139459, 17.43639888205313, 44.35414519872813],
    [77.162935699427, -33.40235894210092, -81.80730925738993],
    [-71.31942824499214, 32.62606426397723, 73.20951985803202],
    [25.13112622477341, -12.24266895238567, -23.07032500287172],
];

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::cache::VolumeCache;
    use crate::geometry::{Affine, GeometryOptions, ViewPlane};
    use crate::header::Mat3x4;
    use crate::progress::Progress;
    use crate::test_util::Fixture;
    use crate::voxel::DataType;

    fn load(dir: &TempDir, fixture: &Fixture, name: &str) -> NiiVolume {
        let path = dir.path().join(name);
        fixture.write(&path);
        let cache = VolumeCache::new(dir.path().join("cache"), 1 << 20).unwrap();
        NiiVolume::load(&path, &cache, &Progress::default()).unwrap()
    }

    fn geometry_of(volume: &NiiVolume) -> VolumeGeometry {
        volume.geometry(GeometryOptions::default()).unwrap()
    }

    fn plane_at(geometry: &VolumeGeometry, axis: ViewPlane, voxel: Point3D) -> SlicePlane {
        geometry.plane(axis, geometry.voxel_to_world.point(voxel), false)
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

    fn render_axial(volume: &NiiVolume, coloring: Coloring, frame: usize) -> SliceImage {
        let geometry = geometry_of(volume);
        let slice = Slice {
            volume,
            geometry: &geometry,
            plane: plane_at(&geometry, ViewPlane::Axial, Point3D::new(1.0, 1.0, 1.0)),
            frame,
            labels: coloring.is_labels(),
        };
        slice.sample(4, 3).colorize(coloring)
    }

    #[test]
    fn samples_voxel_centers_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let ramp = Fixture {
            datatype: DataType::F32,
            value: |i| (i / 12) as f64 * 10.0,
            ..Fixture::default()
        };
        let volume = load(&dir, &ramp, "ramp.nii");
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
        let dir = tempfile::tempdir().unwrap();
        for (slope, expected) in [(0.0, 10.0), (-2.0, 80.0), (0.001, 100.01)] {
            let scaled = Fixture {
                datatype: DataType::F32,
                value: |i| (i / 12) as f64 * 10.0,
                slope,
                intercept: 100.0,
                ..Fixture::default()
            };
            let volume = load(&dir, &scaled, &format!("scaled-{slope}.nii"));
            let geometry = geometry_of(&volume);
            let value = sample_linear(&volume, &geometry, 0, Point3D::new(1.0, 1.0, 1.0)).unwrap();
            assert!((value - expected).abs() < 1e-4, "slope {slope}: {value}");
        }
    }

    #[test]
    fn hides_non_finite_values_and_empty_space() {
        let dir = tempfile::tempdir().unwrap();
        let all_nan = Fixture {
            datatype: DataType::F32,
            value: |_| f64::NAN,
            ..Fixture::default()
        };
        let image = render_axial(&load(&dir, &all_nan, "nan.nii"), grayscale(0.0, 1.0), 0);
        assert!(image.pixels.chunks(4).all(|p| p[3] == 0));

        let first_slice_nan = Fixture {
            datatype: DataType::F32,
            value: |i| if i < 12 { f64::NAN } else { 1.0 },
            ..Fixture::default()
        };
        let volume = load(&dir, &first_slice_nan, "partial.nii");
        let geometry = geometry_of(&volume);
        let at = |z| sample_linear(&volume, &geometry, 0, Point3D::new(1.0, 1.0, z));
        assert_eq!(at(1.0), Some(1.0));
        assert_eq!(at(0.5), None);

        let volume = load(&dir, &Fixture::default(), "outside.nii");
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

    fn orientation(case: usize) -> Mat3x4 {
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
            Affine {
                m: orientation(case),
            }
            .point(voxel),
        )
    }

    #[test]
    fn reslices_every_orientation_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let values: [fn(usize) -> f64; 4] = [
            |i| phantom(0, i),
            |i| phantom(1, i),
            |i| phantom(2, i),
            |i| phantom(3, i),
        ];
        let (mut sampled, mut rendered) = (0, 0);
        for (case, value) in values.into_iter().enumerate() {
            let fixture = Fixture {
                dims: vec![7, 9, 5],
                datatype: DataType::F32,
                sform: orientation(case),
                value,
                ..Fixture::default()
            };
            let volume = load(&dir, &fixture, &format!("orientation-{case}.nii"));
            let geometry = geometry_of(&volume);
            let cursor = geometry.voxel_to_world.point(Point3D::new(3.0, 4.0, 2.0));
            let interior = |world: Point3D| {
                let voxel = geometry.world_to_voxel.point(world);
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
        let dir = tempfile::tempdir().unwrap();
        let labels = Fixture {
            datatype: DataType::I32,
            dims: vec![4, 3, 4, 2],
            value: |i| {
                const IDS: [f64; 4] = [0.0, 300.0, 1000.0, 16_777_217.0];
                IDS[(i + usize::from(i >= 48)) % 4]
            },
            ..Fixture::default()
        };
        let volume = load(&dir, &labels, "labels.nii");
        let frame0 = render_axial(&volume, Coloring::Labels, 0);
        let row = 2;
        assert_eq!(pixel(&frame0, 0, row), [0, 0, 0, 0]);
        assert_eq!(pixel(&frame0, 1, row), label_color(300));
        assert_eq!(pixel(&frame0, 2, row), label_color(1000));
        assert_eq!(pixel(&frame0, 3, row), label_color(16_777_217));

        let frame1 = render_axial(&volume, Coloring::Labels, 1);
        assert_eq!(pixel(&frame1, 0, row), label_color(300));
    }

    #[test]
    fn recolors_without_resampling() {
        let dir = tempfile::tempdir().unwrap();
        let volume = load(&dir, &Fixture::default(), "recolor.nii");
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
                .colorize(Coloring::Labels)
                .pixels
                .iter()
                .all(|&b| b == 0)
        );
    }

    #[test]
    fn clips_values_below_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let ramp = Fixture {
            datatype: DataType::F32,
            value: |i| (i % 4) as f64 * 10.0,
            ..Fixture::default()
        };
        let volume = load(&dir, &ramp, "clip.nii");
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
    fn colormaps() {
        assert_eq!(Colormap::Grayscale.color(0.0), [0, 0, 0]);
        assert_eq!(Colormap::Grayscale.color(1.0), [255, 255, 255]);
        assert_eq!(Colormap::Grayscale.color(7.0), [255, 255, 255]);
        assert_eq!(Colormap::Grayscale.color(f64::NAN), [0, 0, 0]);
        // Close to the real viridis endpoints, dark purple and yellow.
        assert_eq!(Colormap::Viridis.color(0.0), [71, 1, 85]);
        assert_eq!(Colormap::Viridis.color(1.0), [252, 231, 33]);
    }

    #[test]
    fn fits_images_inside_panels() {
        let dir = tempfile::tempdir().unwrap();
        let volume = load(&dir, &Fixture::default(), "fit.nii");
        let geometry = geometry_of(&volume);
        let plane = plane_at(&geometry, ViewPlane::Axial, Point3D::new(1.0, 1.0, 1.0));
        assert_eq!(fit_size(&plane, 400, 400), (400, 300));
        assert_eq!(fit_size(&plane, 400, 150), (200, 150));
        assert_eq!(fit_size(&plane, 0, 150), (0, 0));
    }
}
