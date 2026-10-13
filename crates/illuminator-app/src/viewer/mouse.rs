//! Clicking, dragging, scrolling, dragging

use super::*;

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
            let layer = (!self.layout.overlaid()).then_some(hit.base);
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
    pub(super) fn set_hover(
        &mut self,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let hover = self
            .hit(position, window)
            .filter(|hit| (0.0..1.0).contains(&hit.s) && (0.0..1.0).contains(&hit.t))
            .map(|hit| hit.plane.point(hit.s, hit.t));
        if hover != self.hover {
            self.hover = hover;
            cx.notify();
        }
    }

    pub(super) fn set_active_panel(
        &mut self,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(hit) = self.hit(position, window) {
            let panel = (hit.base, hit.axis);
            if panel != self.active_panel {
                if !self.layout.overlaid() {
                    self.selected = hit.base;
                }
                self.active_panel = panel;
                cx.notify();
            }
        }
    }

    pub(super) fn pick(
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
        let picked = if hit.mip {
            display.pick_brightest(hit.axis, radiological, hit.s, hit.t)
        } else {
            display.pick_on(hit.plane, hit.s, hit.t)
        };
        if picked {
            self.sync_from(hit.base);
            cx.notify();
        }
    }

    pub(super) fn scroll(
        &mut self,
        event: &ScrollWheelEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(hit) = self.hit(event.position, window) else {
            return;
        };
        if event.modifiers.secondary() {
            let pixels = f64::from(f32::from(event.delta.pixel_delta(px(SCROLL_STEP)).y));
            let zoom = &mut self.zoom[hit.axis.index()];
            // 2^x should make the zooming feel even across different zooms
            *zoom = zoom.zoom_at((pixels / ZOOM_SPEED).exp2(), hit.in_view, hit.fit);
            cx.notify();
            return;
        }
        self.scroll_remainder += f32::from(event.delta.pixel_delta(px(SCROLL_STEP)).y);
        let steps = (self.scroll_remainder / SCROLL_STEP).trunc();
        if steps == 0.0 {
            return;
        }
        self.scroll_remainder -= steps * SCROLL_STEP;
        let steps = f64::from(steps) * self.slice_stride();
        if self.layers[hit.base].display.step_slice(hit.axis, steps) {
            self.sync_from(hit.base);
            cx.notify();
        }
    }

    pub(super) fn start_pan(&mut self, position: Point<Pixels>, window: &Window) {
        self.pan_drag = self.hit(position, window).map(|hit| PanDrag {
            axis: hit.axis,
            last: position,
            image_size: hit.image_size,
            fit: hit.fit,
        });
    }

    /// moves the zoomed view with mouse
    pub(super) fn drag_pan(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.pan_drag else {
            return;
        };
        let dx = f64::from(f32::from(position.x - drag.last.x) / drag.image_size.width);
        // y is downwards
        let dy = -f64::from(f32::from(position.y - drag.last.y) / drag.image_size.height);
        let zoom = &mut self.zoom[drag.axis.index()];
        *zoom = zoom.pan((dx, dy), drag.fit);
        self.pan_drag = Some(PanDrag {
            last: position,
            ..drag
        });
        cx.notify();
    }

    pub(super) fn drag_contrast(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some((column, last)) = self.contrast_drag else {
            return;
        };
        let dx = f64::from(f32::from(position.x - last.x));
        let dy = f64::from(f32::from(position.y - last.y));
        self.layers[column].display.drag_window(dx, dy);
        self.contrast_drag = Some((column, position));
        cx.notify();
    }
}
