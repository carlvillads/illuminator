use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::*;
use illuminator_core::labels::label_color;
use illuminator_core::slicer::Colormap;
use std::sync::Arc;

const CAPTION_WIDTH: f32 = 64.0;
pub(crate) const ORIENTATION_COLOR: u32 = 0xe8c547;
pub(crate) const ACTIVE_VIEW_COLOR: u32 = 0xc86b98;
pub(crate) const CROSSHAIR_COLOR: u32 = 0x00ccffb3; // cyan, 70% opacity

pub trait AppStyle: Styled + Sized {
    fn truncate_start(self) -> Self {
        self.flex_1()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis_start()
    }
}

impl<E: Styled> AppStyle for E {}

pub fn setting_row(caption: &'static str, color: Hsla) -> Div {
    h_flex().gap_2().child(
        div()
            .w(px(CAPTION_WIDTH))
            .text_xs()
            .text_color(color)
            .child(caption),
    )
}

pub fn readout(text: impl Into<SharedString>, template: &'static str) -> Div {
    v_flex()
        .items_end()
        .child(text.into())
        .child(div().h_0().overflow_hidden().child(template))
}

pub fn format_value(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e9 {
        format!("{value:.0}")
    } else {
        format!("{value:.3}")
    }
}

pub fn section_title(text: impl Into<SharedString>, cx: &App) -> Div {
    let theme = cx.theme();
    h_flex()
        .gap_2()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.muted_foreground)
        .child(text.into())
        .child(div().flex_1().h(px(1.0)).bg(theme.border))
}

pub fn colormap_strip(colormap: Colormap, labels: bool, blocks: usize) -> Div {
    h_flex()
        .w_full()
        .h(px(10.0))
        .rounded_sm()
        .overflow_hidden()
        .children((0..blocks).map(|block| {
            let color = if labels {
                let [r, g, b, _] = label_color(block as i64 + 1);
                [r, g, b]
            } else {
                colormap.color((block as f64 + 0.5) / blocks as f64)
            };
            div().flex_1().h_full().bg(rgb_to_rgba(color))
        }))
}

pub fn tabular_nums() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}

pub fn rgb_to_rgba([r, g, b]: [u8; 3]) -> Rgba {
    rgb(u32::from_be_bytes([0, r, g, b]))
}

#[cfg(test)]
mod tests {
    use super::format_value;

    #[test]
    fn values_are_short() {
        assert_eq!(format_value(-1024.0), "-1024");
        assert_eq!(format_value(3.81234), "3.812");
    }
}
