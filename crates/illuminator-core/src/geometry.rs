//! geometry.rs
//!
//! Points, affines, planes, and world space
//!

use std::ops::{Add, Index, IndexMut, Mul, Neg, Sub};

use crate::error::{Error, Result};
use crate::meta::{Mat3x4, SpatialUnits, VolumeMeta};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point3D {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point3D {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub(crate) fn from_array([x, y, z]: [f64; 3]) -> Self {
        Self { x, y, z }
    }

    pub(crate) fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub(crate) fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    pub(crate) fn length(self) -> f64 {
        self.x.hypot(self.y).hypot(self.z)
    }

    pub(crate) fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    pub(crate) fn min(self, other: Self) -> Self {
        Self::new(
            self.x.min(other.x),
            self.y.min(other.y),
            self.z.min(other.z),
        )
    }

    pub(crate) fn max(self, other: Self) -> Self {
        Self::new(
            self.x.max(other.x),
            self.y.max(other.y),
            self.z.max(other.z),
        )
    }
}

impl Add for Point3D {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }
}

impl Sub for Point3D {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }
}

impl Mul<f64> for Point3D {
    type Output = Self;

    fn mul(self, scale: f64) -> Self {
        Self::new(self.x * scale, self.y * scale, self.z * scale)
    }
}

impl Neg for Point3D {
    type Output = Self;

    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}

impl Index<usize> for Point3D {
    type Output = f64;

    fn index(&self, axis: usize) -> &f64 {
        match axis {
            0 => &self.x,
            1 => &self.y,
            2 => &self.z,
            _ => panic!("axis {axis} is out of range"),
        }
    }
}

impl IndexMut<usize> for Point3D {
    fn index_mut(&mut self, axis: usize) -> &mut f64 {
        match axis {
            0 => &mut self.x,
            1 => &mut self.y,
            2 => &mut self.z,
            _ => panic!("axis {axis} is out of range"),
        }
    }
}

/// Transform with implicit last row (0, 0, 0, 1)
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Affine {
    pub(crate) m: Mat3x4,
}

impl Affine {
    pub(crate) const IDENTITY: Self = Self {
        m: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
    };

    pub(crate) fn point(&self, p: Point3D) -> Point3D {
        let row = |r: [f64; 4]| r[0] * p.x + r[1] * p.y + r[2] * p.z + r[3];
        Point3D::new(row(self.m[0]), row(self.m[1]), row(self.m[2]))
    }

    pub(crate) fn inverse(&self) -> Result<Self> {
        if self.m.iter().flatten().any(|value| !value.is_finite()) {
            return Err(Error::invalid("Nonfinite image affine"));
        }
        let [[a, b, c, _], [d, e, f, _], [g, h, i, _]] = self.m;
        let det = self.determinant();
        let scale = Point3D::new(a, d, g).length()
            * Point3D::new(b, e, h).length()
            * Point3D::new(c, f, i).length();
        if !scale.is_finite() || scale == 0.0 || !det.is_finite() || det.abs() / scale < 1e-10 {
            return Err(Error::invalid("Singular image affine"));
        }
        let mut inverse = Self {
            m: [
                [
                    (e * i - f * h) / det,
                    (c * h - b * i) / det,
                    (b * f - c * e) / det,
                    0.0,
                ],
                [
                    (f * g - d * i) / det,
                    (a * i - c * g) / det,
                    (c * d - a * f) / det,
                    0.0,
                ],
                [
                    (d * h - e * g) / det,
                    (b * g - a * h) / det,
                    (a * e - b * d) / det,
                    0.0,
                ],
            ],
        };
        let t = inverse.point(Point3D::new(self.m[0][3], self.m[1][3], self.m[2][3]));
        for (row, value) in inverse.m.iter_mut().zip([t.x, t.y, t.z]) {
            row[3] = -value;
        }
        Ok(inverse)
    }

