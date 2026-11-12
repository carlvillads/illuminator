//! Reports failures/errors that prevent the main window from showing up.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const DIALOG_TITLE: &str = "Illuminator could not start";
static FIRST_FRAME_DRAWN: AtomicBool = AtomicBool::new(false);

/// Writes message to stderr and shows it in a native popup dialog.
pub fn report(message: &str) {
    eprintln!("Illuminator: {message}");
    show_dialog(message);
}

pub fn report_and_exit(message: &str) -> ! {
    report(message);
    std::process::exit(1)
}

pub fn startup_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        default_hook(panic_info);
        if !FIRST_FRAME_DRAWN.load(Ordering::Relaxed) {
            show_dialog(&format!(
                "Illuminator stopped unexpectedly while starting.\n\n{panic_info}"
            ));
        }
    }));
}

pub fn record_first_frame() {
    FIRST_FRAME_DRAWN.store(true, Ordering::Relaxed);
}

pub fn watch_for_first_frame() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(15));
        if !FIRST_FRAME_DRAWN.load(Ordering::Relaxed) {
            report(
                "The window has not been drawn after 15 seconds. The graphics adapter may be bysy or unavailable. Illuminator is still trying to start...",
            );
        }
    });
}

#[cfg(windows)]
fn show_dialog(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    let to_wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain([0]).collect() };
    let text = to_wide(message);
    let caption = to_wide(DIALOG_TITLE);

    // SAFETY: both pointers refer to NUL-terminated UTF-16 buffers,
    // that outlive the call, and a null-owner window is permitted.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | MB_ICONERROR,
        )
    };
}

#[cfg(target_os = "macos")]
fn show_dialog(message: &str) {
    // text passed as args so they don't need AppleScript quoting
    let _ = std::process::Command::new("osascript")
        .args(["-e", "on run arguments"])
        .args([
            "-e",
            "display alert (item 1 of arguments) message (item 2 of arguments) as critical",
        ])
        .args(["-e", "end run"])
        .args([DIALOG_TITLE, message])
        .status();
}

#[cfg(all(unix, not(target_os = "macos")))]
fn show_dialog(message: &str) {
    let candidates: [(&str, &[&str]); 3] = [
        (
            "zenity",
            &[
                "--error",
                "--no-markup",
                "--title",
                DIALOG_TITLE,
                "--text",
                message,
            ],
        ),
        ("kdialog", &["--title", DIALOG_TITLE, "--error", message]),
        ("xmessage", &["-center", message]),
    ];
    for (program, arguments) in candidates {
        // err=not installed
        if std::process::Command::new(program)
            .args(arguments)
            .status()
            .is_ok()
        {
            return;
        }
    }
}
