//! The status bar.

use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::{ActiveTheme, h_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{Context, FontWeight, IntoElement, ParentElement, Styled, div, px};

use crate::ui::{self, format_value};

use super::{STATUS_BAR_HEIGHT, Viewer};

const POSITION_TEMPLATE: &str = "-000.0";
const VALUE_TEMPLATE: &str = "-0000.000";
const LABEL_TEMPLATE: &str = "00000";
const VOXEL_TEMPLATE: &str = "0000";

impl Viewer {
    /// Values of all layers at the mouse
    pub(super) fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, background) = (
            theme.muted_foreground,
            theme.status_bar_border,
            theme.status_bar,
        );
        let status = StatusBar::new()
            .h(px(STATUS_BAR_HEIGHT))
            .flex_none()
            .px_2()
            .gap_4()
            .text_xs()
            .bg(background)
            .border_t_1()
            .border_color(border);
        let Some(point) = self.hovered_point else {
            return status.left(div().text_color(muted).child("Nothing to read"));
        };
        let position = |value: f64| ui::readout(format!("{value:.1}"), POSITION_TEMPLATE);
        let mut status = status.left(
            h_flex()
                .child(position(point.x))
                .child(", ")
                .child(position(point.y))
                .child(", ")
                .child(position(point.z))
                .child(" mm"),
        );
        for (index, layer) in self.layers.iter().enumerate() {
            let value = layer
                .display
                .value_at_world_point(point)
                .map_or("-".to_string(), |(_, value)| format_value(value));
            let template = if layer.display.label_mode() {
                LABEL_TEMPLATE
            } else {
                VALUE_TEMPLATE
            };
            status = status.child(
                h_flex()
                    .ml_2()
                    .when(!layer.visible, |item| item.text_color(muted))
                    .when(index == self.selected_layer, |item| {
                        item.font_weight(FontWeight::SEMIBOLD)
                    })
                    .child(format!("{}: ", index + 1))
                    .child(ui::readout(value, template)),
            );
        }
        if let Some(index) = self.selected_layer() {
            let voxel = self.layers[index].display.value_at_world_point(point);
            let coordinate = |axis: usize| {
                let text = voxel.map_or("-".to_string(), |(voxel, _)| voxel[axis].to_string());
                ui::readout(text, VOXEL_TEMPLATE)
            };
            status = status.right(
                h_flex()
                    .child(format!("Layer {} voxel ", index + 1))
                    .child(coordinate(0))
                    .child(", ")
                    .child(coordinate(1))
                    .child(", ")
                    .child(coordinate(2)),
            );
        }
        status
    }
}
