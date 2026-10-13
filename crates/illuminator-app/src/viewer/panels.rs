//! Drawing the views

use super::*;

impl Viewer {
    pub(super) fn render_panels(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);
        let label_color: Hsla = rgb(ui::ORIENTATION_COLOR).into();
        let scale = window.scale_factor();
        let overlaid = self.layout.overlaid();
        let mut area = div()
            .id("views")
            .relative()
            .flex_1()
            .h_full()
            .bg(gpui::black())
            .on_hover(cx.listener(|viewer, hovered: &bool, _, cx| {
                if !hovered {
                    viewer.hover = None;
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|viewer, event: &MouseDownEvent, window, cx| {
                    viewer.focus_handle.focus(window, cx);
                    if event.click_count == 2 {
                        viewer.toggle_maximize(event.position, window, cx);
                        return;
                    }
                    viewer.set_active_panel(event.position, window, cx);
                    if event.modifiers.alt {
                        viewer.start_pan(event.position, window);
                    } else {
                        viewer.pick(event.position, window, cx);
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|viewer, event: &MouseDownEvent, window, _| {
                    viewer.start_pan(event.position, window);
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|viewer, event: &MouseDownEvent, window, _| {
                    viewer.contrast_drag = viewer.hit(event.position, window).map(|hit| {
                        let layer = if viewer.layout.overlaid() {
                            viewer.selected
                        } else {
                            hit.base
                        };
                        (layer, event.position)
                    });
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|viewer, _: &MouseUpEvent, _, _| viewer.contrast_drag = None),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|viewer, _: &MouseUpEvent, _, _| viewer.pan_drag = None),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|viewer, _: &MouseUpEvent, _, _| viewer.pan_drag = None),
            )
            .on_mouse_move(cx.listener(|viewer, event: &MouseMoveEvent, window, cx| {
                viewer.set_hover(event.position, window, cx);
                if event.pressed_button.is_none() {
                    viewer.set_active_panel(event.position, window, cx);
                }
                match event.pressed_button {
                    Some(MouseButton::Left | MouseButton::Middle) if viewer.pan_drag.is_some() => {
                        viewer.drag_pan(event.position, cx)
                    }
                    Some(MouseButton::Left) => viewer.pick(event.position, window, cx),
                    Some(MouseButton::Right) => viewer.drag_contrast(event.position, cx),
                    _ => {}
                }
            }))
            .on_scroll_wheel(cx.listener(|viewer, event: &ScrollWheelEvent, window, cx| {
                viewer.scroll(event, window, cx);
            }));

        if self.layers.is_empty() {
            return area.child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(muted)
                    .child("Drop scans here or open from the toolbar"),
            );
        }

        for view in self.views(window) {
            let display = &self.layers[view.base].display;
            let labels = display.labels(view.axis, self.radiological);
            // only show slice number for lightbox views
            let compact = view.slice.is_some();
            let mut panel = div()
                .absolute()
                .left(px(view.origin.x))
                .top(px(view.origin.y))
                .w(px(view.size.width))
                .h(px(view.size.height))
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .text_sm();
            let shown = self.shown(&view);
            let stack = div()
                .relative()
                .w(px(view.image_size.width))
                .h(px(view.image_size.height))
                .children(shown.iter().map(|(image, opacity)| {
                    img(image.clone())
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .opacity(*opacity)
                }));
            panel = panel.child(stack);
            if !shown.is_empty()
                && let Some(lines) = self.crosshair(&view, scale)
            {
                panel = panel.children(lines);
            }

            let edge = |text: &'static str| div().absolute().text_color(label_color).child(text);
            panel = panel
                .child(div().absolute().inset_0().map(|frame| {
                    if self.active_panel == (view.base, view.axis) && view.current {
                        frame.border_2().border_color(rgb(ui::ACTIVE_VIEW_COLOR))
                    } else {
                        frame.border_1().border_color(border)
                    }
                }))
                .when(self.info == InfoLevel::Full && !compact, |panel| {
                    panel.child(
                        v_flex()
                            .absolute()
                            .top_1()
                            .left_2()
                            .text_size(px(CORNER_TEXT))
                            .text_color(muted)
                            .child(labels.title)
                            .when(!overlaid, |title| title.child(display.name())),
                    )
                })
                .when(self.info != InfoLevel::Off && !compact, |panel| {
                    panel
                        .child(edge(labels.left).left_2())
                        .child(edge(labels.right).right_2())
                        .child(edge(labels.top).top_1())
                        .child(edge(labels.bottom).bottom_1())
                })
                .when(self.info != InfoLevel::Off, |panel| {
                    panel.child(self.corner_info(&view, muted))
                });
            area = area.child(panel);
        }
        area
    }

    pub(super) fn shown(&self, view: &View) -> Vec<(Arc<RenderImage>, f32)> {
        let mut shown = Vec::new();
        for layer in view.layers.clone() {
            let entry = &self.layers[layer];
            let Some(image) = entry
                .panel(view.slot)
                .and_then(|panel| panel.image.as_ref())
                .filter(|_| entry.visible)
            else {
                continue;
            };
            let opacity = if shown.is_empty() { 1.0 } else { entry.opacity };
            shown.push((Arc::clone(&image.image), opacity));
        }
        shown
    }

    pub(super) fn crosshair_at(&self, view: &View, scale: f32) -> Option<(f32, f32)> {
        if !self.show_crosshair || !view.current {
            return None;
        }
        let display = &self.layers[view.base].display;
        let (s, t) = view.visible.to_view(view.plane.project(display.cursor()));
        if !(0.0..=1.0).contains(&s) || !(0.0..=1.0).contains(&t) {
            return None;
        }
        let image = view.image_size;
        let left = (view.size.width - image.width) / 2.0;
        let top = (view.size.height - image.height) / 2.0;
        // "snap" to device screen pixels for sharp lines
        let snap = |value: f32| (value * scale).floor() / scale;
        Some((
            snap(left + s as f32 * image.width),
            snap(top + (1.0 - t as f32) * image.height),
        ))
    }

    fn crosshair(&self, view: &View, scale: f32) -> Option<[Div; 2]> {
        let (x, y) = self.crosshair_at(view, scale)?;
        let image = view.image_size;
        let left = (view.size.width - image.width) / 2.0;
        let top = (view.size.height - image.height) / 2.0;
        let line = || div().absolute().bg(rgba(ui::CROSSHAIR_COLOR));
        Some([
            line()
                .left(px(x))
                .top(px(top))
                .w(px(1.0))
                .h(px(image.height)),
            line()
                .left(px(left))
                .top(px(y))
                .w(px(image.width))
                .h(px(1.0)),
        ])
    }

    fn corner_info(&self, view: &View, color: Hsla) -> Div {
        let overlaid = view.layers.len() > 1;
        let numbered = |layer: usize, text: String| {
            if overlaid {
                format!("{}: {text}", layer + 1)
            } else {
                text
            }
        };
        let values = view
            .layers
            .clone()
            .filter(|&layer| self.layers[layer].visible)
            .map(|layer| {
                let value = self.layers[layer]
                    .display
                    .cursor_sample()
                    .map_or("-".to_string(), |(_, value)| format_value(value));
                numbered(layer, value)
            });
        let windowed = if overlaid {
            self.selected().unwrap_or(view.base)
        } else {
            view.base
        };
        let window_text = match self.layers[windowed].display.coloring() {
            Coloring::Labels { .. } => "Labels".to_string(),
            Coloring::Intensity { level, width, .. } => {
                format!("W {} L {}", format_value(width), format_value(level))
            }
        };
        let colorbar = match self.layers[windowed].display.coloring() {
            Coloring::Intensity {
                level,
                width,
                colormap,
                ..
            } => Some(colorbar(colormap, level - width / 2.0, level + width / 2.0)),
            Coloring::Labels { .. } => None,
        };
        let base = &self.layers[view.base].display;
        let (slice, count) = base.slice_position(view.axis);
        let slice = view.slice.unwrap_or(slice);
        let corner = || {
            div()
                .absolute()
                .text_size(px(CORNER_TEXT))
                .text_color(color)
        };
        let full = self.info == InfoLevel::Full && view.slice.is_none();
        div()
            .absolute()
            .inset_0()
            .when(full, |info| {
                info.child(
                    corner()
                        .top_1()
                        .right_2()
                        .child(v_flex().items_end().children(values)),
                )
                .child(
                    corner().bottom_1().left_2().child(
                        v_flex()
                            .when(!base.geometry().contains_world(base.cursor()), |text| {
                                text.child("Cursor outside volume")
                            })
                            .child(numbered(windowed, window_text)),
                    ),
                )
                .when_some(colorbar, |info, colorbar| {
                    info.child(
                        corner()
                            .top_0()
                            .bottom_0()
                            .right_2()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .child(colorbar),
                    )
                })
            })
            .child(corner().bottom_1().right_2().child({
                let position = if view.mip {
                    "MIP".to_string()
                } else {
                    format!("{}/{count}", slice + 1)
                };
                if view.zoom.scale > 1.0 {
                    format!("\u{d7}{:.1} {position}", view.zoom.scale)
                } else {
                    position
                }
            }))
    }
}

/// A vertical color bar with the window's top and bottom value
fn colorbar(colormap: Colormap, low: f64, high: f64) -> Div {
    const BLOCKS: usize = 32;
    v_flex()
        .items_end()
        .gap_0p5()
        .child(format_value(high))
        .child(
            v_flex()
                .w(px(6.0))
                .h(px(80.0))
                .rounded_sm()
                .overflow_hidden()
                .children((0..BLOCKS).rev().map(|block| {
                    let color = colormap.color((block as f64 + 0.5) / BLOCKS as f64);
                    div().w_full().flex_1().bg(rgb_to_rgba(color))
                })),
        )
        .child(format_value(low))
}
