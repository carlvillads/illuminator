//! Drawing the views.

use gpui_kit::component::{ActiveTheme, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    Context, Div, Hsla, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement, RenderImage, ScrollEvent,
    StatefulInteractiveElement, Styled, Window, div, img, px, rgb, rgba,
};
use illuminator_core::colormap::Colormap;
use illuminator_core::slicer::Coloring;
use std::sync::Arc;

use crate::theme;
use crate::ui::{format_value, rgb_to_rgba};

use super::{CORNER_TEXT_FONT_SIZE, CornerTextLevel, View, Viewer};

impl Viewer {
    pub(super) fn render_panels(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let (border, ring) = (theme.border, theme.ring);
        let muted: Hsla = rgb(theme::VIEW_TEXT_COLOR).into();
        let orientation_color: Hsla = rgb(theme::ORIENTATION_LABEL_COLOR).into();
        let scale = window.scale_factor();
        let overlaid = self.layout.is_overlaid();
        let mut area = div()
            .id("views")
            .relative()
            .flex_1()
            .h_full()
            .bg(rgb(theme::VIEW_BACKGROUND))
            .on_hover(cx.listener(|viewer, hovered: &bool, _, cx| {
                if !hovered {
                    viewer.hovered_point = None;
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
                        viewer.select_point_at(event.position, window, cx);
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
                cx.listener(|viewer, event: &MouseDownEvent, window, cx| {
                    viewer.set_active_panel(event.position, window, cx);
                    viewer.window_drag = viewer.hit(event.position, window).map(|hit| {
                        let layer = if viewer.layout.is_overlaid() {
                            viewer.selected_layer
                        } else {
                            hit.base
                        };
                        (layer, event.position)
                    });
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|viewer, _: &MouseUpEvent, _, _| viewer.window_drag = None),
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
                viewer.set_hovered_point(event.position, window, cx);
                match event.pressed_button {
                    Some(MouseButton::Left | MouseButton::Middle) if viewer.pan_drag.is_some() => {
                        viewer.update_pan(event.position, cx)
                    }
                    Some(MouseButton::Left) => viewer.select_point_at(event.position, window, cx),
                    Some(MouseButton::Right) => viewer.drag_window(event.position, cx),
                    _ => {}
                }
            }))
            .on_scroll(cx.listener(|viewer, event: &ScrollEvent, window, cx| {
                viewer.handle_scroll(event, window, cx);
            }));

        if self.layers.is_empty() {
            return area.child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(muted)
                    .child("Drop scans here or open from the sidebar"),
            );
        }

        for view in self.views(window) {
            let display = &self.layers[view.base_layer].display;
            let orientation = display.orientation_labels(view.axis, self.radiological);
            // only show slice number for lightbox views
            let compact = view.lightbox_slice.is_some();
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
            let visible_images = self.visible_images(&view);
            let stack = div()
                .relative()
                .w(px(view.image_size.width))
                .h(px(view.image_size.height))
                .children(visible_images.iter().map(|(image, opacity)| {
                    img(image.clone())
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .opacity(*opacity)
                }));
            panel = panel.child(stack);
            if !visible_images.is_empty()
                && let Some(lines) = self.crosshair_lines(&view, scale)
            {
                panel = panel.children(lines);
            }

            let orientation_label =
                |text: &'static str| div().absolute().text_color(orientation_color).child(text);
            panel = panel
                .child(div().absolute().inset_0().map(|frame| {
                    if self.active_panel == (view.base_layer, view.axis)
                        && view.lightbox_slice.is_none()
                    {
                        frame.border_2().border_color(ring)
                    } else {
                        frame.border_1().border_color(border)
                    }
                }))
                .when(
                    self.corner_text == CornerTextLevel::Full && !compact,
                    |panel| {
                        panel.child(
                            v_flex()
                                .absolute()
                                .top_1()
                                .left_2()
                                .text_size(px(CORNER_TEXT_FONT_SIZE))
                                .text_color(muted)
                                .child(
                                    div()
                                        .text_color(rgb(theme::plane_color(view.axis)))
                                        .child(orientation.title),
                                )
                                .when(!overlaid, |title| title.child(display.name())),
                        )
                    },
                )
                .when(
                    self.corner_text != CornerTextLevel::Off && !compact,
                    |panel| {
                        panel
                            .child(orientation_label(orientation.left).left_2())
                            .child(orientation_label(orientation.right).right_2())
                            .child(orientation_label(orientation.top).top_1())
                            .child(orientation_label(orientation.bottom).bottom_1())
                    },
                )
                .when(self.corner_text != CornerTextLevel::Off, |panel| {
                    panel.child(self.render_corner_text(&view, muted))
                });
            area = area.child(panel);
        }
        area
    }

    pub(super) fn visible_images(&self, view: &View) -> Vec<(Arc<RenderImage>, f32)> {
        let mut visible_images = Vec::new();
        for index in view.layers.clone() {
            let layer = &self.layers[index];
            let Some(image) = layer
                .panel(view.slot)
                .and_then(|panel| panel.image.as_ref())
                .filter(|_| layer.visible)
            else {
                continue;
            };
            let opacity = if visible_images.is_empty() {
                1.0
            } else {
                layer.opacity
            };
            visible_images.push((Arc::clone(&image.image), opacity));
        }
        visible_images
    }

    pub(super) fn crosshair_position(&self, view: &View, scale: f32) -> Option<(f32, f32)> {
        if !self.show_crosshair || !view.shows_cursor_slice {
            return None;
        }
        let display = &self.layers[view.base_layer].display;
        let (s, t) = view
            .visible_region
            .plane_to_view(view.plane.project(display.cursor()));
        if !(0.0..=1.0).contains(&s) || !(0.0..=1.0).contains(&t) {
            return None;
        }
        let image = view.image_size;
        let offset = view.image_offset();
        // "snap" to device screen pixels for sharp lines
        let align_to_device_pixel = |value: f32| (value * scale).floor() / scale;
        Some((
            align_to_device_pixel(offset.x + s as f32 * image.width),
            align_to_device_pixel(offset.y + (1.0 - t as f32) * image.height),
        ))
    }

    fn crosshair_lines(&self, view: &View, scale: f32) -> Option<[Div; 2]> {
        let (x, y) = self.crosshair_position(view, scale)?;
        let image = view.image_size;
        let offset = view.image_offset();
        let [vertical, horizontal] = theme::crosshair_colors(view.axis);
        let line = |color: u32| div().absolute().bg(rgba(color));
        Some([
            line(vertical)
                .left(px(x))
                .top(px(offset.y))
                .w(px(1.0))
                .h(px(image.height)),
            line(horizontal)
                .left(px(offset.x))
                .top(px(y))
                .w(px(image.width))
                .h(px(1.0)),
        ])
    }

    fn render_corner_text(&self, view: &View, color: Hsla) -> Div {
        let overlaid = view.layers.len() > 1;
        let numbered = |index: usize, text: String| {
            if overlaid {
                format!("{}: {text}", index + 1)
            } else {
                text
            }
        };
        let values = view
            .layers
            .clone()
            .filter(|&index| self.layers[index].visible)
            .map(|index| {
                let value = self.layers[index]
                    .display
                    .value_at_cursor()
                    .map_or("-".to_string(), |(_, value)| format_value(value));
                numbered(index, value)
            });
        let windowed_layer = if overlaid {
            self.selected_layer().unwrap_or(view.base_layer)
        } else {
            view.base_layer
        };
        let (window_text, bar) = match self.layers[windowed_layer].display.coloring() {
            Coloring::Labels { .. } => ("Labels".to_string(), None),
            Coloring::Intensity {
                level,
                width,
                colormap,
                ..
            } => (
                format!("W {} L {}", format_value(width), format_value(level)),
                Some(vertical_color_bar(
                    colormap,
                    level - width / 2.0,
                    level + width / 2.0,
                )),
            ),
        };
        let base = &self.layers[view.base_layer].display;
        let (slice, count) = base.slice_position(view.axis);
        let slice = view.lightbox_slice.unwrap_or(slice);
        let mut position = if view.mip {
            "MIP".to_string()
        } else {
            format!("{}/{count}", slice + 1)
        };
        if view.zoom.scale > 1.0 {
            position = format!("\u{d7}{:.1} {position}", view.zoom.scale);
        }
        let corner = || {
            div()
                .absolute()
                .text_size(px(CORNER_TEXT_FONT_SIZE))
                .text_color(color)
        };
        let full = self.corner_text == CornerTextLevel::Full && view.lightbox_slice.is_none();
        let bar = bar.filter(|_| full);
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
                            .child(numbered(windowed_layer, window_text)),
                    ),
                )
            })
            .child(
                corner()
                    .bottom_1()
                    .right_2()
                    .child(v_flex().items_end().gap_2().children(bar).child(position)),
            )
    }
}

/// A vertical color bar with the window's top and bottom value
fn vertical_color_bar(colormap: Colormap, low: f64, high: f64) -> Div {
    const SEGMENT_COUNT: usize = 32;
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
                .children((0..SEGMENT_COUNT).rev().map(|block| {
                    let color = colormap.color((block as f64 + 0.5) / SEGMENT_COUNT as f64);
                    div().w_full().flex_1().bg(rgb_to_rgba(color))
                })),
        )
        .child(format_value(low))
}
