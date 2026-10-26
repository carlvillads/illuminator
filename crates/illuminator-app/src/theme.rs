//! Illuminator application theming.

use gpui_kit::base::{Root, RootPlugin};
use gpui_kit::component::{ActiveTheme, Theme, ThemeMode, ThemeRegistry};
use gpui_kit::*;
use illuminator_core::geometry::ViewPlane;
use std::borrow::Cow;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

const THEMES: &str = include_str!("../assets/theming/theme.json");
const THEME_NAME_DARK: &str = "Illuminated";
const THEME_NAME_LIGHT: &str = "Platinum";

/// See illuminator/crates/illuminator-app/assets/fonts/ for licenses.
const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/inter/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/inter/Inter-Bold.ttf"),
    include_bytes!("../assets/fonts/paper/PaperMono-Regular.ttf"),
];
const UI_FONT_FEATURES: &[&str] = &[
    "calt", "tnum", "ss07", "ss08", "cv02", "cv10", "cv08", "cv05",
];

pub const VIEW_BACKGROUND: u32 = 0x000000;
pub const VIEW_TEXT_COLOR: u32 = 0x999999;
pub const ORIENTATION_COLOR: u32 = 0xe8c547;
const CROSSHAIR_COLOR_SAGITTAL: u32 = 0xff3352;
const CROSSHAIR_COLOR_CORONAL: u32 = 0x8bdc00;
const CROSSHAIR_COLOR_AXIAL: u32 = 0x2890ff;
const CROSSHAIR_ALPHA: u32 = 0xb3; // 70% opacity
pub const HISTOGRAM_CURVE_COLORS_DARK: [u32; 8] = [
    0x5aa9ff, 0xf5a65b, 0x6fcf97, 0xf2757a, 0xc39bff, 0xf2d15b, 0x56d4e0, 0xf78fc4,
];
pub const HISTOGRAM_CURVE_COLORS_LIGHT: [u32; 8] = [
    0x1f6fd1, 0xc8650a, 0x23874d, 0xc93a44, 0x7b4fd0, 0x9a7a00, 0x12899a, 0xc0428a,
];

/// init theming
pub fn init(cx: &mut App) {
    let fonts = FONTS.iter().map(|font| Cow::Borrowed(*font)).collect();
    if let Err(error) = cx.text_system().add_fonts(fonts) {
        eprintln!("Illuminator: could not load fonts: {error}");
    }
    ThemeRegistry::global_mut(cx)
        .load_themes_from_str(THEMES)
        .expect("bundled themes are valid JSON");
    let themes = ThemeRegistry::global(cx).themes();
    let (dark, light) = (
        themes[THEME_NAME_DARK].clone(),
        themes[THEME_NAME_LIGHT].clone(),
    );
    Theme::update(cx, |theme| {
        theme.dark_theme = dark;
        theme.light_theme = light;
    });
    let mode = if saved_choice().as_deref() == Some(THEME_NAME_LIGHT) {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    Theme::change(mode, None, cx);
    Root::register_plugin(cx, |_, _| InterfaceFont(ui_font_features()));
    cx.set_reduce_motion(true);
    watch(cx);
}

pub fn ui_font(cx: &App) -> Font {
    Font {
        features: ui_font_features(),
        ..font(cx.theme().font_family.clone())
    }
}

fn ui_font_features() -> FontFeatures {
    let features = UI_FONT_FEATURES.iter().map(|tag| (tag.to_string(), 1));
    FontFeatures(Arc::new(features.collect()))
}

/// Switches every window between the dark and the light theme.
pub fn toggle(cx: &mut App) {
    let mode = if cx.theme().is_dark() {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    Theme::change(mode, None, cx);
    save_choice(cx.theme().theme_name());
}

/// The file that remembers the chosen theme between launches.
fn choice_file() -> Option<PathBuf> {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let config = if cfg!(target_os = "macos") {
        home()?.join("Library/Application Support")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else {
        match std::env::var_os("XDG_CONFIG_HOME") {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir),
            _ => home()?.join(".config"),
        }
    };
    Some(config.join("illuminator").join("theme"))
}

fn saved_choice() -> Option<String> {
    let text = fs::read_to_string(choice_file()?).ok()?;
    Some(text.trim().to_string())
}

fn save_choice(name: &str) {
    let Some(file) = choice_file() else {
        return;
    };
    let saved = file
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(&file, name));
    if let Err(error) = saved {
        eprintln!("Illuminator: cannot remember theme: {error}");
    }
}

/// The histogram curve color of the layer at index
pub fn curve_color(index: usize, cx: &App) -> Hsla {
    let colors = if cx.theme().is_dark() {
        &HISTOGRAM_CURVE_COLORS_DARK
    } else {
        &HISTOGRAM_CURVE_COLORS_LIGHT
    };
    rgb(colors[index % colors.len()]).into()
}

pub fn plane_color(plane: ViewPlane) -> u32 {
    match plane {
        ViewPlane::Sagittal => CROSSHAIR_COLOR_SAGITTAL,
        ViewPlane::Coronal => CROSSHAIR_COLOR_CORONAL,
        ViewPlane::Axial => CROSSHAIR_COLOR_AXIAL,
    }
}

pub fn crosshair_colors(axis: ViewPlane) -> [u32; 2] {
    let planes = match axis {
        ViewPlane::Axial => [ViewPlane::Sagittal, ViewPlane::Coronal],
        ViewPlane::Coronal => [ViewPlane::Sagittal, ViewPlane::Axial],
        ViewPlane::Sagittal => [ViewPlane::Coronal, ViewPlane::Axial],
    };
    planes.map(|plane| plane_color(plane) << 8 | CROSSHAIR_ALPHA)
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
    use super::{THEME_NAME_DARK, THEME_NAME_LIGHT, THEMES};
    use gpui_kit::component::{ThemeMode, ThemeSet};

    #[test]
    fn bundled_themes_parse_with_their_modes() {
        let set: ThemeSet = serde_json::from_str(THEMES).unwrap();
        let mode = |name| {
            set.themes
                .iter()
                .find(|theme| theme.name == name)
                .map(|theme| theme.mode)
        };
        assert_eq!(mode(THEME_NAME_DARK), Some(ThemeMode::Dark));
        assert_eq!(mode(THEME_NAME_LIGHT), Some(ThemeMode::Light));
    }
}
