//! The toolbar above the views.
//! Its buttons show their text, or their icons when the text of every button doesn't fit.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, Selectable, Sizable, h_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    Action, App, Context, Hsla, IntoElement, ParentElement, Styled, TextRun, Window, div, px, rems,
};
use illuminator_core::geometry::ViewPlane;

use crate::assets::ApplicationUiIcon;
use crate::shortcuts::{
    self, CopyScreenshot, CycleCornerText, LayoutCompare, LayoutGrid, LayoutLightbox,
    LayoutOrthogonal, LayoutSingle, SECONDARY_MODIFIER, SaveScreenshot, ShowHelp, ToggleCrosshair,
    ToggleCursorSynchronization, ToggleMip, ToggleRadiological, ToggleTheme,
};
use crate::theme;

use super::{CornerTextLevel, Layout, TOOLBAR_HEIGHT, Viewer, window_insets};

const BUTTON_PADDING: f32 = 0.5;
const TOOLBAR_GAP: f32 = 0.25;
const TOOLBAR_PADDING: f32 = 0.5;
const SEPARATOR_MARGIN: f32 = 0.25;
const SEPARATOR_WIDTH: f32 = 1.0;

/// A toolbar button that shows its text or its icon
#[derive(Clone, Copy)]
struct ToolbarButton {
    id: &'static str,
    icon: ApplicationUiIcon,
    text: &'static str,
}

const ORTHOGONAL: ToolbarButton = ToolbarButton {
    id: "layout-ortho",
    icon: ApplicationUiIcon::Orthogonal,
    text: "Ortho",
};
const COMPARE: ToolbarButton = ToolbarButton {
    id: "layout-compare",
    icon: ApplicationUiIcon::Compare,
    text: "Compare",
};
const GRID: ToolbarButton = ToolbarButton {
    id: "layout-grid",
    icon: ApplicationUiIcon::Grid,
    text: "Grid",
};
const SINGLE: ToolbarButton = ToolbarButton {
    id: "layout-single",
    icon: ApplicationUiIcon::SingleView,
    text: "Single",
};
const LIGHTBOX: ToolbarButton = ToolbarButton {
    id: "layout-lightbox",
    icon: ApplicationUiIcon::Lightbox,
    text: "Lightbox",
};
const CROSSHAIR: ToolbarButton = ToolbarButton {
    id: "crosshair",
    icon: ApplicationUiIcon::Crosshair,
    text: "Crosshair",
};
const MIP: ToolbarButton = ToolbarButton {
    id: "mip",
    icon: ApplicationUiIcon::Mip,
    text: "MIP",
};
const RADIOLOGICAL: ToolbarButton = ToolbarButton {
    id: "radiological",
    icon: ApplicationUiIcon::Radiological,
    text: "Radiological",
};
const CORNER_TEXT: ToolbarButton = ToolbarButton {
    id: "corner-text",
    icon: ApplicationUiIcon::CornerText,
    text: "Overlay",
};
const CURSOR_SYNCHRONIZATION: ToolbarButton = ToolbarButton {
    id: "sync",
    icon: ApplicationUiIcon::CursorSynchronization,
    text: "Sync",
};
const SCREENSHOT: ToolbarButton = ToolbarButton {
    id: "image",
    icon: ApplicationUiIcon::Camera,
    text: "Screenshot",
};
const THEME: ToolbarButton = ToolbarButton {
    id: "theme",
    icon: ApplicationUiIcon::Theme,
    text: "Theme",
};
const HELP: ToolbarButton = ToolbarButton {
    id: "help",
    icon: ApplicationUiIcon::Help,
    text: "Help",
};

/// The plane buttons are short enough to always show their text
const PLANES: [(ViewPlane, &str, &str); 3] = [
    (ViewPlane::Axial, "A", "Axial"),
    (ViewPlane::Coronal, "C", "Coronal"),
    (ViewPlane::Sagittal, "S", "Sagittal"),
];

