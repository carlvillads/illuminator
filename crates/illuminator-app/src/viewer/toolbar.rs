//! The toolbar above the views.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, Selectable, Sizable, h_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{Action, Context, IntoElement, ParentElement, Styled, Window, div, px};
use illuminator_core::geometry::ViewPlane;

use crate::assets::Carbon;
use crate::shortcuts::{
    self, CopyImage, LayoutCompare, LayoutGrid, LayoutLightbox, LayoutOrtho, LayoutSingle,
    SECONDARY, SaveImage, ShowHelp, ToggleCrosshair, ToggleInfo, ToggleMip, ToggleRadiological,
    ToggleSync, ToggleTheme,
};
use crate::theme;

use super::{InfoLevel, Layout, TOOLBAR_HEIGHT, Viewer};

const TOOLBAR_LABELS_WIDTH: f32 = 1000.0;

impl Viewer {
    pub(super) fn render_toolbar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let (border, background, edge) = (theme.border, theme.title_bar, theme.title_bar_border);
        let labels = f32::from(window.viewport_size().width) >= TOOLBAR_LABELS_WIDTH;
        let context = Some(shortcuts::CONTEXT);
        let axis = self.current_axis();
        let viewer = cx.entity().downgrade();
        let layout_button = |id: &'static str,
                             icon: Carbon,
                             label: &'static str,
                             tooltip: &'static str,
                             action: &dyn Action,
                             layout: Layout| {
            Button::new(id)
                .ghost()
                .small()
                .icon(icon)
                .when(labels, |button| button.label(label))
                .selected(self.layout.same_kind(layout))
                .tooltip_with_action(tooltip, action, context)
                .on_click(cx.listener(move |viewer, _, _, cx| viewer.set_layout(layout, cx)))
        };
        let plane_button = |axis: ViewPlane, label: &'static str, tooltip: &'static str| {
            Button::new(label)
                .ghost()
                .small()
                .label(label)
                .tooltip(tooltip)
                .selected(self.layout.axis() == Some(axis))
                .on_click(cx.listener(move |viewer, _, _, cx| {
                    viewer.set_layout(viewer.layout.with_axis(axis), cx);
                }))
        };
        let separator = || div().w(px(1.0)).h(px(18.0)).mx_1().bg(border);

        h_flex()
            .h(px(TOOLBAR_HEIGHT))
            .flex_none()
            .px_2()
            .gap_1()
            .bg(background)
            .border_b_1()
            .border_color(edge)
            .child(layout_button(
                "layout-ortho",
                Carbon::Ortho,
                "Ortho",
                "All layers three planes",
                &LayoutOrtho,
                Layout::Ortho,
            ))
            .child(layout_button(
                "layout-compare",
                Carbon::Compare,
                "Compare",
                "Side by side three planes",
                &LayoutCompare,
                Layout::Compare,
            ))
            .child(layout_button(
                "layout-grid",
                Carbon::Grid,
                "Grid",
                "Side by side one plane",
                &LayoutGrid,
                Layout::Grid(axis),
            ))
            .child(layout_button(
                "layout-single",
                Carbon::Single,
                "Single",
                "Single view",
                &LayoutSingle,
                Layout::Single { axis, layer: None },
            ))
            .child(layout_button(
                "layout-lightbox",
                Carbon::Lightbox,
                "Lightbox",
                "Lightbox view",
                &LayoutLightbox,
                Layout::Lightbox(axis),
            ))
            .when(self.layout.axis().is_some(), |bar| {
                bar.child(separator())
                    .child(plane_button(ViewPlane::Axial, "A", "Axial"))
                    .child(plane_button(ViewPlane::Coronal, "C", "Coronal"))
                    .child(plane_button(ViewPlane::Sagittal, "S", "Sagittal"))
            })
            .child(separator())
            .child(
                Button::new("crosshair")
                    .ghost()
                    .small()
                    .icon(Carbon::Crosshair)
                    .when(labels, |button| button.label("Crosshair"))
                    .selected(self.show_crosshair)
                    .tooltip_with_action("Crosshair", &ToggleCrosshair, context)
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.toggle_crosshair(&ToggleCrosshair, window, cx);
                    })),
            )
            .child(
                Button::new("mip")
                    .ghost()
                    .small()
                    .icon(Carbon::Mip)
                    .when(labels, |button| button.label("MIP"))
                    .selected(self.show_mip)
                    .tooltip_with_action("Maximum Intensity Projection (MIP)", &ToggleMip, context)
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.toggle_mip(&ToggleMip, window, cx);
                    })),
            )
            .child(
                Button::new("radiological")
                    .ghost()
                    .small()
                    .icon(Carbon::Radiological)
                    .when(labels, |button| button.label("Radiological"))
                    .selected(self.radiological)
                    .tooltip_with_action("Radiological convention", &ToggleRadiological, context)
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.toggle_radiological(&ToggleRadiological, window, cx);
                    })),
            )
            .child(
                Button::new("info")
                    .ghost()
                    .small()
                    .icon(Carbon::CornerText)
                    .when(labels, |button| button.label("Overlay"))
                    .selected(self.info != InfoLevel::Off)
                    .tooltip_with_action(
                        format!("Text overlay: {}", self.info.name()),
                        &ToggleInfo,
                        context,
                    )
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.toggle_info(&ToggleInfo, window, cx);
                    })),
            )
            .when(!self.layout.overlaid(), |bar| {
                bar.child(
                    Button::new("sync")
                        .ghost()
                        .small()
                        .icon(Carbon::Link)
                        .when(labels, |button| button.label("Sync"))
                        .selected(self.sync)
                        .tooltip_with_action("Sync cursors", &ToggleSync, context)
                        .on_click(cx.listener(|viewer, _, window, cx| {
                            viewer.toggle_sync(&ToggleSync, window, cx);
                        })),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("image")
                    .ghost()
                    .small()
                    .icon(Carbon::Camera)
                    .when(labels, |button| button.label("Image"))
                    .tooltip(format!(
                        "Copy ({SECONDARY}C) or save ({SECONDARY}E) view as an image"
                    ))
                    .dropdown_menu(move |menu, _, _| {
                        let (copy, save) = (viewer.clone(), viewer.clone());
                        menu.item(PopupMenuItem::new("Copy image").on_click(
                            move |_, window, cx| {
                                let _ = copy.update(cx, |viewer, cx| {
                                    viewer.copy_image(&CopyImage, window, cx);
                                });
                            },
                        ))
                        .item(
                            PopupMenuItem::new("Save image\u{2026}").on_click(
                                move |_, window, cx| {
                                    let _ = save.update(cx, |viewer, cx| {
                                        viewer.save_image(&SaveImage, window, cx);
                                    });
                                },
                            ),
                        )
                    }),
            )
            .child(
                Button::new("theme")
                    .ghost()
                    .small()
                    .icon(Carbon::Theme)
                    .when(labels, |button| button.label("Theme"))
                    .selected(!cx.theme().is_dark())
                    .tooltip_with_action("Switch theme", &ToggleTheme, context)
                    .on_click(|_, _, cx| theme::toggle(cx)),
            )
            .child(
                Button::new("help")
                    .ghost()
                    .small()
                    .icon(Carbon::Help)
                    .when(labels, |button| button.label("Help"))
                    .tooltip_with_action("Shortcuts", &ShowHelp, context)
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.show_help(&ShowHelp, window, cx);
                    })),
            )
    }
}
