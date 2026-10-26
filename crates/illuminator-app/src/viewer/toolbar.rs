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

use crate::assets::Carbon;
use crate::shortcuts::{
    self, CopyImage, LayoutCompare, LayoutGrid, LayoutLightbox, LayoutOrtho, LayoutSingle,
    SECONDARY, SaveImage, ShowHelp, ToggleCrosshair, ToggleInfo, ToggleMip, ToggleRadiological,
    ToggleSync, ToggleTheme,
};
use crate::theme;

use super::{InfoLevel, Layout, TOOLBAR_HEIGHT, Viewer, window_frame};

const BUTTON_PADDING: f32 = 0.5;
const TOOLBAR_GAP: f32 = 0.25;
const TOOLBAR_PADDING: f32 = 0.5;
const SEPARATOR_MARGIN: f32 = 0.25;
const SEPARATOR_WIDTH: f32 = 1.0;

/// A toolbar button that shows its text or its icon
#[derive(Clone, Copy)]
struct Tool {
    id: &'static str,
    icon: Carbon,
    text: &'static str,
}

const ORTHO: Tool = Tool {
    id: "layout-ortho",
    icon: Carbon::Ortho,
    text: "Ortho",
};
const COMPARE: Tool = Tool {
    id: "layout-compare",
    icon: Carbon::Compare,
    text: "Compare",
};
const GRID: Tool = Tool {
    id: "layout-grid",
    icon: Carbon::Grid,
    text: "Grid",
};
const SINGLE: Tool = Tool {
    id: "layout-single",
    icon: Carbon::Single,
    text: "Single",
};
const LIGHTBOX: Tool = Tool {
    id: "layout-lightbox",
    icon: Carbon::Lightbox,
    text: "Lightbox",
};
const CROSSHAIR: Tool = Tool {
    id: "crosshair",
    icon: Carbon::Crosshair,
    text: "Crosshair",
};
const MIP: Tool = Tool {
    id: "mip",
    icon: Carbon::Mip,
    text: "MIP",
};
const RADIOLOGICAL: Tool = Tool {
    id: "radiological",
    icon: Carbon::Radiological,
    text: "Radiological",
};
const OVERLAY: Tool = Tool {
    id: "info",
    icon: Carbon::CornerText,
    text: "Overlay",
};
const SYNC: Tool = Tool {
    id: "sync",
    icon: Carbon::Link,
    text: "Sync",
};
const IMAGE: Tool = Tool {
    id: "image",
    icon: Carbon::Camera,
    text: "Image",
};
const THEME: Tool = Tool {
    id: "theme",
    icon: Carbon::Theme,
    text: "Theme",
};
const HELP: Tool = Tool {
    id: "help",
    icon: Carbon::Help,
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
        let (border, background, edge) = (theme.border, theme.title_bar, theme.title_bar_border);
        let planes = self.layout.axis().is_some();
        let sync = !self.layout.overlaid();
        let text = toolbar_fits_text(planes, sync, window, cx);
        let context = Some(shortcuts::CONTEXT);
        let axis = self.current_axis();
        let viewer = cx.entity().downgrade();
        let tool_button = |tool: Tool| {
            let button = Button::new(tool.id).ghost().small();
            if text {
                button.label(tool.text)
            } else {
                button.icon(tool.icon)
            }
        };
        let layout_button =
            |tool: Tool, tooltip: &'static str, action: &dyn Action, layout: Layout| {
                tool_button(tool)
                    .selected(self.layout.same_kind(layout))
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
            .border_color(edge)
            .child(layout_button(
                ORTHO,
                "All layers three planes",
                &LayoutOrtho,
                Layout::Ortho,
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
            .when(planes, |bar| {
                bar.child(separator()).children(PLANES.map(plane_button))
            })
            .child(separator())
            .child(
                tool_button(CROSSHAIR)
                    .selected(self.show_crosshair)
                    .tooltip_with_action("Crosshair", &ToggleCrosshair, context)
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.toggle_crosshair(&ToggleCrosshair, window, cx);
                    })),
            )
            .child(
                tool_button(MIP)
                    .selected(self.show_mip)
                    .tooltip_with_action("Maximum Intensity Projection (MIP)", &ToggleMip, context)
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.toggle_mip(&ToggleMip, window, cx);
                    })),
            )
            .child(
                tool_button(RADIOLOGICAL)
                    .selected(self.radiological)
                    .tooltip_with_action("Radiological convention", &ToggleRadiological, context)
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.toggle_radiological(&ToggleRadiological, window, cx);
                    })),
            )
            .child(
                tool_button(OVERLAY)
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
            .when(sync, |bar| {
                bar.child(
                    tool_button(SYNC)
                        .selected(self.sync)
                        .tooltip_with_action("Sync cursors", &ToggleSync, context)
                        .on_click(cx.listener(|viewer, _, window, cx| {
                            viewer.toggle_sync(&ToggleSync, window, cx);
                        })),
                )
            })
            .child(div().flex_1())
            .child(
                tool_button(IMAGE)
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
                tool_button(THEME)
                    .selected(!cx.theme().is_dark())
                    .tooltip_with_action("Switch theme", &ToggleTheme, context)
                    .on_click(|_, _, cx| theme::toggle(cx)),
            )
            .child(
                tool_button(HELP)
                    .tooltip_with_action("Shortcuts", &ShowHelp, context)
                    .on_click(cx.listener(|viewer, _, window, cx| {
                        viewer.show_help(&ShowHelp, window, cx);
                    })),
            )
    }
}

/// Whether the window is wide enough for the text of every toolbar button
fn toolbar_fits_text(planes: bool, sync: bool, window: &Window, cx: &App) -> bool {
    let mut texts = [
        ORTHO,
        COMPARE,
        GRID,
        SINGLE,
        LIGHTBOX,
        CROSSHAIR,
        MIP,
        RADIOLOGICAL,
        OVERLAY,
        IMAGE,
        THEME,
        HELP,
    ]
    .map(|tool| tool.text)
    .to_vec();
    if sync {
        texts.push(SYNC.text);
    }
    if planes {
        texts.extend(PLANES.map(|(_, text, _)| text));
    }
    let separators = 1 + usize::from(planes);
    let children = texts.len() + separators + 1;
    let font_size = rems(0.875).to_pixels(window.rem_size());
    let font = theme::ui_font(cx);
    let text_width: f32 = texts
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
    let needed = text_width
        + texts.len() as f32 * 2.0 * BUTTON_PADDING * rem
        + (children - 1) as f32 * TOOLBAR_GAP * rem
        + separators as f32 * (SEPARATOR_WIDTH + 2.0 * SEPARATOR_MARGIN * rem)
        + 2.0 * TOOLBAR_PADDING * rem;
    let frame = window_frame(window);
    let available = f32::from(window.viewport_size().width) - frame.left - frame.right;
    needed <= available
}
