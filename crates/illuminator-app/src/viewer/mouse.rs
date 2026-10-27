//! Clicking, dragging and scrolling in the views.

use gpui_kit::{Context, Pixels, Point, ScrollWheelEvent, Size, Window, px};
use illuminator_core::geometry::ViewPlane;

use super::{Layout, Viewer};

const PIXELS_PER_SCROLL_STEP: f32 = 20.0;
const PIXELS_PER_ZOOM_DOUBLING: f64 = 200.0;

#[derive(Clone, Copy)]
pub(super) struct PanDrag {
    axis: ViewPlane,
    last_position: Point<Pixels>,
    image_size: Size<f32>,
    fit: (f64, f64),
}

impl Viewer {
    pub(super) fn toggle_maximize(
        &mut self,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.layout, Layout::Single { .. }) {
            self.set_layout(self.previous_layout, cx);
        } else if let Some(hit) = self.hit(position, window) {
            let layer = (!self.layout.is_overlaid()).then_some(hit.base);
            self.set_layout(
                Layout::Single {
                    axis: hit.axis,
                    layer,
                },
                cx,
            );
        }
    }

    /// Remembers the world position under the mouse
    pub(super) fn set_hovered_point(
        &mut self,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let hover = self
            .hit(position, window)
            .filter(|hit| (0.0..1.0).contains(&hit.s) && (0.0..1.0).contains(&hit.t))
            .map(|hit| hit.plane.point(hit.s, hit.t));
        if hover != self.hovered_point {
            self.hovered_point = hover;
            cx.notify();
        }
    }

    /// Focuses the clicked view.
    pub(super) fn set_active_panel(
        &mut self,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(hit) = self.hit(position, window) {
            let panel = (hit.base, hit.axis);
            if panel != self.active_panel {
                if !self.layout.is_overlaid() {
                    self.selected_layer = hit.base;
                }
                self.active_panel = panel;
                cx.notify();
            }
        }
    }

    pub(super) fn select_point_at(
        &mut self,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(hit) = self.hit(position, window) else {
            return;
        };
        let radiological = self.radiological;
        let display = &mut self.layers[hit.base].display;
        let cursor_moved = if hit.mip {
            display.select_maximum_along_view(hit.axis, radiological, hit.s, hit.t)
        } else {
            display.select_point(hit.plane, hit.s, hit.t)
        };
        if cursor_moved {
            self.synchronize_cursors_from(hit.base);
            cx.notify();
        }
    }

    pub(super) fn handle_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(hit) = self.hit(event.position, window) else {
            return;
        };
        if event.modifiers.secondary() {
            let pixels = f64::from(f32::from(
                event.delta.pixel_delta(px(PIXELS_PER_SCROLL_STEP)).y,
            ));
            let zoom = &mut self.zoom[hit.axis.index()];
            // 2^x should make the zooming feel even across different zooms
            *zoom = zoom.zoom_at(
                (pixels / PIXELS_PER_ZOOM_DOUBLING).exp2(),
                hit.view_position,
                hit.fit,
            );
            cx.notify();
            return;
        }
        self.scroll_remainder += f32::from(event.delta.pixel_delta(px(PIXELS_PER_SCROLL_STEP)).y);
        let steps = (self.scroll_remainder / PIXELS_PER_SCROLL_STEP).trunc();
        if steps == 0.0 {
            return;
        }
        self.scroll_remainder -= steps * PIXELS_PER_SCROLL_STEP;
        let steps = f64::from(steps) * self.slice_stride();
        if self.layers[hit.base].display.step_slice(hit.axis, steps) {
            self.synchronize_cursors_from(hit.base);
            cx.notify();
        }
    }

    pub(super) fn start_pan(&mut self, position: Point<Pixels>, window: &Window) {
        self.pan_drag = self.hit(position, window).map(|hit| PanDrag {
            axis: hit.axis,
            last_position: position,
            image_size: hit.image_size,
            fit: hit.fit,
        });
    }

    /// moves the zoomed view with mouse
    pub(super) fn update_pan(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.pan_drag else {
            return;
        };
        let dx = f64::from(f32::from(position.x - drag.last_position.x) / drag.image_size.width);
        // y is downwards
        let dy = -f64::from(f32::from(position.y - drag.last_position.y) / drag.image_size.height);
        let zoom = &mut self.zoom[drag.axis.index()];
        *zoom = zoom.pan((dx, dy), drag.fit);
        self.pan_drag = Some(PanDrag {
            last_position: position,
            ..drag
        });
        cx.notify();
    }

    pub(super) fn drag_window(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((index, last)) = self.window_drag else {
            return;
        };
        let dx = f64::from(f32::from(position.x - last.x));
        let dy = f64::from(f32::from(position.y - last.y));
        self.layers[index].display.adjust_window(dx, dy);
        self.window_drag = Some((index, position));
        cx.notify();
    }
}