impl Viewer {
    pub(super) fn render_toolbar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let (border, background, bottom_border) =
            (theme.border, theme.title_bar, theme.title_bar_border);
        let show_plane_buttons = self.layout.axis().is_some();
        let show_synchronization_button = !self.layout.is_overlaid();
        let show_labels =
            toolbar_fits_text(show_plane_buttons, show_synchronization_button, window, cx);
        let context = Some(shortcuts::VIEWER_CONTEXT);
        let axis = self.current_axis();
        let viewer = cx.entity().downgrade();
        let toolbar_button = |tool: ToolbarButton| {
            let button = Button::new(tool.id).ghost().small();
            if show_labels {
                button.label(tool.text)
            } else {
                button.icon(tool.icon)
            }
        };
        let layout_button =
            |tool: ToolbarButton, tooltip: &'static str, action: &dyn Action, layout: Layout| {
                toolbar_button(tool)
                    .selected(self.layout.same_variant(layout))
                    .tooltip_with_action(tooltip, action, context)
                    .on_click(cx.listener(move |viewer, _, _, cx| viewer.set_layout(layout, cx)))
            };
        let plane_button = |(axis, text, tooltip): (ViewPlane, &'static str, &'static str)| {
            Button::new(text)
                .ghost()
                .small()
                .label(text)
                .tooltip(tooltip)
                .selected(self.layout.axis() == Some(axis))
                .on_click(cx.listener(move |viewer, _, _, cx| {
                    viewer.set_layout(viewer.layout.with_axis(axis), cx);
                }))
        };
        let separator = || div().w(px(SEPARATOR_WIDTH)).h(px(18.0)).mx_1().bg(border);

        h_flex()
            .h(px(TOOLBAR_HEIGHT))
            .flex_none()
            .px_2()
            .gap_1()
            .bg(background)
            .border_b_1()
            .border_color(bottom_border)
            .child(layout_button(
                ORTHOGONAL,
                "All layers three planes",
                &LayoutOrthogonal,
                Layout::Orthogonal,
            ))
            .child(layout_button(
                COMPARE,
                "Side by side three planes",
                &LayoutCompare,
                Layout::Compare,
            ))
            .child(layout_button(
                GRID,
                "Side by side one plane",
                &LayoutGrid,
                Layout::Grid(axis),
            ))
            .child(layout_button(
                SINGLE,
                "Single view",
                &LayoutSingle,
                Layout::Single { axis, layer: None },
            ))
            .child(layout_button(
                LIGHTBOX,
                "Lightbox view",
                &LayoutLightbox,
                Layout::Lightbox(axis),
            ))
            .when(show_plane_buttons, |bar| {
                bar.child(separator()).children(PLANES.map(plane_button))
            })
            .child(separator())
            .child(
                toolbar_button(CROSSHAIR)
                    .selected(self.show_crosshair)
                    .tooltip_with_action("Crosshair", &ToggleCrosshair, context)
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.toggle_crosshair(&ToggleCrosshair, window, cx);
                    })),
            )
            .child(
                toolbar_button(MIP)
                    .selected(self.show_mip)
                    .tooltip_with_action("Maximum Intensity Projection (MIP)", &ToggleMip, context)
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.toggle_mip(&ToggleMip, window, cx);
                    })),
            )
            .child(
                toolbar_button(RADIOLOGICAL)
                    .selected(self.radiological)
                    .tooltip_with_action("Radiological convention", &ToggleRadiological, context)
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.toggle_radiological(&ToggleRadiological, window, cx);
                    })),
            )
            .child(
                toolbar_button(CORNER_TEXT)
                    .selected(self.corner_text != CornerTextLevel::Off)
                    .tooltip_with_action(
                        format!("Text overlay: {}", self.corner_text.name()),
                        &CycleCornerText,
                        context,
                    )
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.cycle_corner_text(&CycleCornerText, window, cx);
                    })),
            )
            .when(show_synchronization_button, |bar| {
                bar.child(
                    toolbar_button(CURSOR_SYNCHRONIZATION)
                        .selected(self.cursor_synchronization)
                        .tooltip_with_action("Sync cursors", &ToggleCursorSynchronization, context)
                        .on_click(cx.listener(|viewer, _, window, cx| {
                            viewer.toggle_curson_synchronization(&ToggleCursorSynchronization, window, cx);
                        })),
                )
            })
            .child(div().flex_1())
            .child(
                toolbar_button(SCREENSHOT)
                    .tooltip(format!(
                        "Copy ({SECONDARY_MODIFIER}C) or save ({SECONDARY_MODIFIER}E) view as an image"
                    ))
                    .dropdown_menu(move |menu, _, _| {
                        let (copy, save) = (viewer.clone(), viewer.clone());
                        menu.item(PopupMenuItem::new("Copy image").on_click(
                            move |_, window, cx| {
                                let _ = copy.update(cx, |viewer, cx| {
                                    viewer.copy_screenshot(&CopyScreenshot, window, cx);
                                });
                            },
                        ))
                        .item(
                            PopupMenuItem::new("Save image\u{2026}").on_click(
                                move |_, window, cx| {
                                    let _ = save.update(cx, |viewer, cx| {
                                        viewer.save_screenshot(&SaveScreenshot, window, cx);
                                    });
                                },
                            ),
                        )
                    }),
            )
            .child(
                toolbar_button(THEME)
                    .selected(!cx.theme().is_dark())
                    .tooltip_with_action("Switch theme", &ToggleTheme, context)
                    .on_click(|_, _, cx| theme::toggle_theme_mode(cx)),
            )
            .child(
                toolbar_button(HELP)
                    .tooltip_with_action("Shortcuts", &ShowHelp, context)
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.show_help(&ShowHelp, window, cx);
                    })),
            )
    }
}

/// Whether the window is wide enough for the text of every toolbar button
fn toolbar_fits_text(
    show_plane_buttons: bool,
    show_synchronization_button: bool,
    window: &Window,
    cx: &App,
) -> bool {
    let mut labels = [
        ORTHOGONAL,
        COMPARE,
        GRID,
        SINGLE,
        LIGHTBOX,
        CROSSHAIR,
        MIP,
        RADIOLOGICAL,
        CORNER_TEXT,
        SCREENSHOT,
        THEME,
        HELP,
    ]
    .map(|tool| tool.text)
    .to_vec();
    if show_synchronization_button {
        labels.push(CURSOR_SYNCHRONIZATION.text);
    }
    if show_plane_buttons {
        labels.extend(PLANES.map(|(_, text, _)| text));
    }
    let separators = 1 + usize::from(show_plane_buttons);
    let children = labels.len() + separators + 1;
    let font_size = rems(0.875).to_pixels(window.rem_size());
    let font = theme::interface_font(cx);
    let text_width: f32 = labels
        .iter()
        .map(|&text| {
            let run = TextRun {
                len: text.len(),
                font: font.clone(),
                color: Hsla::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let line = window
                .text_system()
                .shape_line(text.into(), font_size, &[run], None);
            f32::from(line.width)
        })
        .sum();

    let rem = f32::from(window.rem_size());
    let required_width = text_width
        + labels.len() as f32 * 2.0 * BUTTON_PADDING * rem
        + (children - 1) as f32 * TOOLBAR_GAP * rem
        + separators as f32 * (SEPARATOR_WIDTH + 2.0 * SEPARATOR_MARGIN * rem)
        + 2.0 * TOOLBAR_PADDING * rem;
    let frame = window_insets(window);
    let available = f32::from(window.viewport_size().width) - frame.left - frame.right;
    required_width <= available
}