    /// Determinant of the 3x3 matrix without the last implicit row.
    pub(crate) fn determinant(&self) -> f64 {
        let [[a, b, c, _], [d, e, f, _], [g, h, i, _]] = self.m;
        a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Bounds3D {
    pub(crate) min: Point3D,
    pub(crate) max: Point3D,
}

impl Bounds3D {
    pub(crate) const EMPTY: Self = Self {
        min: Point3D::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
        max: Point3D::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
    };

    pub(crate) fn include(&mut self, p: Point3D) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    pub(crate) fn extent(&self) -> Point3D {
        self.max - self.min
    }
}

/// Slice though world space
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlicePlane {
    pub origin: Point3D,
    pub u: Point3D,
    pub v: Point3D,
}

impl SlicePlane {
    pub fn point(&self, s: f64, t: f64) -> Point3D {
        self.origin + self.u * s + self.v * t
    }

    pub fn project(&self, p: Point3D) -> (f64, f64) {
        let d = p - self.origin;
        (
            d.dot(self.u) / self.u.dot(self.u),
            d.dot(self.v) / self.v.dot(self.v),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewPlane {
    Axial,
    Coronal,
    Sagittal,
}

impl ViewPlane {
    pub const ALL: [Self; 3] = [Self::Axial, Self::Coronal, Self::Sagittal];

    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GeometryOptions {
    /// Ignore the sform/qform and show the image on its own voxel axes
    pub native: bool,
    /// Treat unknown spatial units as mm
    pub assume_mm: bool,
}

/// Where a volumes voxels sit in world space in mm
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeGeometry {
    pub(crate) voxel_to_world: Affine,
    pub(crate) world_to_voxel: Affine,
    pub(crate) bounds: Bounds3D,
    pub(crate) cursor_bounds: Bounds3D,
    pub(crate) dimensions: [usize; 3],
    /// True when placed by an sform/qform in known units
    pub anatomical: bool,
    pub(crate) units_known: bool,
    pub transform_name: &'static str,
    pub unit_name: &'static str,
}

impl VolumeGeometry {
    pub(crate) fn new(meta: &VolumeMeta, options: GeometryOptions) -> Result<Self> {
        let (factor, units_known, unit_name) = match meta.units {
            SpatialUnits::Meter => (1000.0, true, "meters"),
            SpatialUnits::Millimeter => (1.0, true, "mm"),
            SpatialUnits::Micron => (0.001, true, "microns"),
            SpatialUnits::Unknown if options.assume_mm => (1.0, true, "assumed mm"),
            SpatialUnits::Unknown => (1.0, false, "unknown"),
        };

        // prefer sform, then qform
        let transform = if options.native || !units_known {
            None
        } else {
            meta.transform.as_ref()
        };
        let anatomical = transform.is_some();
        let (voxel_to_world, transform_name) = match transform {
            Some(transform) => {
                let m = transform.matrix.map(|row| row.map(|value| value * factor));
                (Affine { m }, transform.name)
            }
            None => {
                let mut affine = Affine::IDENTITY;
                for (axis, row) in affine.m.iter_mut().enumerate() {
                    let spacing = meta.spacing[axis].abs();
                    let spacing = if spacing.is_finite() && spacing > 0.0 {
                        spacing
                    } else {
                        1.0
                    };

                    row[axis] = spacing * factor;
                }
                (affine, "native voxel axes")
            }
        };
        let world_to_voxel = voxel_to_world.inverse()?;
        let dimensions = [meta.width(), meta.height(), meta.depth()];

        let mut bounds = Bounds3D::EMPTY;
        let mut cursor_bounds = Bounds3D::EMPTY;
        for corner in 0..8 {
            let voxel = |inset: f64| {
                Point3D::from_array(std::array::from_fn(|axis| {
                    if corner & (1 << axis) != 0 {
                        dimensions[axis] as f64 - 0.5 - inset
                    } else {
                        inset - 0.5
                    }
                }))
            };
            let edge = voxel_to_world.point(voxel(0.0));
            if !edge.is_finite() {
                return Err(Error::invalid("invalid image FoV"));
            }
            bounds.include(edge);
            cursor_bounds.include(voxel_to_world.point(voxel(0.5)));
        }

        Ok(Self {
            voxel_to_world,
            world_to_voxel,
            bounds,
            cursor_bounds,
            dimensions,
            anatomical,
            units_known,
            transform_name,
            unit_name,
        })
    }

    pub(crate) fn contains_voxel(&self, p: Point3D) -> bool {
        p.is_finite()
            && (0..3).all(|axis| p[axis] >= -0.5 && p[axis] < self.dimensions[axis] as f64 - 0.5)
    }

    pub fn contains_world(&self, p: Point3D) -> bool {
        self.contains_voxel(self.world_to_voxel.point(p))
    }

    /// Voxel whose center is the closest to the world point `p` if `p` is in the volume.
    pub(crate) fn nearest_voxel(&self, p: Point3D) -> Option<[usize; 3]> {
        let voxel = self.world_to_voxel.point(p);
        if !self.contains_voxel(voxel) {
            return None;
        }
        Some([0, 1, 2].map(|axis| (voxel[axis] + 0.5).floor() as usize))
    }

    pub(crate) fn plane(&self, axis: ViewPlane, cursor: Point3D, radiological: bool) -> SlicePlane {
        let min = self.bounds.min;
        let extent = self.bounds.extent();
        let mut plane = match axis {
            ViewPlane::Axial => SlicePlane {
                origin: Point3D::new(min.x, min.y, cursor.z),
                u: Point3D::new(extent.x, 0.0, 0.0),
                v: Point3D::new(0.0, extent.y, 0.0),
            },
            ViewPlane::Coronal => SlicePlane {
                origin: Point3D::new(min.x, cursor.y, min.z),
                u: Point3D::new(extent.x, 0.0, 0.0),
                v: Point3D::new(0.0, 0.0, extent.z),
            },
            ViewPlane::Sagittal => SlicePlane {
                origin: Point3D::new(cursor.x, min.y, min.z),
                u: Point3D::new(0.0, extent.y, 0.0),
                v: Point3D::new(0.0, 0.0, extent.z),
            },
        };
        if self.anatomical && radiological && axis != ViewPlane::Sagittal {
            plane.origin = plane.origin + plane.u;
            plane.u = -plane.u;
        }
        plane
    }
}

pub(crate) fn aspect_scale(
    view_w: f32,
    view_h: f32,
    physical_w: f64,
    physical_h: f64,
) -> (f32, f32) {
    if view_w <= 0.0 || view_h <= 0.0 || physical_w <= 0.0 || physical_h <= 0.0 {
        return (1.0, 1.0);
    }
    let view = f64::from(view_w) / f64::from(view_h);
    let physical = physical_w / physical_h;
    if view > physical {
        ((physical / view) as f32, 1.0)
    } else {
        (1.0, (view / physical) as f32)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::meta::{DataFile, Format, Transform};
    use crate::voxel::{DataType, Endian, Scaling};

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-6
    }

    fn near_point(a: Point3D, b: Point3D) -> bool {
        (a - b).length() <= 1e-6
    }

    fn header(sform: Option<Mat3x4>) -> VolumeMeta {
        VolumeMeta {
            format: Format::Nifti1,
            dims: [4, 3, 4, 1],
            spacing: [1.0; 3],
            units: SpatialUnits::Millimeter,
            transform: sform.map(|matrix| Transform {
                name: "sform",
                matrix,
            }),
            datatype: DataType::I16,
            endian: Endian::Little,
            scaling: Scaling::default(),
            display_range: None,
            data: DataFile {
                path: PathBuf::from("test.nii"),
                offset: 352,
                bytes: 4 * 3 * 4 * 2,
                gzip: false,
                start: 0,
            },
        }
    }

    fn geometry(meta: &VolumeMeta) -> VolumeGeometry {
        VolumeGeometry::new(meta, GeometryOptions::default()).unwrap()
    }

    const FLIP: Mat3x4 = [
        [-2.0, 0.0, 0.0, 30.0],
        [0.0, 3.0, 0.0, -20.0],
        [0.0, 0.0, 4.0, 10.0],
    ];

    #[test]
    fn point_arithmetic() {
        let mut p = Point3D::new(1.0, 2.0, 3.0);
        assert_eq!(
            p + p * 2.0 - Point3D::new(3.0, 6.0, 9.0),
            Point3D::default()
        );
        assert_eq!(-p, Point3D::new(-1.0, -2.0, -3.0));
        assert_eq!(Point3D::new(3.0, 4.0, 0.0).length(), 5.0);
        assert_eq!(
            p.cross(Point3D::new(4.0, 5.0, 6.0)),
            Point3D::new(-3.0, 6.0, -3.0)
        );
        p[2] = 7.0;
        assert_eq!(p.z, 7.0);
    }

    #[test]
    fn applies_and_inverts_affine() {
        let geo = geometry(&header(Some(FLIP)));
        let p = geo.voxel_to_world.point(Point3D::new(1.0, 2.0, 3.0));
        assert_eq!(p, Point3D::new(28.0, -14.0, 22.0));
        assert!(near_point(
            geo.world_to_voxel.point(p),
            Point3D::new(1.0, 2.0, 3.0)
        ));
        assert!(geo.anatomical);
        assert_eq!(geo.transform_name, "sform");
    }

    #[test]
    fn bounds_cover_voxel_edges() {
        let geo = geometry(&header(Some(FLIP)));
        assert_eq!((geo.bounds.min.x, geo.bounds.max.x), (23.0, 31.0));
        assert_eq!(
            (geo.cursor_bounds.min.x, geo.cursor_bounds.max.x),
            (24.0, 30.0)
        );
        assert!(!geo.contains_world(Point3D::new(10000.0, 0.0, 0.0)));
        assert!(geo.contains_voxel(Point3D::new(-0.5, 0.0, 0.0)));
        assert!(!geo.contains_voxel(Point3D::new(3.5, 0.0, 0.0)));
    }

    #[test]
    fn radiological_planes_mirror_left_and_right() {
        let geo = geometry(&header(Some(FLIP)));
        let cursor = Point3D::new(28.0, -14.0, 22.0);
        let axial = geo.plane(ViewPlane::Axial, cursor, false);
        let radio = geo.plane(ViewPlane::Axial, cursor, true);
        assert!(axial.u.x > 0.0 && radio.u.x < 0.0);
        assert!(near(axial.point(0.2, 0.5).x, radio.point(0.8, 0.5).x));
        // Sagittal views are never mirrored.
        assert_eq!(
            geo.plane(ViewPlane::Sagittal, cursor, false),
            geo.plane(ViewPlane::Sagittal, cursor, true)
        );
        let (s, t) = axial.project(axial.point(0.2, 0.3));
        assert!(near(s, 0.2) && near(t, 0.3));
    }

    #[test]
    fn converts_meters_to_millimeters() {
        let mut meters = header(Some(FLIP.map(|row| row.map(|value| value * 0.001))));
        meters.units = SpatialUnits::Meter;
        let p = geometry(&meters)
            .voxel_to_world
            .point(Point3D::new(1.0, 2.0, 3.0));
        assert!(near_point(p, Point3D::new(28.0, -14.0, 22.0)));
    }

    #[test]
    fn handles_permuted_and_oblique_axes() {
        let permuted = geometry(&header(Some([
            [0.0, 0.0, -3.0, 30.0],
            [2.0, 0.0, 0.0, -20.0],
            [0.0, 4.0, 0.0, -10.0],
        ])));
        assert_eq!(
            permuted.voxel_to_world.point(Point3D::new(1.0, 1.0, 1.0)),
            Point3D::new(27.0, -18.0, -6.0)
        );

        let (sin, cos) = 0.4f64.sin_cos();
        let oblique = geometry(&header(Some([
            [cos, -sin, 0.0, 5.0],
            [sin, cos, 0.0, -5.0],
            [0.0, 0.0, 1.0, 0.0],
        ])));
        let p = Point3D::new(1.25, 1.5, 2.75);
        assert!(near_point(
            oblique
                .world_to_voxel
                .point(oblique.voxel_to_world.point(p)),
            p
        ));
    }

    #[test]
    fn unknown_units_are_not_assumed() {
        let assume_mm = GeometryOptions {
            assume_mm: true,
            ..Default::default()
        };
        let mut h = header(Some(FLIP));
        h.units = SpatialUnits::Unknown;
        let geo = geometry(&h);
        assert!(!geo.anatomical);
        assert_eq!(geo.unit_name, "unknown");

        let assumed = VolumeGeometry::new(&h, assume_mm).unwrap();
        assert!(assumed.anatomical);
        assert_eq!(assumed.unit_name, "assumed mm");

        // Without any transform, assuming millimeters still gives native axes.
        h.transform = None;
        assert!(!VolumeGeometry::new(&h, assume_mm).unwrap().anatomical);
    }

    #[test]
    fn native_axes_use_voxel_spacing() {
        let mut h = header(Some(FLIP));
        h.spacing = [-2.0, 3.0, 0.0];
        let native = GeometryOptions {
            native: true,
            ..Default::default()
        };
        let geo = VolumeGeometry::new(&h, native).unwrap();
        assert!(!geo.anatomical);
        assert_eq!(geo.transform_name, "native voxel axes");
        // |-2| = 2, and a spacing of 0 falls back to 1.
        assert_eq!(
            geo.voxel_to_world.point(Point3D::new(1.0, 1.0, 1.0)),
            Point3D::new(2.0, 3.0, 1.0)
        );
    }

    #[test]
    fn rejects_singular_affine() {
        let mut singular = FLIP;
        singular[2] = [0.0; 4];
        assert!(VolumeGeometry::new(&header(Some(singular)), GeometryOptions::default()).is_err());
    }

    #[test]
    fn aspect_scale_fits_inside_panel() {
        assert_eq!(aspect_scale(200.0, 100.0, 1.0, 1.0), (0.5, 1.0));
        assert_eq!(aspect_scale(100.0, 200.0, 1.0, 1.0), (1.0, 0.5));
        assert_eq!(aspect_scale(0.0, 100.0, 1.0, 1.0), (1.0, 1.0));
    }
}
