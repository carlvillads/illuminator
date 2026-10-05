//! Illuminator viwer application
//! Usage: illuminate [FILE...]

mod shortcuts;
mod viewer;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;
use illuminator_core::cache::VolumeCache;
use std::path::PathBuf;
use std::sync::Arc;

use crate::viewer::Viewer;

fn main() {
    let cache = match VolumeCache::from_env() {
        Ok(cache) => Arc::new(cache),
        Err(error) => {
            eprintln!("Illuminator: {error}");
            std::process::exit(1);
        }
    };
    let files: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        shortcuts::bind(cx);
        // quit when the last window closes
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1440.0), px(900.0)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("Illuminator".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Viewer::new(cache, files, window, cx))
        })
        .expect("cannot open main window");
        cx.activate(true);
    });
}
