use illuminator_core::geometry::SlicePlane;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Zoom {
    pub scale: f64,
    origin: (f64, f64),
}

impl Default for Zoom {
    fn default() -> Self {
        Self {
            scale: 1.0,
            origin: (0.0, 0.0),
        }
    }
}

impl Zoom {
    pub const MAX: f64 = 32.0;

    pub fn apply(self, plane: SlicePlane) -> SlicePlane {
        SlicePlane {
            origin: plane.point(self.origin.0, self.origin.1),
            u: plane.u * (1.0 / self.scale),
            v: plane.v * (1.0 / self.scale),
        }
    }

    pub fn to_plane(self, (s, t): (f64, f64)) -> (f64, f64) {
        (
            self.origin.0 + s / self.scale,
            self.origin.1 + t / self.scale,
        )
    }

    pub fn to_view(self, (s, t): (f64, f64)) -> (f64, f64) {
        (
            (s - self.origin.0) * self.scale,
            (t - self.origin.1) * self.scale,
        )
    }

    pub fn zoom_at(self, factor: f64, anchor: (f64, f64)) -> Self {
        let fixed = self.to_plane(anchor);
        let scale = (self.scale * factor).clamp(1.0, Self::MAX);
        Self {
            scale,
            origin: (fixed.0 - anchor.0 / scale, fixed.1 - anchor.1 / scale),
        }
        .clamped()
    }

    pub fn pan(self, (ds, dt): (f64, f64)) -> Self {
        Self {
            origin: (
                self.origin.0 - ds / self.scale,
                self.origin.1 - dt / self.scale,
            ),
            ..self
        }
        .clamped()
    }

    /// keeps the visible part inside the plane
    fn clamped(self) -> Self {
        let last = 1.0 - 1.0 / self.scale;
        Self {
            origin: (
                self.origin.0.clamp(0.0, last),
                self.origin.1.clamp(0.0, last),
            ),
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use illuminator_core::geometry::Point3D;

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-12 && (a.1 - b.1).abs() < 1e-12
    }

    #[test]
    fn zooming_keeps_the_anchor_in_place() {
        let anchor = (0.25, 0.75);
        let zoom = Zoom::default().zoom_at(4.0, anchor);
        assert_eq!(zoom.scale, 4.0);
        assert!(close(zoom.to_plane(anchor), anchor));
        assert!(close(zoom.to_view(zoom.to_plane((0.1, 0.9))), (0.1, 0.9)));
        assert_eq!(zoom.zoom_at(0.01, anchor), Zoom::default());
    }

    #[test]
    fn panning_stops_at_the_edges() {
        let zoom = Zoom::default().zoom_at(2.0, (0.5, 0.5));
        assert!(close(zoom.to_plane((0.0, 0.0)), (0.25, 0.25)));
        assert!(close(
            zoom.pan((0.5, 0.0)).to_plane((0.0, 0.0)),
            (0.0, 0.25)
        ));
        assert!(close(
            zoom.pan((5.0, -5.0)).to_plane((0.0, 0.0)),
            (0.0, 0.5)
        ));
    }

    #[test]
    fn the_visible_plane_is_a_part_of_the_whole() {
        let plane = SlicePlane {
            origin: Point3D::new(0.0, 0.0, 5.0),
            u: Point3D::new(100.0, 0.0, 0.0),
            v: Point3D::new(0.0, 80.0, 0.0),
        };
        let zoom = Zoom::default().zoom_at(4.0, (0.5, 0.5));
        let visible = zoom.apply(plane);
        assert_eq!(visible.origin, plane.point(0.375, 0.375));
        assert_eq!(visible.u, Point3D::new(25.0, 0.0, 0.0));
        assert_eq!(visible.v, Point3D::new(0.0, 20.0, 0.0));
    }
}
