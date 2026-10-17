//! Small UI helpers and colors that the viewer and dialogs share.

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::*;
use illuminator_core::colormap::Colormap;
use illuminator_core::labels::label_color;
use std::rc::Rc;

const CAPTION_WIDTH: f32 = 64.0;
pub const ORIENTATION_COLOR: u32 = 0xe8c547;
pub const ACTIVE_VIEW_COLOR: u32 = 0xc86b98;
pub const CROSSHAIR_COLOR: u32 = 0x00ccffb3; // cyan, 70% opacity

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

/// A editable `value` (number) with up to six significant digits.
pub fn format_editable(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    let decimals = (5 - value.abs().log10().floor() as i32).clamp(0, 15) as usize;
    let text = format!("{value:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

/// The number typed into a text field.
pub fn parse_value(text: &str) -> Option<f64> {
    // convert comma to dot instead of failing
    let value: f64 = text.trim().replace(',', ".").parse().ok()?;
    value.is_finite().then_some(value)
}

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

pub fn rgb_to_rgba([r, g, b]: [u8; 3]) -> Rgba {
    rgb(u32::from_be_bytes([0, r, g, b]))
}

/// Adds one menu item per (label, choice) to menu.
pub fn choice_items<E: 'static, L: Into<SharedString>, T: Copy + 'static>(
    mut menu: PopupMenu,
    entity: &WeakEntity<E>,
    choices: impl IntoIterator<Item = (L, T)>,
    choose: impl Fn(&mut E, T, &mut Window, &mut Context<E>) + 'static,
) -> PopupMenu {
    let choose = Rc::new(choose);
    for (label, choice) in choices {
        let (entity, choose) = (entity.clone(), Rc::clone(&choose));
        menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
            let _ = entity.update(cx, |entity, cx| choose(entity, choice, window, cx));
        }));
    }
    menu
}

#[cfg(test)]
mod tests {
    use super::{format_editable, format_value, parse_value};

    #[test]
    fn values_are_short() {
        assert_eq!(format_value(-1024.0), "-1024");
        assert_eq!(format_value(3.81234), "3.812");
    }

    #[test]
    fn editable_values_keep_six_digits() {
        assert_eq!(format_editable(1300.0000000000002), "1300");
        assert_eq!(format_editable(-1350.0), "-1350");
        assert_eq!(format_editable(2.5), "2.5");
        assert_eq!(format_editable(0.000123456789), "0.000123457");
        assert_eq!(format_editable(-0.0), "0");
        assert_eq!(format_editable(123456789.0), "123456789");
    }

    #[test]
    fn typed_values() {
        assert_eq!(parse_value(" -150 "), Some(-150.0));
        assert_eq!(parse_value("2,5"), Some(2.5));
        assert_eq!(parse_value("1e3"), Some(1000.0));
        assert_eq!(
            parse_value(&format_editable(0.000123456789)),
            Some(0.000123457)
        );
        for wrong in ["", "abc", "inf", "NaN", "1.2.3"] {
            assert_eq!(parse_value(wrong), None, "{wrong}");
        }
    }
}
