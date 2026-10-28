//! Maximum intensity projection.

use rayon::prelude::*;

use crate::geometry::{Point3D, SlicePlane, VolumeGeometry};
use crate::volume::Volume;
use crate::voxel::{Scalar, dispatch_data_type};

pub(crate) struct MaximumIntensityProjection {
    pub(crate) frame: usize,
    pub(crate) axis: usize,
    dimensions: [usize; 2],
    maximum_values: Vec<f32>,
    maximum_depths: Vec<u32>,
}

impl MaximumIntensityProjection {
    pub(crate) fn of(volume: &Volume, frame: usize, axis: usize) -> Self {
        let frame = volume.clamp_frame(frame);
        let metadata = volume.metadata();
        let (maximum_values, maximum_depths) =
            dispatch_data_type!(metadata.data_type, |T| project::<T>(volume, frame, axis));
        let [a, b] = perpendicular_axes(axis);
        Self {
            frame,
            axis,
            dimensions: [metadata.dimensions[a], metadata.dimensions[b]],
            maximum_values,
            maximum_depths,
        }
    }

    pub(crate) fn sample_nearest(&self, voxel: Point3D) -> f32 {
        let [i, j] = self.grid_cell(voxel, 0.5);
        self.maximum_values[i + self.dimensions[0] * j]
    }

    pub(crate) fn sample_bilinear(&self, voxel: Point3D) -> f32 {
        let [a, b] = perpendicular_axes(self.axis);
        let (x, y) = (voxel[a].max(0.0), voxel[b].max(0.0));
        let [i, j] = self.grid_cell(voxel, 0.0);
        let (i1, j1) = (
            (i + 1).min(self.dimensions[0] - 1),
            (j + 1).min(self.dimensions[1] - 1),
        );
        let (fx, fy) = ((x - i as f64) as f32, (y - j as f64) as f32);
        let at = |i: usize, j: usize| self.maximum_values[i + self.dimensions[0] * j];
        let corners = [at(i, j), at(i1, j), at(i, j1), at(i1, j1)];
        if corners.iter().any(|value| value.is_nan()) {
            // interpolating with a line without a value spreads the gap
            return self.sample_nearest(voxel);
        }
        let top = corners[0] * (1.0 - fx) + corners[1] * fx;
        let bottom = corners[2] * (1.0 - fx) + corners[3] * fx;
        top * (1.0 - fy) + bottom * fy
    }

    pub(crate) fn maximum_voxel(&self, voxel: Point3D) -> Option<[usize; 3]> {
        let [i, j] = self.grid_cell(voxel, 0.5);
        let index = i + self.dimensions[0] * j;
        if self.maximum_values[index].is_nan() {
            return None;
        }
        let [a, b] = perpendicular_axes(self.axis);
        let mut found = [0; 3];
        found[a] = i;
        found[b] = j;
        found[self.axis] = self.maximum_depths[index] as usize;
        Some(found)
    }

    fn grid_cell(&self, voxel: Point3D, rounding_offset: f64) -> [usize; 2] {
        let [a, b] = perpendicular_axes(self.axis);
        let index = |coordinate: f64, size: usize| {
            ((coordinate + rounding_offset).floor().max(0.0) as usize).min(size - 1)
        };
        [
            index(voxel[a], self.dimensions[0]),
            index(voxel[b], self.dimensions[1]),
        ]
    }
}

pub(crate) fn perpendicular_axes(axis: usize) -> [usize; 2] {
    match axis {
        0 => [1, 2],
        1 => [0, 2],
        _ => [0, 1],
    }
}

pub(crate) fn projection_axis(geometry: &VolumeGeometry, plane: &SlicePlane) -> usize {
    let normal = plane.u.cross(plane.v);
    let to_voxel = &geometry.world_to_voxel;
    let step =
        to_voxel.transform_point(plane.origin + normal) - to_voxel.transform_point(plane.origin);
    (0..3)
        .max_by(|&a, &b| step[a].abs().total_cmp(&step[b].abs()))
        .unwrap_or(2)
}

fn project<T: Scalar>(volume: &Volume, frame: usize, axis: usize) -> (Vec<f32>, Vec<u32>) {
    let metadata = volume.metadata();
    let bytes = volume.frame_bytes(frame);
    let (scaling, endian) = (metadata.scaling, metadata.endian);
    let dimensions = [
        metadata.dimensions[0],
        metadata.dimensions[1],
        metadata.dimensions[2],
    ];
    let strides = [1, dimensions[0], dimensions[0] * dimensions[1]];
    let [a, b] = perpendicular_axes(axis);
    let mut maximum_values = vec![f32::NAN; dimensions[a] * dimensions[b]];
    let mut maximum_depths = vec![0u32; dimensions[a] * dimensions[b]];
    // one line of the projection per task
    maximum_values
        .par_chunks_mut(dimensions[a])
        .zip(maximum_depths.par_chunks_mut(dimensions[a]))
        .enumerate()
        .for_each(|(j, (line_maximums, line_depths))| {
            for k in 0..dimensions[axis] {
                for i in 0..dimensions[a] {
                    let index = i * strides[a] + j * strides[b] + k * strides[axis];
                    let raw = T::read(&bytes[index * T::SIZE..(index + 1) * T::SIZE], endian);
                    let value = scaling.apply(raw) as f32;
                    if value.is_finite() && (line_maximums[i].is_nan() || value > line_maximums[i])
                    {
                        line_maximums[i] = value;
                        line_depths[i] = k as u32;
                    }
                }
            }
        });
    (maximum_values, maximum_depths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{GeometryOptions, ViewPlane};
    use crate::test_fixtures::{Fixture, write_and_load};

    #[test]
    fn projects_the_highest_value_along_each_line() {
        let directory = tempfile::tempdir().unwrap();
        let volume = write_and_load(&directory, &Fixture::default(), "values.nii");
        let mip = MaximumIntensityProjection::of(&volume, 0, 1);
        assert_eq!(mip.dimensions, [4, 4]);
        assert_eq!(mip.sample_nearest(Point3D::new(1.0, 0.0, 3.0)), 45.0);
        assert_eq!(
            mip.maximum_voxel(Point3D::new(1.0, 0.0, 3.0)),
            Some([1, 2, 3])
        );
        assert_eq!(mip.sample_bilinear(Point3D::new(1.5, 0.0, 3.0)), 45.5);

        let peaked = Fixture {
            value: |i| if i == 17 { 100.0 } else { 0.0 },
            ..Fixture::default()
        };
        let volume = write_and_load(&directory, &peaked, "peaked.nii");
        let mip = MaximumIntensityProjection::of(&volume, 0, 2);
        assert_eq!(
            mip.maximum_voxel(Point3D::new(1.0, 1.0, 0.0)),
            Some([1, 1, 1])
        );
        assert_eq!(mip.sample_nearest(Point3D::new(0.0, 0.0, 0.0)), 0.0);
    }

    #[test]
    fn looks_along_the_axis_closest_to_the_view() {
        let directory = tempfile::tempdir().unwrap();
        let volume = write_and_load(&directory, &Fixture::default(), "values.nii");
        let geometry = volume.geometry(GeometryOptions::default()).unwrap();
        let cursor = Point3D::new(1.0, 1.0, 1.0);
        let axis = |plane| projection_axis(&geometry, &geometry.plane(plane, cursor, false));
        assert_eq!(axis(ViewPlane::Sagittal), 0);
        assert_eq!(axis(ViewPlane::Coronal), 1);
        assert_eq!(axis(ViewPlane::Axial), 2);
    }
}
