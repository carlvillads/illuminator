//! Illuminator application theming.

use gpui_kit::base::{Root, RootPlugin};
use gpui_kit::component::{Theme, ThemeRegistry};
use gpui_kit::*;
use std::borrow::Cow;
use std::sync::Arc;

const THEME: &str = include_str!("../assets/theming/theme.json");
const THEME_NAME: &str = "Illuminated";

/// See illuminator/crates/illuminator-app/assets/fonts/ for licenses.
const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/inter/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/inter/Inter-Bold.ttf"),
    include_bytes!("../assets/fonts/paper/PaperMono-Regular.ttf"),
];
const UI_FONT_FEATURES: &[&str] = &[
    "calt", "tnum", "ss07", "ss08", "cv02", "cv10", "cv08", "cv05",
];

/// init theming
pub fn init(cx: &mut App) {
    let fonts = FONTS.iter().map(|font| Cow::Borrowed(*font)).collect();
    if let Err(error) = cx.text_system().add_fonts(fonts) {
        eprintln!("Illuminator: could not load fonts: {error}");
    }
    ThemeRegistry::global_mut(cx)
        .load_themes_from_str(THEME)
        .expect("bundled theme is valid JSON");
    let config = ThemeRegistry::global(cx).themes()[THEME_NAME].clone();
    Theme::update(cx, |theme| theme.apply_config(&config));
    Root::register_plugin(cx, |_, _| {
        let features = UI_FONT_FEATURES.iter().map(|tag| (tag.to_string(), 1));
        InterfaceFont(FontFeatures(Arc::new(features.collect())))
    });
    cx.set_reduce_motion(true);
    watch(cx);
}

struct InterfaceFont(FontFeatures);

impl RootPlugin for InterfaceFont {
    fn style(&self, surface: &mut Stateful<Div>, _: &mut Window, _: &mut App) {
        surface.text_style().font_features = Some(self.0.clone());
    }
}

impl Render for InterfaceFont {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// Reload theme whenever a file is changed during debugging
#[cfg(debug_assertions)]
fn watch(cx: &mut App) {
    let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/theming"));
    if let Err(error) = ThemeRegistry::watch_dir(dir, cx, |_| {}) {
        eprintln!("Illuminator: cannot watch themes: {error}");
    }
}

#[cfg(not(debug_assertions))]
fn watch(_: &mut App) {}

#[cfg(test)]
mod tests {
    use super::{THEME, THEME_NAME};
    use gpui_kit::component::ThemeSet;

    #[test]
    fn bundled_theme_parses() {
        let set: ThemeSet = serde_json::from_str(THEME).unwrap();
        assert!(set.themes.iter().any(|theme| theme.name == THEME_NAME));
    }
}
