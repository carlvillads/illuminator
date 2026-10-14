//! main.rs
//!
//! The Illuminator viewer application.
//! Usage: illuminate [OPTIONS] [FILE]... (see illuminate --help)
//!

// no console on windows in release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod cli;
mod label_statistics;
mod picture;
mod shortcuts;
mod single_instance;
mod suv_form;
mod theme;
mod ui;
mod viewer;
mod zoom;

use futures::channel::mpsc;
use gpui_kit::*;
use illuminator_core::cache::VolumeCache;
use std::path::PathBuf;
use std::sync::Arc;

use crate::cli::Cli;
use crate::single_instance::Instance;
use crate::viewer::Viewer;

fn main() {
    attach_console();
    let cli = Cli::read();
    let (opened, opened_files) = mpsc::unbounded::<Vec<PathBuf>>();
    if cli.only_files() {
        // if only plain files are passed, add them to the current open instance of Illuminator
        match single_instance::claim(&single_instance::socket_name(), &cli.files) {
            Ok(Instance::Forwarded) => return,
            Ok(Instance::First(listener)) => single_instance::listen(listener, opened.clone()),
            Err(error) => eprintln!("Illuminator: cannot listen for other instances: {error}"),
        }
    }
    let (width, height) = cli.size.unwrap_or((1440.0, 900.0));

    let cache = match VolumeCache::from_env() {
        Ok(cache) => Arc::new(cache),
        Err(error) => {
            eprintln!("Illuminator: {error}");
            std::process::exit(1);
        }
    };

    let app = gpui_kit::application().with_assets(assets::AppAssets);
    app.on_open_urls(move |urls| {
        let mut paths = Vec::new();
        for url in urls {
            match url::Url::parse(&url)
                .ok()
                .and_then(|url| url.to_file_path().ok())
            {
                Some(path) => paths.push(path),
                None => eprintln!("Illuminator: cannot open {url}"),
            }
        }
        let _ = opened.unbounded_send(paths);
    });
    app.run(move |cx| {
        gpui_kit::init(cx);
        theme::init(cx);
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
                size(px(width), px(height)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("Illuminator".into()),
                ..Default::default()
            }),
            app_id: Some("illuminator".into()),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Viewer::new(cache, cli, opened_files, window, cx))
        })
        .expect("cannot open main window");
        cx.activate(true);
    });
}

/// Release builds won't have a console of their own on Windows. We borrow the terminal that started
/// illuminator, if it was opened from one.
#[cfg(windows)]
fn attach_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // SAFETY: AttachConsole takes no pointers. Without a parent console it returns 0, which should be fine
    // to ignore.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

#[cfg(not(windows))]
fn attach_console() {}
