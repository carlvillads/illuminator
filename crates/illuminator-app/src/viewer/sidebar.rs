//! Loading list, layer list, view settings.

use super::*;

impl Viewer {
    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (border, sidebar) = (theme.sidebar_border, theme.sidebar);
        v_flex()
            .id("sidebar")
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_none()
            .p_1p5()
            .gap_2()
            .overflow_y_scroll()
            .bg(sidebar)
            .border_l_1()
            .border_color(border)
            .text_sm()
            .child(self.render_loading(cx))
            .child(self.render_layer_list(cx))
            .child(self.render_layer_settings(cx))
            .child(self.render_view_settings(cx))
    }

    fn render_loading(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        v_flex()
            .gap_1()
            .children(self.loader.statuses().into_iter().map(|status| {
                let id = status.load_id;
                v_flex()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .truncate_start()
                                    .child(file_name(&status.path)),
                            )
                            .child(
                                Button::new(("load-cancel", id))
                                    .ghost()
                                    .xsmall()
                                    .icon(Carbon::Close)
                                    .tooltip("Cancel")
                                    .on_click(
                                        cx.listener(move |viewer, _, _, _| {
                                            viewer.loader.cancel(id)
                                        }),
                                    ),
                            ),
                    )
                    .child(
                        Progress::new(("load-progress", id)).value(status.progress as f32 * 100.0),
                    )
                    .child(div().text_xs().text_color(muted).child(status.stage))
            }))
    }

    /// One row per layer
    fn render_layer_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, list, radius) = (theme.muted_foreground, theme.colors.list, theme.radius);
        let rows = v_flex()
            .p_0p5()
            .rounded(radius)
            .bg(list)
            .when(self.layers.is_empty(), |list| {
                list.child(
                    div()
                        .p_1()
                        .text_xs()
                        .text_color(muted)
                        .child("No volumes open"),
                )
            })
            .children((0..self.layers.len()).map(|index| self.render_layer_row(index, cx)));
        let commands = h_flex()
            .gap_1()
            .child(
                Button::new("layers-open")
                    .xsmall()
                    .icon(Carbon::FolderOpen)
                    .label("Open\u{2026}")
                    .on_click(cx.listener(|viewer, _, _, cx| viewer.open_files(cx))),
            )
            .child(
                Button::new("layers-reload")
                    .xsmall()
                    .icon(Carbon::Reload)
                    .label("Reload")
                    .disabled(self.layers.is_empty())
                    .on_click(cx.listener(|viewer, _, _, _| {
                        if let Some(index) = viewer.selected() {
                            viewer.reload_layer(index);
                        }
                    })),
            )
            .child(
                Button::new("layers-close")
                    .xsmall()
                    .icon(Carbon::Close)
                    .label("Close")
                    .disabled(self.layers.is_empty())
                    .on_click(cx.listener(|viewer, _, _, cx| {
                        if let Some(index) = viewer.selected() {
                            viewer.close_layer(index, cx);
                        }
                    })),
            );
        ui::panel("Layers", cx).child(rows).child(commands)
    }

    /// A layers row in the layer list.
    fn render_layer_row(&self, index: usize, cx: &mut Context<Self>) -> HoverCard {
        let theme = cx.theme();
        let selected = index == self.selected;
        let (active, active_text, hover, info, radius) = (
            theme.accent,
            theme.accent_foreground,
            theme.list_hover,
            theme.info,
            theme.radius,
        );
        let muted = if selected {
            active_text.opacity(0.7)
        } else {
            theme.muted_foreground
        };
        let context = Some(shortcuts::CONTEXT);
        let layer = &self.layers[index];
        let display = &layer.display;
        let drag = DragLayer {
            index,
            name: display.name().into(),
        };
        let row = h_flex()
            .id(("layer-row", index))
            .group("layer-row")
            .gap_1()
            .h(px(24.0))
            .px_1()
            .rounded(radius)
            .map(|row| {
                if selected {
                    row.bg(active).text_color(active_text)
                } else {
                    row.hover(|row| row.bg(hover))
                }
            })
            .on_click(cx.listener(move |viewer, _, _, cx| viewer.select(index, cx)))
            .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
            .drag_over::<DragLayer>(move |style, _, _, _| style.border_t_2().border_color(info))
            .on_drop(cx.listener(move |viewer, drag: &DragLayer, _, cx| {
                viewer.move_layer(drag.index, index, cx);
            }))
            .child(
                Button::new(("layer-visible", index))
                    .ghost()
                    .xsmall()
                    .icon(if layer.visible {
                        Carbon::View
                    } else {
                        Carbon::ViewOff
                    })
                    .on_click(cx.listener(move |viewer, _, _, cx| {
                        let layer = &mut viewer.layers[index];
                        layer.visible = !layer.visible;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .w(px(14.0))
                    .text_xs()
                    .text_color(muted)
                    .child(format!("{}", index + 1)),
            )
            .child(div().w(px(18.0)).child(colormap_strip(
                display.colormap,
                display.label_mode(),
                3,
            )))
            .child(
                div()
                    .truncate_start()
                    .when(!layer.visible, |name| name.text_color(muted))
                    .child(display.name()),
            )
            .child(
                h_flex()
                    .opacity(0.4)
                    .group_hover("layer-row", |buttons| buttons.opacity(1.0))
                    .child(
                        Button::new(("layer-reload", index))
                            .ghost()
                            .xsmall()
                            .icon(Carbon::Reload)
                            .tooltip_with_action("Reload from disk", &ReloadLayer, context)
                            .on_click(cx.listener(move |viewer, _, _, _| {
                                viewer.reload_layer(index);
                            })),
                    )
                    .child(
                        Button::new(("layer-close", index))
                            .ghost()
                            .xsmall()
                            .icon(Carbon::Close)
                            .tooltip_with_action("Close", &CloseLayer, context)
                            .on_click(cx.listener(move |viewer, _, _, cx| {
                                viewer.close_layer(index, cx);
                            })),
                    ),
            );
        let details = layer_details(display);
        HoverCard::new(("layer-card", index))
            .anchor(Anchor::RightCenter)
            .trigger(row)
            .content(move |_, _, cx| layer_card(&details, cx.theme().muted_foreground))
    }

    /// How many lightbox views and how far apart each view is
    fn render_lightbox_settings(&self, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let grid_label = |(columns, rows): (usize, usize)| format!("{columns} \u{d7} {rows}");
        let step_label = |step: usize| match step {
            1 => "1 step".to_string(),
            step => format!("{step} steps"),
        };
        ui::setting_row("Lightbox", muted)
            .child(
                Button::new("lightbox-grid")
                    .ghost()
                    .xsmall()
                    .label(grid_label(self.lightbox_grid))
                    .tooltip("Columns \u{d7} rows")
                    .dropdown_menu({
                        let viewer = cx.entity().downgrade();
                        move |menu, _, _| {
                            let choices = LIGHTBOX_GRIDS.map(|grid| (grid_label(grid), grid));
                            ui::choice_items(menu, &viewer, choices, |viewer, grid, _, cx| {
                                viewer.set_lightbox_grid(grid, cx);
                            })
                        }
                    }),
            )
            .child(
                Button::new("lightbox-step")
                    .ghost()
                    .xsmall()
                    .label(step_label(self.lightbox_step))
                    .tooltip("Step size")
                    .dropdown_menu({
                        let viewer = cx.entity().downgrade();
                        move |menu, _, _| {
                            let choices = LIGHTBOX_STEPS.map(|step| (step_label(step), step));
                            ui::choice_items(menu, &viewer, choices, |viewer, step, _, cx| {
                                viewer.lightbox_step = step;
                                cx.notify();
                            })
                        }
                    }),
            )
    }

    /// Settings shared by all layers. Empty unless there is something to set
    fn render_view_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let sync_modes = !self.layout.overlaid() && self.sync;
        let lightbox = matches!(self.layout, Layout::Lightbox(_));
        if !sync_modes && self.frame_slider.is_none() && !lightbox {
            return div();
        }
        let lightbox_settings = lightbox.then(|| self.render_lightbox_settings(cx));
        let mode_button = |id: &'static str, label: &'static str, mode: SyncMode| {
            Button::new(id)
                .xsmall()
                .label(label)
                .selected(self.sync_mode == mode)
                .on_click(cx.listener(move |viewer, _, _, cx| {
                    viewer.sync_mode = mode;
                    viewer.sync_from_first();
                    cx.notify();
                }))
        };
        ui::panel("View", cx)
            .children(lightbox_settings)
            .when_some(self.frame_slider.as_ref(), |settings, slider| {
                settings.child(
                    ui::setting_row("Frame", muted)
                        .child(
                            v_flex()
                                .flex_1()
                                .gap_0p5()
                                .child(frame_tag(self.frame, slider.last_frame, cx))
                                .child(Slider::new(&slider.state))
                                .child(frame_ticks(slider.last_frame, muted)),
                        )
                        .child(
                            div()
                                .w(px(48.0))
                                .text_xs()
                                .child(format!("of {}", slider.last_frame + 1)),
                        ),
                )
            })
            .when(self.frame_slider.is_some(), |settings| {
                let playing = self.time_playing.is_some();
                let viewer = cx.entity().downgrade();
                settings.child(
                    ui::setting_row("Movie", muted)
                        .child(
                            Button::new("play")
                                .xsmall()
                                .icon(if playing { Carbon::Pause } else { Carbon::Play })
                                .label(if playing { "Pause" } else { "Play" })
                                .selected(playing)
                                .tooltip_with_action(
                                    "Play the frames",
                                    &TogglePlay,
                                    Some(shortcuts::CONTEXT),
                                )
                                .on_click(cx.listener(|viewer, _, window, cx| {
                                    viewer.toggle_play(&TogglePlay, window, cx);
                                })),
                        )
                        .child(
                            Button::new("frame-rate")
                                .ghost()
                                .xsmall()
                                .label(format!("{} fps", self.time_fps))
                                .tooltip("Frames per second")
                                .dropdown_menu(move |menu, _, _| {
                                    let choices = TIMEPOINT_FRAME_RATES
                                        .map(|rate| (format!("{rate} fps"), rate));
                                    ui::choice_items(
                                        menu,
                                        &viewer,
                                        choices,
                                        |viewer, rate, window, cx| {
                                            viewer.set_frames_per_second(rate, window, cx);
                                        },
                                    )
                                }),
                        ),
                )
            })
            .when(sync_modes, |settings| {
                settings.child(
                    ui::setting_row("Sync", muted)
                        .child(mode_button("sync-world", "World", SyncMode::World))
                        .child(mode_button(
                            "sync-proportional",
                            "Proportional",
                            SyncMode::Proportional,
                        )),
                )
            })
    }
}

/// The shown frames number above the slider's thumb
fn frame_tag(frame: usize, last_frame: usize, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    div().relative().w_full().h(px(16.0)).child(
        h_flex()
            .absolute()
            .top_0()
            .w_0()
            .left(relative(frame as f32 / last_frame as f32))
            .justify_center()
            .child(
                div()
                    .px_1()
                    .rounded(theme.radius)
                    .bg(theme.accent)
                    .text_color(theme.accent_foreground)
                    .text_xs()
                    .child((frame + 1).to_string()),
            ),
    )
}

fn frame_ticks(last_frame: usize, color: Hsla) -> impl IntoElement {
    div()
        .relative()
        .w_full()
        .h(px(4.0))
        .children((0..=last_frame).map(move |frame| {
            div()
                .absolute()
                .top_0()
                .h_full()
                .w(px(1.0))
                .left(relative(frame as f32 / last_frame as f32))
                .bg(color)
        }))
}

fn layer_details(display: &VolumeDisplay) -> Vec<(&'static str, String)> {
    let meta = display.volume().meta();
    let geometry = display.geometry();
    vec![
        ("File", display.name()),
        (
            "Size",
            format!("{} x {} x {}", meta.width(), meta.height(), meta.depth()),
        ),
        (
            "Voxel",
            format!(
                "{} x {} x {} {}",
                format_value(meta.spacing[0]),
                format_value(meta.spacing[1]),
                format_value(meta.spacing[2]),
                geometry.unit_name
            ),
        ),
        ("Frames", meta.timepoints().to_string()),
        ("Type", meta.datatype.name().to_string()),
        ("Format", meta.format.name().to_string()),
        ("Transform", geometry.transform_name.to_string()),
        ("Path", display.volume().path().display().to_string()),
    ]
}

/// The hover card with a layer's details
fn layer_card(details: &[(&'static str, String)], muted: Hsla) -> Div {
    v_flex()
        .w(px(360.0))
        .gap_1()
        .text_xs()
        .children(details.iter().map(|(label, value)| {
            h_flex()
                .gap_2()
                .items_start()
                .child(div().w(px(80.0)).text_color(muted).child(*label))
                .child(div().flex_1().min_w_0().child(value.clone()))
        }))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}
