//! Keyboard shortcuts

use gpui_kit::*;

actions!(
    illuminator,
    [
        /// Quits the application
        Quit,
        /// Closes window
        CloseWindow,
        /// Shows "open file" dialog
        OpenFiles,
        /// Moves the active panel one slice forward
        NextSlice,
        /// Moves the active panel one slice back
        PreviousSlice,
        /// Moves the active panel 10 slices forward
        NextSlices,
        /// Moves the active panel 10 slices back
        PreviousSlices,
        /// Shows the next frame of 4D volumes
        NextFrame,
        /// Shows the previous frame of 4D volumes
        PreviousFrame,
        /// Puts the active volume's cursor back on its center voxel
        CenterCursor,
        /// Fits the active volume's contrast to its current frame
        AutoContrast,
        /// Switches the active volume to the next colormap
        NextColormap,
        /// Turns segmentation display on or off for the active volume
        ToggleSegmentation,
        /// Switches between overlaid layers and side by side columns
        ToggleOverlay,
        /// Shows or hides the selected layer
        ToggleLayer,
        /// Selects the next layer
        NextLayer,
        /// Selects the previous layer
        PreviousLayer,
        /// Turns crosshair sync on or off
        ToggleSync,
        /// Switches between radiological and neurological orientation
        ToggleRadiological,
        /// Shows or hides the crosshairs
        ToggleCrosshair,
    ]
);

pub const CONTEXT: &str = "Viewer";
const SECONDARY: &str = if cfg!(target_os = "macos") {
    "\u{2318}"
} else {
    "Ctrl+"
};

pub fn bind(cx: &mut App) {
    let viewer = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("secondary-w", CloseWindow, viewer),
        KeyBinding::new("secondary-o", OpenFiles, viewer),
        KeyBinding::new("up", NextSlice, viewer),
        KeyBinding::new("down", PreviousSlice, viewer),
        KeyBinding::new("shift-up", NextSlices, viewer),
        KeyBinding::new("shift-down", PreviousSlices, viewer),
        KeyBinding::new("right", NextFrame, viewer),
        KeyBinding::new("left", PreviousFrame, viewer),
        KeyBinding::new("h", CenterCursor, viewer),
        KeyBinding::new("a", AutoContrast, viewer),
        KeyBinding::new("m", NextColormap, viewer),
        KeyBinding::new("l", ToggleSegmentation, viewer),
        KeyBinding::new("s", ToggleSync, viewer),
        KeyBinding::new("r", ToggleRadiological, viewer),
        KeyBinding::new("c", ToggleCrosshair, viewer),
        KeyBinding::new("o", ToggleOverlay, viewer),
        KeyBinding::new("v", ToggleLayer, viewer),
        KeyBinding::new("j", NextLayer, viewer),
        KeyBinding::new("k", PreviousLayer, viewer),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
}

pub fn help() -> Vec<(String, &'static str)> {
    vec![
        (format!("{SECONDARY}O"), "Open volumes"),
        ("\u{2191}/\u{2193}".into(), "Next/previous slice"),
        ("Shift+\u{2191}/\u{2193}".into(), "Ten slices"),
        ("\u{2190}/\u{2192}".into(), "Previous/next frame"),
        ("H".into(), "Center cursor"),
        ("A".into(), "Auto contrast"),
        ("M".into(), "Next colormap"),
        ("L".into(), "Segmentation map"),
        ("O".into(), "Overlay/side by side"),
        ("J/K".into(), "Next/previous layer"),
        ("V".into(), "Show/hide layer"),
        ("S/R/C".into(), "Sync/radiological/crosshairs"),
        (format!("{SECONDARY}W/{SECONDARY}Q"), "Close window/quit"),
    ]
}
