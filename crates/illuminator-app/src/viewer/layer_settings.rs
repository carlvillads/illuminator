//! Settings of the selected layer

use super::*;

impl Viewer {
    /// Settings of the selected layer
    pub(super) fn render_layer_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(index) = self.selected() else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        let layer = &self.layers[index];
        let display = &layer.display;
        let anatomical = display.geometry().anatomical;
        let overlaid = self.layout.overlaid();
        let bottom_visible = self.layers.iter().position(|layer| layer.visible);
        let link_world = !overlaid && self.sync && self.sync_mode == SyncMode::World && anatomical;

        v_flex()
            .gap_2()
            .child(section_title(format!("Layer {}", index + 1), cx))
            .when(overlaid && bottom_visible != Some(index), |settings| {
                settings.child(
                    ui::setting_row("Opacity", muted)
                        .child(div().flex_1().child(Slider::new(&layer.opacity_slider)))
                        .child(
                            div()
                                .w(px(32.0))
                                .text_xs()
                                .child(format!("{:.0}%", layer.opacity * 100.0)),
                        ),
                )
            })
            .child(if display.label_mode() {
                self.render_label_settings(index, cx)
            } else {
                self.render_intensity_settings(index, cx)
            })
            .children(self.render_time_curve(index, cx))
            .child(display_checkbox(
                index,
                "labels",
                "Label map",
                display.label_mode(),
                VolumeDisplay::set_label_mode,
                cx,
            ))
            .when(display.can_be_anatomical(), |settings| {
                settings.child(display_checkbox(
                    index,
                    "native",
                    "Native axes",
                    display.options().native,
                    VolumeDisplay::set_native,
                    cx,
                ))
            })
            .when(display.units_unknown(), |settings| {
                settings.child(display_checkbox(
                    index,
                    "assume-mm",
                    "Assume mm",
                    display.options().assume_mm,
                    VolumeDisplay::set_assume_mm,
                    cx,
                ))
            })
            .when(link_world, |settings| {
                settings.child(display_checkbox(
                    index,
                    "link",
                    "Link in world",
                    display.link_world,
                    |display, link| {
                        display.link_world = link;
                        Ok(())
                    },
                    cx,
                ))
            })
            .when(overlaid && !anatomical, |settings| {
                settings.child(
                    div()
                        .text_xs()
                        .text_color(danger)
                        .child("No orientation in header. Overlay position may be wrong"),
                )
            })
            .into_any_element()
    }

    /// Settings of a layer shown as a label map
    fn render_label_settings(&self, index: usize, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let display = &self.layers[index].display;
        let id = |name: &'static str| (name, index as u64);
        v_flex()
            .gap_2()
            .child(
                ui::setting_row("Labels", muted)
                    .child(
                        div()
                            .flex_1()
                            .child(format!("{} labels", display.label_count())),
                    )
                    .child(
                        Button::new(id("statistics"))
                            .xsmall()
                            .label("Statistics\u{2026}")
                            .on_click(cx.listener(move |viewer, _, window, cx| {
                                viewer.show_statistics(index, window, cx);
                            })),
                    ),
            )
            .child(
                Checkbox::new(id("outline"))
                    .label("Outline")
                    .checked(display.outline)
                    .on_click(cx.listener(move |viewer, checked: &bool, _, cx| {
                        viewer.layers[index].display.outline = *checked;
                        cx.notify();
                    })),
            )
    }

    /// Settings of a layer shown with a colormap
    fn render_intensity_settings(&self, index: usize, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let layer = &self.layers[index];
        let display = &layer.display;
        let window = display.window;
        let id = |name: &'static str| (name, index as u64);
        v_flex()
            .gap_2()
            .child(
                ui::setting_row("Window", muted)
                    .child(div().flex_1().text_xs().child(format!(
                        "W {}  L {}",
                        format_value(window.width),
                        format_value(window.level)
                    )))
                    .child(self.presets_button(index, cx))
                    .child(
                        Button::new(id("auto"))
                            .ghost()
                            .xsmall()
                            .icon(Carbon::AutoWindow)
                            .tooltip_with_action(
                                "Fit to the current frame",
                                &AutoContrast,
                                Some(shortcuts::CONTEXT),
                            )
                            .on_click(cx.listener(move |viewer, _, _, cx| {
                                viewer.auto_window(index, cx);
                            })),
                    ),
            )
            .child(self.render_histogram(index, cx))
            .child(self.render_colormaps(index, cx))
            .child(
                Checkbox::new(id("clip-low"))
                    .label("Transparent below window")
                    .checked(display.clip_low)
                    .on_click(cx.listener(move |viewer, checked: &bool, _, cx| {
                        viewer.layers[index].display.clip_low = *checked;
                        cx.notify();
                    })),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Checkbox::new(id("suv"))
                            .label("SUV")
                            .checked(display.scale() != 1.0)
                            .disabled(layer.suv.is_none())
                            .on_click(cx.listener(move |viewer, checked: &bool, _, cx| {
                                viewer.set_suv(index, *checked, cx);
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(muted)
                            .child(layer.suv.map_or("no sidecar", |(_, source)| source)),
                    )
                    .child(
                        Button::new(id("suv-edit"))
                            .ghost()
                            .xsmall()
                            .label("Edit\u{2026}")
                            .tooltip("Edit SUV calculation")
                            .on_click(cx.listener(move |viewer, _, window, cx| {
                                viewer.edit_suv(index, window, cx);
                            })),
                    ),
            )
            .when(display.range().is_none(), |settings| {
                settings.child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child("No finite values in this frame"),
                )
            })
    }

    /// One color strip per colormap. Clicking a color strip "icon" picks that colormap
    fn render_colormaps(&self, index: usize, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let (muted, info) = (theme.muted_foreground, theme.info);
        let current = self.layers[index].display.colormap;
        ui::setting_row("Colormap", muted).child(h_flex().flex_1().gap_1().children(
            Colormap::ALL.into_iter().map(|colormap| {
                let name = colormap.name();
                div()
                    .id(SharedString::from(format!("colormap-{name}")))
                    .flex_1()
                    .p_0p5()
                    .rounded_sm()
                    .border_1()
                    .border_color(if colormap == current {
                        info
                    } else {
                        transparent_black()
                    })
                    .child(colormap_strip(colormap, false, 8))
                    .tooltip(move |window, cx| Tooltip::new(name).build(window, cx))
                    .on_click(cx.listener(move |viewer, _, _, cx| {
                        viewer.layers[index].display.colormap = colormap;
                        cx.notify();
                    }))
            }),
        ))
    }

    fn render_histogram(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, foreground, info, background) = (
            theme.muted_foreground,
            theme.foreground,
            theme.info,
            theme.secondary,
        );
        let layer = &self.layers[index];
        let load_id = layer.load_id;
        let chart = div()
            .id(("histogram", index))
            .relative()
            .w_full()
            .h(px(HISTOGRAM_HEIGHT))
            .rounded_sm()
            .overflow_hidden()
            .bg(background)
            .on_drag_move(
                cx.listener(move |viewer, event: &DragMoveEvent<DragEdge>, _, cx| {
                    let drag = *event.drag(cx);
                    if drag.load_id == load_id {
                        viewer.drag_edge(drag, event.event.position, event.bounds, cx);
                    }
                }),
            );
        let Some(histogram) = &layer.histogram else {
            return v_flex().child(
                chart
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .text_color(muted)
                    .child("Counting values\u{2026}"),
            );
        };

        let span = histogram.max - histogram.min;
        let at = |value: f64| {
            if span > 0.0 {
                ((value - histogram.min) / span) as f32
            } else {
                0.5
            }
        };
        let (low, high) = layer.display.window_limits();
        let peak = histogram.counts.iter().copied().max().unwrap_or(0).max(1);
        let bins = histogram.counts.len() as f32;
        let bars = histogram.counts.iter().enumerate().map(|(bin, &count)| {
            let center = histogram.min + (bin as f64 + 0.5) * histogram.bin_width();
            let inside = (low..=high).contains(&center);
            let height = ((count as f64).ln_1p() / (peak as f64).ln_1p()) as f32;
            div()
                .absolute()
                .bottom_0()
                .left(relative(bin as f32 / bins))
                .w(relative(1.0 / bins))
                .h(relative(height))
                .bg(if inside {
                    foreground
                } else {
                    muted.opacity(0.5)
                })
        });
        let edge = |edge: WindowEdge, value: f64| {
            let id = match edge {
                WindowEdge::Low => "window-low",
                WindowEdge::High => "window-high",
            };
            div()
                .id((id, index))
                .absolute()
                .top_0()
                .bottom_0()
                .left(relative(at(value).clamp(0.0, 1.0)))
                .ml(px(-4.0))
                .w(px(8.0))
                .flex()
                .justify_center()
                .cursor_ew_resize()
                .child(div().w(px(2.0)).h_full().bg(info))
                .on_drag(DragEdge { load_id, edge }, |drag, _, _, cx| {
                    cx.new(|_| *drag)
                })
        };
        let shaded_left = at(low).clamp(0.0, 1.0);
        let shaded_right = at(high).clamp(0.0, 1.0);

        v_flex()
            .gap_0p5()
            .child(
                chart
                    .children(bars)
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(relative(shaded_left))
                            .w(relative(shaded_right - shaded_left))
                            .bg(info.opacity(0.15)),
                    )
                    .child(edge(WindowEdge::Low, low))
                    .child(edge(WindowEdge::High, high)),
            )
            .child(transfer_strip(histogram, layer.display.coloring()))
            .child(
                h_flex()
                    .justify_between()
                    .text_size(px(CORNER_TEXT))
                    .text_color(muted)
                    .child(format_value(histogram.min))
                    .child(format_value(histogram.max)),
            )
    }

    fn render_time_curve(&self, index: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let layer = &self.layers[index];
        let frames = layer.display.volume().meta().timepoints();
        if frames < 2 {
            return None;
        }
        let theme = cx.theme();
        let (muted, info, background, stroke) = (
            theme.muted_foreground,
            theme.info,
            theme.secondary,
            theme.foreground,
        );
        let title = section_title("Curve at cursor", cx);
        let Some(curve) = &layer.curve else {
            return Some(
                v_flex()
                    .gap_1()
                    .child(title)
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("Cursor is outside the volume"),
                    )
                    .into_any_element(),
            );
        };

        let finite = curve
            .values
            .iter()
            .copied()
            .filter(|value| value.is_finite());
        let low = finite.clone().fold(0.0, f64::min);
        let high = finite.fold(low, f64::max);
        let high = if high > low { high } else { low + 1.0 };
        let points: Vec<(usize, f64)> = curve
            .values
            .iter()
            .map(|&value| if value.is_finite() { value } else { low })
            .enumerate()
            .collect();
        let frame = layer.display.frame();
        let value = points.get(frame).map_or(0.0, |&(_, value)| value);
        let bounds = Rc::clone(&self.time_curve_bounds);
        let chart = LineChart::new(points)
            .x(|&(frame, _)| SharedString::from((frame + 1).to_string()))
            .y(|&(_, value)| value)
            .y_domain(low, high)
            .y_padding(4.0, 0.0)
            .x_axis(false)
            .grid(false)
            .appear(false)
            .linear()
            .stroke(stroke)
            .tooltip_title(|&(frame, _)| format!("Frame {}", frame + 1).into())
            .tooltip_value(|_, value| format_value(value).into());
        let plot = div()
            .id(("time-curve", index))
            .relative()
            .w_full()
            .h(px(CHART_CURVE_HEIGHT))
            .rounded_sm()
            .bg(background)
            .cursor_pointer()
            .child(
                canvas(move |area, _, _| bounds.set(area), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(chart)
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(frame as f32 / (frames - 1) as f32))
                    .w(px(1.0))
                    .bg(info),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|viewer, event: &MouseDownEvent, window, cx| {
                    viewer.pick_frame(event.position.x, window, cx);
                }),
            )
            .on_mouse_move(cx.listener(|viewer, event: &MouseMoveEvent, window, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    viewer.pick_frame(event.position.x, window, cx);
                }
            }));
        Some(
            v_flex()
                .gap_1()
                .child(title)
                .child(plot)
                .child(
                    h_flex()
                        .justify_between()
                        .text_xs()
                        .text_color(muted)
                        .child(format!(
                            "{} \u{2013} {}",
                            format_value(low),
                            format_value(high)
                        ))
                        .child(format!("frame {}: {}", frame + 1, format_value(value))),
                )
                .into_any_element(),
        )
    }

    /// Menu of window presets for layer index
    fn presets_button(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let viewer = cx.entity().downgrade();
        Button::new(("presets", index))
            .ghost()
            .xsmall()
            .icon(Carbon::WindowPreset)
            .tooltip("Window presets")
            .dropdown_menu(move |mut menu, _, _| {
                for (n, (group, presets)) in PRESETS.iter().enumerate() {
                    if n > 0 {
                        menu = menu.separator();
                    }
                    menu = menu.label(*group);
                    for &(name, preset) in *presets {
                        let viewer = viewer.clone();
                        menu = menu.item(PopupMenuItem::new(name).on_click(move |_, _, cx| {
                            let _ = viewer.update(cx, |viewer, cx| {
                                viewer.apply_preset(index, preset, cx);
                            });
                        }));
                    }
                }
                menu
            })
    }
}

