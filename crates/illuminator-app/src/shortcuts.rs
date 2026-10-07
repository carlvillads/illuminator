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
        /// Draws the selected label map as outlines, or filled
        ToggleOutline,
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
        /// Shows or hides the text in the view corners
        ToggleInfo,
        ShowHelp,
        /// All layers overlaid in three planes
        LayoutOrtho,
        /// One column per layer three planes each
        LayoutCompare,
        /// One plane one cell per layer
        LayoutGrid,
        /// One large view
        LayoutSingle,
        /// Closes the selected layer
        CloseLayer,
        /// Loads the selected layers file again
        ReloadLayer,
        /// Moves the selected layer one step towards the base
        MoveLayerUp,
        /// Moves the selected layer one step away from the base
        MoveLayerDown,
    ]
);

pub const CONTEXT: &str = "Viewer";
const SECONDARY: &str = if cfg!(target_os = "macos") {
    "\u{2318}"
} else {
    "Ctrl+"
};
const ALT: &str = if cfg!(target_os = "macos") {
    "\u{2325}"
} else {
    "Alt+"
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
        KeyBinding::new("i", ToggleInfo, viewer),
        KeyBinding::new("?", ShowHelp, viewer),
        KeyBinding::new("o", ToggleOverlay, viewer),
        KeyBinding::new("shift-l", ToggleOutline, viewer),
        KeyBinding::new("v", ToggleLayer, viewer),
        KeyBinding::new("j", NextLayer, viewer),
        KeyBinding::new("k", PreviousLayer, viewer),
        KeyBinding::new("1", LayoutOrtho, viewer),
        KeyBinding::new("2", LayoutCompare, viewer),
        KeyBinding::new("3", LayoutGrid, viewer),
        KeyBinding::new("4", LayoutSingle, viewer),
        KeyBinding::new("secondary-backspace", CloseLayer, viewer),
        KeyBinding::new("secondary-r", ReloadLayer, viewer),
        KeyBinding::new("alt-k", MoveLayerUp, viewer),
        KeyBinding::new("alt-j", MoveLayerDown, viewer),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
}

pub fn help() -> Vec<(String, &'static str)> {
    vec![
        ("Click+drag".into(), "Move cursor"),
        ("Right+drag".into(), "Layer Window/level"),
        ("Scroll".into(), "Change slice"),
        ("Doubleclick".into(), "Focus/unfocus view"),
        ("Hover".into(), "Values in the status bar"),
        ("1-4".into(), "Layouts"),
        ("O".into(), "Overlay/side by side"),
        ("J/K".into(), "Next/previous layer"),
        ("V".into(), "Show/hide layer"),
        ("\u{2191}/\u{2193}".into(), "Next/previous slice"),
        ("Shift+\u{2191}/\u{2193}".into(), "Ten slices"),
        ("\u{2190}/\u{2192}".into(), "Previous/next frame"),
        ("H".into(), "Center cursor"),
        ("A".into(), "Auto contrast"),
        ("M".into(), "Next colormap"),
        ("L".into(), "Label map"),
        ("Shift+L".into(), "Label outlines"),
        ("S/R/C/I".into(), "Sync/radiological/crosshair/corner text"),
        ("?".into(), "This list"),
        (format!("{ALT}+J/K"), "Move layer down/up"),
        (format!("{SECONDARY}R"), "Reload layer from disk"),
        (format!("{SECONDARY}\u{232b}"), "Close layer"),
        ("Drag layer".into(), "Reorder layers"),
        (format!("{SECONDARY}O"), "Open volumes"),
        (format!("{SECONDARY}W/{SECONDARY}Q"), "Close window/quit"),
    ]
}
