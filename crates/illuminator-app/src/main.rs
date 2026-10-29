//! The Illuminator viewer application.
//!
//! Usage: `illuminate [OPTIONS] [FILE]...` (see `illuminate --help`)

// no console on windows in release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod command_line;
mod label_statistics;
mod screenshot;
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

use crate::command_line::CommandLine;
use crate::single_instance::InstanceRole;
use crate::viewer::Viewer;

fn main() {
    attach_console();
    let command_line = CommandLine::read();
    let (open_request_sender, open_request_receiver) = mpsc::unbounded::<Vec<PathBuf>>();
    if command_line.has_only_files() {
        // if only plain files are passed, add them to the current open instance of Illuminator
        match single_instance::claim_primary_instance(
            &single_instance::socket_name(),
            &command_line.files,
        ) {
            Ok(InstanceRole::Forwarded) => return,
            Ok(InstanceRole::Primary(listener)) => {
                single_instance::listen_for_open_requests(listener, open_request_sender.clone())
            }
            Err(error) => eprintln!("Illuminator: cannot listen for other instances: {error}"),
        }
    }
    let (width, height) = command_line.size.unwrap_or((1440.0, 900.0));

    let cache = match VolumeCache::from_environment() {
        Ok(cache) => Arc::new(cache),
        Err(error) => {
            eprintln!("Illuminator: {error}");
            std::process::exit(1);
        }
    };

    let app = gpui_kit::application().with_assets(assets::ApplicationAssets);
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
        let _ = open_request_sender.unbounded_send(paths);
    });
    app.run(move |cx| {
        gpui_kit::init(cx);
        theme::initialize(cx);
        shortcuts::bind_keys(cx);
        gpui_kit::base::TooltipDefaults::new()
            .with_show_delay(std::time::Duration::from_millis(300))
            .install(cx);
        // quit when the last window closes
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        // delete uncompressed volumes when this session is ended
        let session_cache = Arc::clone(&cache);
        cx.on_app_quit(move |_| {
            let cache = Arc::clone(&session_cache);
            async move { cache.remove_session_entries() }
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
            cx.new(|cx| Viewer::new(cache, command_line, open_request_receiver, window, cx))
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
