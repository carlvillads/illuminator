use gpui_kit::component::h_flex;
use gpui_kit::*;

const CAPTION_WIDTH: f32 = 64.0;

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
