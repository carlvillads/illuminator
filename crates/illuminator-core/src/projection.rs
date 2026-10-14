//! Maximum intensity projection.

use rayon::prelude::*;

use crate::geometry::{Point3D, SlicePlane, VolumeGeometry};
use crate::volume::Volume;
use crate::voxel::{Scalar, with_scalar};

pub(crate) struct Projection {
    pub(crate) frame: usize,
    pub(crate) axis: usize,
    dims: [usize; 2],
    max: Vec<f32>,
    depth_of_max: Vec<u32>,
}

impl Projection {
    pub(crate) fn of(volume: &Volume, frame: usize, axis: usize) -> Self {
        let frame = volume.clamp_frame(frame);
        let meta = volume.meta();
        let (max, depth_of_max) =
            with_scalar!(meta.datatype, |T| project::<T>(volume, frame, axis));
        let [a, b] = other_axes(axis);
        Self {
            frame,
            axis,
            dims: [meta.dims[a], meta.dims[b]],
            max,
            depth_of_max,
        }
    }

    pub(crate) fn nearest(&self, voxel: Point3D) -> f32 {
        let [i, j] = self.cell(voxel, 0.5);
        self.max[i + self.dims[0] * j]
    }

    pub(crate) fn linear(&self, voxel: Point3D) -> f32 {
        let [a, b] = other_axes(self.axis);
        let (x, y) = (voxel[a].max(0.0), voxel[b].max(0.0));
        let [i, j] = self.cell(voxel, 0.0);
        let (i1, j1) = ((i + 1).min(self.dims[0] - 1), (j + 1).min(self.dims[1] - 1));
        let (fx, fy) = ((x - i as f64) as f32, (y - j as f64) as f32);
        let at = |i: usize, j: usize| self.max[i + self.dims[0] * j];
        let corners = [at(i, j), at(i1, j), at(i, j1), at(i1, j1)];
        if corners.iter().any(|value| value.is_nan()) {
            // interpolating with a line without a value spreads the gap
            return self.nearest(voxel);
        }
        let top = corners[0] * (1.0 - fx) + corners[1] * fx;
        let bottom = corners[2] * (1.0 - fx) + corners[3] * fx;
        top * (1.0 - fy) + bottom * fy
    }

    pub(crate) fn brightest(&self, voxel: Point3D) -> Option<[usize; 3]> {
        let [i, j] = self.cell(voxel, 0.5);
        let index = i + self.dims[0] * j;
        if self.max[index].is_nan() {
            return None;
        }
        let [a, b] = other_axes(self.axis);
        let mut found = [0; 3];
        found[a] = i;
        found[b] = j;
        found[self.axis] = self.depth_of_max[index] as usize;
        Some(found)
    }

    fn cell(&self, voxel: Point3D, round: f64) -> [usize; 2] {
        let [a, b] = other_axes(self.axis);
        let index = |coordinate: f64, size: usize| {
            ((coordinate + round).floor().max(0.0) as usize).min(size - 1)
        };
        [index(voxel[a], self.dims[0]), index(voxel[b], self.dims[1])]
    }
}

pub(crate) fn other_axes(axis: usize) -> [usize; 2] {
    match axis {
        0 => [1, 2],
        1 => [0, 2],
        _ => [0, 1],
    }
}

pub(crate) fn depth_axis(geometry: &VolumeGeometry, plane: &SlicePlane) -> usize {
    let normal = plane.u.cross(plane.v);
    let to_voxel = &geometry.world_to_voxel;
    let step = to_voxel.point(plane.origin + normal) - to_voxel.point(plane.origin);
    (0..3)
        .max_by(|&a, &b| step[a].abs().total_cmp(&step[b].abs()))
        .unwrap_or(2)
}

fn project<T: Scalar>(volume: &Volume, frame: usize, axis: usize) -> (Vec<f32>, Vec<u32>) {
    let meta = volume.meta();
    let bytes = volume.frame(frame);
    let (scaling, endian) = (meta.scaling, meta.endian);
    let dims = [meta.dims[0], meta.dims[1], meta.dims[2]];
    let strides = [1, dims[0], dims[0] * dims[1]];
    let [a, b] = other_axes(axis);
    // one line of the projection per task
    let rows: Vec<(Vec<f32>, Vec<u32>)> = (0..dims[b])
        .into_par_iter()
        .map(|j| {
            let mut max = vec![f32::NAN; dims[a]];
            let mut at = vec![0u32; dims[a]];
            for k in 0..dims[axis] {
                for i in 0..dims[a] {
                    let index = i * strides[a] + j * strides[b] + k * strides[axis];
                    let raw = T::read(&bytes[index * T::SIZE..(index + 1) * T::SIZE], endian);
                    let value = scaling.apply(raw) as f32;
                    if value.is_finite() && (max[i].is_nan() || value > max[i]) {
                        max[i] = value;
                        at[i] = k as u32;
                    }
                }
            }
            (max, at)
        })
        .collect();
    rows.into_iter().fold(
        (Vec::with_capacity(dims[a] * dims[b]), Vec::new()),
        |(mut max, mut at), (row_max, row_at)| {
            max.extend(row_max);
            at.extend(row_at);
            (max, at)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{GeometryOptions, ViewPlane};
    use crate::test_util::{Fixture, load};

    #[test]
    fn projects_the_highest_value_along_each_line() {
        let dir = tempfile::tempdir().unwrap();
        let volume = load(&dir, &Fixture::default(), "values.nii");
        let mip = Projection::of(&volume, 0, 1);
        assert_eq!(mip.dims, [4, 4]);
        assert_eq!(mip.nearest(Point3D::new(1.0, 0.0, 3.0)), 45.0);
        assert_eq!(mip.brightest(Point3D::new(1.0, 0.0, 3.0)), Some([1, 2, 3]));
        assert_eq!(mip.linear(Point3D::new(1.5, 0.0, 3.0)), 45.5);

        let peaked = Fixture {
            value: |i| if i == 17 { 100.0 } else { 0.0 },
            ..Fixture::default()
        };
        let volume = load(&dir, &peaked, "peaked.nii");
        let mip = Projection::of(&volume, 0, 2);
        assert_eq!(mip.brightest(Point3D::new(1.0, 1.0, 0.0)), Some([1, 1, 1]));
        assert_eq!(mip.nearest(Point3D::new(0.0, 0.0, 0.0)), 0.0);
    }

    #[test]
    fn looks_along_the_axis_closest_to_the_view() {
        let dir = tempfile::tempdir().unwrap();
        let volume = load(&dir, &Fixture::default(), "values.nii");
        let geometry = volume.geometry(GeometryOptions::default()).unwrap();
        let cursor = Point3D::new(1.0, 1.0, 1.0);
        let axis = |plane| depth_axis(&geometry, &geometry.plane(plane, cursor, false));
        assert_eq!(axis(ViewPlane::Sagittal), 0);
        assert_eq!(axis(ViewPlane::Coronal), 1);
        assert_eq!(axis(ViewPlane::Axial), 2);
    }
}