/// Checkbox that changes a display setting of the layer at `index`.
fn display_checkbox(
    index: usize,
    id: &'static str,
    label: &'static str,
    checked: bool,
    set: fn(&mut VolumeDisplay, bool) -> illuminator_core::Result<()>,
    cx: &mut Context<Viewer>,
) -> Checkbox {
    Checkbox::new((id, index as u64))
        .label(label)
        .checked(checked)
        .on_click(cx.listener(move |viewer, checked: &bool, _, cx| {
            let checked = *checked;
            viewer.change(index, |display| set(display, checked), cx);
        }))
}

/// The colors along the histogram's value axis, as the views show them
fn transfer_strip(histogram: &Histogram, coloring: Coloring) -> Div {
    const BLOCKS: usize = 64;
    let Coloring::Intensity { colormap, .. } = coloring else {
        return div();
    };
    h_flex()
        .w_full()
        .h(px(6.0))
        .rounded_sm()
        .overflow_hidden()
        .children((0..BLOCKS).map(|block| {
            let fraction = (block as f64 + 0.5) / BLOCKS as f64;
            let value = histogram.min + fraction * (histogram.max - histogram.min);
            let block = div().flex_1().h_full();
            match coloring.window_position(value) {
                Some(t) => block.bg(rgb_to_rgba(colormap.color(t))),
                None => block,
            }
        }))
}
