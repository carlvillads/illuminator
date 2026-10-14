//! Zoom and pan inside a view.

use illuminator_core::geometry::SlicePlane;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zoom {
    pub scale: f64,
    center: (f64, f64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Visible {
    origin: (f64, f64),
    /// Width and height
    extent: (f64, f64),
}

impl Default for Zoom {
    fn default() -> Self {
        Self {
            scale: 1.0,
            center: (0.5, 0.5),
        }
    }
}

impl Zoom {
    pub const MAX: f64 = 32.0;

    pub fn visible(self, fit: (f64, f64)) -> Visible {
        let extent = |fit: f64| (1.0 / (self.scale * fit)).min(1.0);
        let extent = (extent(fit.0), extent(fit.1));
        let origin = |center: f64, extent: f64| (center - extent / 2.0).clamp(0.0, 1.0 - extent);
        Visible {
            origin: (
                origin(self.center.0, extent.0),
                origin(self.center.1, extent.1),
            ),
            extent,
        }
    }

    pub fn zoom_at(self, factor: f64, anchor: (f64, f64), fit: (f64, f64)) -> Self {
        let fixed = self.visible(fit).to_plane(anchor);
        let zoomed = Self {
            scale: (self.scale * factor).clamp(1.0, Self::MAX),
            ..self
        };
        let extent = zoomed.visible(fit).extent;
        zoomed.starting_at(
            (fixed.0 - anchor.0 * extent.0, fixed.1 - anchor.1 * extent.1),
            fit,
        )
    }

    pub fn pan(self, (ds, dt): (f64, f64), fit: (f64, f64)) -> Self {
        let visible = self.visible(fit);
        self.starting_at(
            (
                visible.origin.0 - ds * visible.extent.0,
                visible.origin.1 - dt * visible.extent.1,
            ),
            fit,
        )
    }

    fn starting_at(self, origin: (f64, f64), fit: (f64, f64)) -> Self {
        let extent = self.visible(fit).extent;
        let center = |origin: f64, extent: f64| origin.clamp(0.0, 1.0 - extent) + extent / 2.0;
        Self {
            center: (center(origin.0, extent.0), center(origin.1, extent.1)),
            ..self
        }
    }
}

impl Visible {
    pub fn apply(self, plane: SlicePlane) -> SlicePlane {
        SlicePlane {
            origin: plane.point(self.origin.0, self.origin.1),
            u: plane.u * self.extent.0,
            v: plane.v * self.extent.1,
        }
    }

    pub fn to_plane(self, (s, t): (f64, f64)) -> (f64, f64) {
        (
            self.origin.0 + s * self.extent.0,
            self.origin.1 + t * self.extent.1,
        )
    }

    pub fn to_view(self, (s, t): (f64, f64)) -> (f64, f64) {
        (
            (s - self.origin.0) / self.extent.0,
            (t - self.origin.1) / self.extent.1,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use illuminator_core::geometry::Point3D;

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-12 && (a.1 - b.1).abs() < 1e-12
    }

    const TALL: (f64, f64) = (1.0 / 3.0, 1.0);

    #[test]
    fn zooming_first_fills_the_free_space() {
        let zoom = Zoom::default().zoom_at(2.0, (0.5, 0.5), TALL);
        let visible = zoom.visible(TALL);
        assert!(close(visible.extent, (1.0, 0.5)));
        let visible = Zoom::default().zoom_at(6.0, (0.5, 0.5), TALL).visible(TALL);
        assert!(close(visible.extent, (0.5, 1.0 / 6.0)));
    }

    #[test]
    fn zooming_keeps_the_anchor_in_place() {
        let anchor = (0.25, 0.75);
        let zoom = Zoom::default().zoom_at(4.0, anchor, (1.0, 1.0));
        assert_eq!(zoom.scale, 4.0);
        let visible = zoom.visible((1.0, 1.0));
        assert!(close(visible.to_plane(anchor), anchor));
        assert!(close(
            visible.to_view(visible.to_plane((0.1, 0.9))),
            (0.1, 0.9)
        ));
        assert_eq!(zoom.zoom_at(0.01, anchor, (1.0, 1.0)), Zoom::default());
    }

    #[test]
    fn panning_stops_at_the_edges() {
        let fit = (1.0, 1.0);
        let zoom = Zoom::default().zoom_at(2.0, (0.5, 0.5), fit);
        assert!(close(zoom.visible(fit).to_plane((0.0, 0.0)), (0.25, 0.25)));
        let panned = zoom.pan((0.5, 0.0), fit).visible(fit);
        assert!(close(panned.to_plane((0.0, 0.0)), (0.0, 0.25)));
        let panned = zoom.pan((5.0, -5.0), fit).visible(fit);
        assert!(close(panned.to_plane((0.0, 0.0)), (0.0, 0.5)));
    }

    #[test]
    fn the_visible_plane_is_a_part_of_the_whole() {
        let plane = SlicePlane {
            origin: Point3D::new(0.0, 0.0, 5.0),
            u: Point3D::new(100.0, 0.0, 0.0),
            v: Point3D::new(0.0, 80.0, 0.0),
        };
        let fit = (1.0, 1.0);
        let visible = Zoom::default().zoom_at(4.0, (0.5, 0.5), fit).visible(fit);
        let part = visible.apply(plane);
        assert_eq!(part.origin, plane.point(0.375, 0.375));
        assert_eq!(part.u, Point3D::new(25.0, 0.0, 0.0));
        assert_eq!(part.v, Point3D::new(0.0, 20.0, 0.0));
    }
}
