//! Settings of the selected layer.

use gpui_kit::component::button::{Button, ButtonVariants, DropdownButton};
use gpui_kit::component::chart::LineChart;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::slider::Slider;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, AppContext, Context, Div, DragMoveEvent, Entity, Focusable, Hsla,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, canvas, div, px,
    relative,
};
use illuminator_core::colormap::Colormap;
use illuminator_core::display::VolumeDisplay;
use illuminator_core::histogram::Histogram;
use illuminator_core::slicer::Coloring;
use std::rc::Rc;

use crate::assets::Carbon;
use crate::shortcuts::{self, AutoContrast, NextColormap, ShowHistogram};
use crate::ui::{
    self, colormap_strip, format_editable, format_value, parse_value, rgb_to_rgba, section_title,
};

use super::{CORNER_TEXT, DragEdge, Preset, STALE_HISTOGRAM_OPACITY, SyncMode, Viewer, WindowEdge};

const HISTOGRAM_HEIGHT: f32 = 56.0;
const CHART_CURVE_HEIGHT: f32 = 64.0;
const TRANSFER_STRIP_HEIGHT: f32 = 6.0;
const PRESETS: &[PresetGroup] = &[
    PresetGroup {
        title: "CT (HU)",
        presets: &[
            ("Brain  W 80 L 40", Preset::Limits(0.0, 80.0)),
            ("Subdural  W 200 L 75", Preset::Limits(-25.0, 175.0)),
            ("Stroke  W 40 L 40", Preset::Limits(20.0, 60.0)),
            ("Soft tissue  W 400 L 50", Preset::Limits(-150.0, 250.0)),
            ("Lung  W 1500 L -600", Preset::Limits(-1350.0, 150.0)),
            ("Bone  W 1800 L 400", Preset::Limits(-500.0, 1300.0)),
        ],
    },
    PresetGroup {
        title: "PET (SUV)",
        presets: &[
            ("0 to 5", Preset::Limits(0.0, 5.0)),
            ("0 to 10", Preset::Limits(0.0, 10.0)),
        ],
    },
    PresetGroup {
        title: "Histogram",
        presets: &[
            ("Full range", Preset::Percentiles(0.0, 1.0)),
            ("1 to 99 %", Preset::Percentiles(0.01, 0.99)),
            ("5 to 95 %", Preset::Percentiles(0.05, 0.95)),
        ],
    },
];

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

        ui::panel(format!("Layer {}", index + 1), cx)
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
                        .child("Shown on voxel axes. Overlay position may be wrong"),
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
            .gap_1p5()
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
        let id = |name: &'static str| (name, index as u64);
        let limit = |input: &Entity<InputState>| div().flex_1().child(Input::new(input).xsmall());
        v_flex()
            .gap_1p5()
            .child(
                ui::setting_row("Window", muted)
                    .child(div().flex_1())
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
                    )
                    .child(
                        Button::new(id("histogram-window"))
                            .ghost()
                            .xsmall()
                            .icon(Carbon::Expand)
                            .tooltip_with_action(
                                "Histogram window",
                                &ShowHistogram,
                                Some(shortcuts::CONTEXT),
                            )
                            .on_click(cx.listener(|viewer, _, window, cx| {
                                viewer.show_histogram(&ShowHistogram, window, cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(limit(&self.limit_inputs.low))
                    .child(div().text_xs().text_color(muted).child("\u{2013}"))
                    .child(limit(&self.limit_inputs.high)),
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
                            .label(
                                layer
                                    .suv
                                    .map_or("SUV", |suv| suv.inputs.normalization.suv_name()),
                            )
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
                            .child(layer.suv.map_or("no sidecar", |suv| suv.source)),
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

    /// Shows the selected layers window limits in text fields.
    pub(super) fn show_limits(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.selected() else {
            return;
        };
        let layer = &self.layers[index];
        let (low, high) = layer.display.window_limits();
        self.limit_inputs.load_id = Some(layer.load_id);
        for (input, limit) in [
            (&self.limit_inputs.low, low),
            (&self.limit_inputs.high, high),
        ] {
            let text = format_editable(limit);
            let state = input.read(cx);
            if !state.focus_handle(cx).is_focused(window) && state.value().as_ref() != text {
                input.update(cx, |state, cx| state.set_value(text, window, cx));
            }
        }
    }

    /// Applies the typed limits and gives the keyboard back to the views.
    /// Works by leaving the field or hitting enter.
    fn limit_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::PressEnter { .. } => {
                self.apply_typed_limits(cx);
                self.focus_handle.focus(window, cx);
            }
            InputEvent::Blur => self.apply_typed_limits(cx),
            _ => {}
        }
    }

    /// Sets the window to the limits in the text fields.
    fn apply_typed_limits(&mut self, cx: &mut Context<Self>) {
        let Some(load_id) = self.limit_inputs.load_id else {
            return;
        };
        let low_text = self.limit_inputs.low.read(cx).value();
        let high_text = self.limit_inputs.high.read(cx).value();
        let Some(layer) = self.layer_by_id(load_id) else {
            return;
        };
        let (low, high) = layer.display.window_limits();
        // a field that still shows its limit keeps the exact value, not the rounded text
        let typed = |text: &str, limit: f64| {
            if text == format_editable(limit) {
                Some(limit)
            } else {
                parse_value(text)
            }
        };
        if let (Some(low), Some(high)) = (typed(&low_text, low), typed(&high_text, high)) {
            layer.display.set_window_limits(low, high);
        }
        cx.notify();
    }

    /// The current colormap. The menu lists all colormaps.
    fn render_colormaps(&self, index: usize, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let current = self.layers[index].display.colormap;
        let viewer = cx.entity().downgrade();
        ui::setting_row("Colormap", muted).child(
            DropdownButton::new(("colormap", index))
                .flex_1()
                .xsmall()
                .outline()
                .button(
                    Button::new(("colormap-next", index))
                        .flex_1()
                        .child(colormap_preview(current).flex_1())
                        .tooltip_with_action(
                            "Next colormap",
                            &NextColormap,
                            Some(shortcuts::CONTEXT),
                        )
                        .on_click(cx.listener(move |viewer, _, _, cx| {
                            let display = &mut viewer.layers[index].display;
                            display.colormap = display.colormap.next();
                            cx.notify();
                        })),
                )
                .dropdown_menu(move |mut menu, _, _| {
                    for colormap in Colormap::ALL {
                        let viewer = viewer.clone();
                        let item = PopupMenuItem::element(move |_, _| {
                            colormap_preview(colormap)
                                .id(colormap.id())
                                .flex_1()
                                .tooltip(move |window, cx| {
                                    Tooltip::new(colormap.description()).build(window, cx)
                                })
                        })
                        .checked(colormap == current)
                        .on_click(move |_, _, cx| {
                            let _ = viewer.update(cx, |viewer, cx| {
                                viewer.layers[index].display.colormap = colormap;
                                cx.notify();
                            });
                        });
                        menu = menu.item(item);
                    }
                    menu
                }),
        )
    }

    fn render_histogram(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, foreground, info, background, border) = (
            theme.muted_foreground,
            theme.foreground,
            theme.info,
            theme.secondary,
            theme.border,
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
        let counting: SharedString = if self.time_playing.is_some() {
            "Counting paused during playback".into()
        } else {
            "Counting values\u{2026}".into()
        };
        let Some(histogram) = layer.shown_histogram() else {
            let message = if layer.histogram.key == Some(layer.histogram_key()) {
                match &layer.histogram.error {
                    Some(error) => format!("Cannot count values: {error}").into(),
                    None => "No values to count".into(),
                }
            } else {
                counting
            };
            return v_flex()
                .gap_0p5()
                .child(
                    chart
                        .flex()
                        .items_center()
                        .justify_center()
                        .px_2()
                        .text_center()
                        .text_xs()
                        .text_color(muted)
                        .child(message),
                )
                .child(div().h(px(TRANSFER_STRIP_HEIGHT)))
                .child(value_range("0", "0", muted).invisible()); // holds the height while counting
        };

        let stale = layer.current_histogram().is_none();
        let fade = if stale { STALE_HISTOGRAM_OPACITY } else { 1.0 };
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
                    foreground.opacity(fade)
                } else {
                    muted.opacity(0.5 * fade)
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
                    .when(stale, |chart| {
                        chart.child(
                            div()
                                .absolute()
                                .top_1()
                                .right_1()
                                .px_1()
                                .rounded_sm()
                                .border_1()
                                .border_color(border)
                                .bg(background)
                                .text_size(px(CORNER_TEXT))
                                .text_color(muted)
                                .child(counting),
                        )
                    })
                    .child(edge(WindowEdge::Low, low))
                    .child(edge(WindowEdge::High, high)),
            )
            .child(transfer_strip(histogram, layer.display.coloring()).opacity(fade))
            .child(value_range(
                format_value(histogram.min),
                format_value(histogram.max),
                muted.opacity(fade),
            ))
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
            let message = if layer.curve_pending {
                "Reading the curve\u{2026}"
            } else {
                "Cursor is outside the volume"
            };
            return Some(
                v_flex()
                    .gap_1()
                    .child(title)
                    .child(
                        div()
                            .w_full()
                            .h(px(CHART_CURVE_HEIGHT))
                            .rounded_sm()
                            .bg(background)
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_xs()
                            .text_color(muted)
                            .child(message),
                    )
                    .child(curve_footer("0", "0", muted).invisible())
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
                .child(curve_footer(
                    format!("{} \u{2013} {}", format_value(low), format_value(high)),
                    format!("frame {}: {}", frame + 1, format_value(value)),
                    muted,
                ))
                .into_any_element(),
        )
    }

    /// Menu of window presets for the layer currently at index.
    fn presets_button(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let viewer = cx.entity().downgrade();
        let load_id = self.layers[index].load_id;
        Button::new(("presets", index))
            .ghost()
            .xsmall()
            .icon(Carbon::WindowPreset)
            .tooltip("Window presets")
            .dropdown_menu(move |mut menu, _, _| {
                for (n, group) in PRESETS.iter().enumerate() {
                    if n > 0 {
                        menu = menu.separator();
                    }
                    let choices = group.presets.iter().copied();
                    menu = ui::choice_items(
                        menu.label(group.title),
                        &viewer,
                        choices,
                        move |viewer, preset, _, cx| {
                            viewer.apply_preset(load_id, preset, cx);
                        },
                    );
                }
                menu
            })
    }
}

/// Text fields for the low and high limit of the window of the selected layer.
pub(super) struct LimitInputs {
    low: Entity<InputState>,
    high: Entity<InputState>,
    /// The layer whose limits the fields show
    load_id: Option<u64>,
    _events: [Subscription; 2],
}

impl LimitInputs {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Viewer>) -> Self {
        let low = cx.new(|cx| InputState::new(window, cx));
        let high = cx.new(|cx| InputState::new(window, cx));
        let events =
            [&low, &high].map(|input| cx.subscribe_in(input, window, Viewer::limit_input_event));
        Self {
            low,
            high,
            load_id: None,
            _events: events,
        }
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

/// Lowest and highest values under a histogram.
fn value_range(low: impl Into<SharedString>, high: impl Into<SharedString>, color: Hsla) -> Div {
    h_flex()
        .justify_between()
        .text_size(px(CORNER_TEXT))
        .text_color(color)
        .child(low.into())
        .child(high.into())
}

/// The value range of a time curve and the value at the shown frame.
fn curve_footer(
    range: impl Into<SharedString>,
    at_frame: impl Into<SharedString>,
    color: Hsla,
) -> Div {
    h_flex()
        .justify_between()
        .text_xs()
        .text_color(color)
        .child(range.into())
        .child(at_frame.into())
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

/// A color strip and the name of `colormap`.
fn colormap_preview(colormap: Colormap) -> Div {
    h_flex()
        .gap_2()
        .text_xs()
        .child(div().w(px(48.0)).child(colormap_strip(colormap, false, 12)))
        .child(colormap.name())
}

/// Window presets under one heading in the presets menu
struct PresetGroup {
    title: &'static str,
    presets: &'static [(&'static str, Preset)],
}

#[cfg(test)]
mod tests {
    use super::{PRESETS, Preset};

    #[test]
    fn presets_go_from_low_to_high() {
        for group in PRESETS {
            for (name, preset) in group.presets {
                match *preset {
                    Preset::Limits(low, high) => assert!(low < high, "{name}"),
                    Preset::Percentiles(low, high) => {
                        assert!((0.0..high).contains(&low) && high <= 1.0, "{name}");
                    }
                }
            }
        }
    }
}
