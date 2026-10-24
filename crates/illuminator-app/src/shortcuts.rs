//! Keyboard shortcuts.

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
        /// Plays the frames of 4D volumes
        TogglePlay,
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
        /// Shows MIPs
        ToggleMip,
        ShowHelp,
        /// Copies a picture of the views to the clipboard
        CopyImage,
        /// Saves a picture of the views as an image
        SaveImage,
        /// All layers overlaid in three planes
        LayoutOrtho,
        /// One column per layer three planes each
        LayoutCompare,
        /// One plane one cell per layer
        LayoutGrid,
        /// One large view
        LayoutSingle,
        /// Lightbox view
        LayoutLightbox,
        /// Closes the selected layer
        CloseLayer,
        /// Loads the selected layers file again
        ReloadLayer,
        /// Moves the selected layer one step towards the base
        MoveLayerUp,
        /// Moves the selected layer one step away from the base
        MoveLayerDown,
        /// Zooms the active view's plane in
        ZoomIn,
        /// Zooms the active view's plane out
        ZoomOut,
        /// Shows every plane whole again
        ZoomReset,
        /// Opens histogram window
        ShowHistogram,
        /// Opens time series window
        ShowTimeSeries,
        /// Switches between the dark and the Platinum theme
        ToggleTheme,
    ]
);

pub const CONTEXT: &str = "Viewer";
pub const HISTOGRAM_CONTEXT: &str = "Histogram";
pub const TIME_SERIES_CONTEXT: &str = "TimeSeries";
const NOT_TYPING: &str = "Viewer && !Input";
pub const SECONDARY: &str = if cfg!(target_os = "macos") {
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
    let keys = Some(NOT_TYPING);
    cx.bind_keys([
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("secondary-w", CloseWindow, viewer),
        KeyBinding::new("secondary-w", CloseWindow, Some(HISTOGRAM_CONTEXT)),
        KeyBinding::new("secondary-w", CloseWindow, Some(TIME_SERIES_CONTEXT)),
        KeyBinding::new("secondary-o", OpenFiles, viewer),
        KeyBinding::new("secondary-c", CopyImage, viewer),
        KeyBinding::new("secondary-e", SaveImage, viewer),
        KeyBinding::new("up", NextSlice, keys),
        KeyBinding::new("down", PreviousSlice, keys),
        KeyBinding::new("shift-up", NextSlices, keys),
        KeyBinding::new("shift-down", PreviousSlices, keys),
        KeyBinding::new("right", NextFrame, keys),
        KeyBinding::new("left", PreviousFrame, keys),
        KeyBinding::new("space", TogglePlay, keys),
        KeyBinding::new("h", CenterCursor, keys),
        KeyBinding::new("a", AutoContrast, keys),
        KeyBinding::new("m", NextColormap, keys),
        KeyBinding::new("l", ToggleSegmentation, keys),
        KeyBinding::new("s", ToggleSync, keys),
        KeyBinding::new("r", ToggleRadiological, keys),
        KeyBinding::new("c", ToggleCrosshair, keys),
        KeyBinding::new("i", ToggleInfo, keys),
        KeyBinding::new("p", ToggleMip, keys),
        KeyBinding::new("?", ShowHelp, keys),
        KeyBinding::new("g", ShowHistogram, keys),
        KeyBinding::new("shift-g", ShowTimeSeries, keys),
        KeyBinding::new("t", ToggleTheme, keys),
        KeyBinding::new("o", ToggleOverlay, keys),
        KeyBinding::new("shift-l", ToggleOutline, keys),
        KeyBinding::new("v", ToggleLayer, keys),
        KeyBinding::new("j", NextLayer, keys),
        KeyBinding::new("k", PreviousLayer, keys),
        KeyBinding::new("1", LayoutOrtho, keys),
        KeyBinding::new("2", LayoutCompare, keys),
        KeyBinding::new("3", LayoutGrid, keys),
        KeyBinding::new("4", LayoutSingle, keys),
        KeyBinding::new("5", LayoutLightbox, keys),
        KeyBinding::new("secondary-backspace", CloseLayer, viewer),
        KeyBinding::new("secondary-r", ReloadLayer, viewer),
        KeyBinding::new("alt-k", MoveLayerUp, viewer),
        KeyBinding::new("alt-j", MoveLayerDown, viewer),
        KeyBinding::new("=", ZoomIn, keys),
        KeyBinding::new("-", ZoomOut, keys),
        KeyBinding::new("0", ZoomReset, keys),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &ToggleTheme, cx| crate::theme::toggle(cx));
}

pub fn help() -> Vec<(String, &'static str)> {
    vec![
        ("Click+drag".into(), "Move cursor"),
        ("Right+drag".into(), "Layer Window/level"),
        ("Scroll".into(), "Change slice"),
        (format!("{SECONDARY}scroll"), "Zoom at the mouse"),
        (format!("Middle+drag/{ALT}drag"), "Pan a zoomed view"),
        ("=/-/0".into(), "Zoom in/out/reset"),
        ("Doubleclick".into(), "Focus/unfocus view"),
        ("Hover".into(), "Values in the status bar"),
        ("1–5".into(), "Layouts"),
        ("Space".into(), "Play/pause frames"),
        ("O".into(), "Overlay/side by side"),
        ("J/K".into(), "Next/previous layer"),
        ("V".into(), "Show/hide layer"),
        ("\u{2191}/\u{2193}".into(), "Next/previous slice"),
        ("Shift+\u{2191}/\u{2193}".into(), "Next/previous ten slices"),
        ("\u{2190}/\u{2192}".into(), "Previous/next frame"),
        ("H".into(), "Center cursor"),
        ("A".into(), "Auto contrast"),
        ("M".into(), "Next colormap"),
        ("L".into(), "Label map"),
        ("Shift+L".into(), "Label outlines"),
        ("S/R/C/I".into(), "Sync/radiological/crosshair/corner text"),
        ("P".into(), "Maximum intensity projection (MIP)"),
        ("G".into(), "Histogram window"),
        ("Shift+G".into(), "Time series window"),
        ("T".into(), "Switch theme"),
        ("?".into(), "This list"),
        (format!("{ALT}J/K"), "Move layer down/up"),
        (format!("{SECONDARY}R"), "Reload layer from disk"),
        (format!("{SECONDARY}\u{232b}"), "Close layer"),
        ("Drag layer".into(), "Reorder layers"),
        (format!("{SECONDARY}O"), "Open volumes"),
        (
            format!("{SECONDARY}C/{SECONDARY}E"),
            "Copy/save the view as an image",
        ),
        (format!("{SECONDARY}W/{SECONDARY}Q"), "Close window/quit"),
    ]
}
