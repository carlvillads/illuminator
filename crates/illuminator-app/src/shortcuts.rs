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
        NextSlicePage,
        /// Moves the active panel 10 slices back
        PreviousSlicePage,
        /// Shows the next frame of 4D volumes
        NextFrame,
        /// Shows the previous frame of 4D volumes
        PreviousFrame,
        /// Plays the frames of 4D volumes
        TogglePlayback,
        /// Puts the active volume's cursor back on its center voxel
        CenterCursor,
        /// Fits the active volume's contrast to its current frame
        FitWindow,
        /// Switches the active volume to the next colormap
        NextColormap,
        /// Turns segmentation display on or off for the active volume
        ToggleLabelMode,
        /// Draws the selected label map as outlines, or filled
        ToggleOutline,
        /// Switches between overlaid layers and side by side columns
        ToggleOverlay,
        /// Shows or hides the selected layer
        ToggleLayerVisibility,
        /// Selects the next layer
        NextLayer,
        /// Selects the previous layer
        PreviousLayer,
        /// Turns crosshair sync on or off
        ToggleCursorSynchronization,
        /// Switches between radiological and neurological orientation
        ToggleRadiological,
        /// Shows or hides the crosshairs
        ToggleCrosshair,
        /// Shows or hides the text in the view corners
        CycleCornerText,
        /// Shows MIPs
        ToggleMip,
        ShowHelp,
        /// Copies a screenshot of the views to the clipboard
        CopyScreenshot,
        /// Saves a screenshot of the views as an image
        SaveScreenshot,
        /// All layers overlaid in three planes
        LayoutOrthogonal,
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
        ResetZoom,
        /// Opens histogram window
        ShowHistogramWindow,
        /// Opens time series window
        ShowTimeSeriesWindow,
        /// Switches between the dark and the Platinum theme
        ToggleTheme,
    ]
);

pub const VIEWER_CONTEXT: &str = "Viewer";
pub const PLOT_WINDOW_CONTEXT: &str = "Plot";
const VIEWER_OUTSIDE_TEXT_INPUT: &str = "Viewer && !Input";
pub const SECONDARY_MODIFIER: &str = if cfg!(target_os = "macos") {
    "\u{2318}"
} else {
    "Ctrl+"
};
const ALT_MODIFIER: &str = if cfg!(target_os = "macos") {
    "\u{2325}"
} else {
    "Alt+"
};

macro_rules! secondary_keystroke {
    ($key:literal) => {
        if cfg!(target_os = "macos") {
            concat!("cmd-", $key)
        } else {
            concat!("ctrl-", $key)
        }
    };
}

pub fn bind_keys(cx: &mut App) {
    let viewer_context = Some(VIEWER_CONTEXT);
    let outside_text_input = Some(VIEWER_OUTSIDE_TEXT_INPUT);
    cx.bind_keys([
        KeyBinding::new(secondary_keystroke!("q"), Quit, None),
        KeyBinding::new(secondary_keystroke!("w"), CloseWindow, viewer_context),
        KeyBinding::new(secondary_keystroke!("w"), CloseWindow, Some(PLOT_WINDOW_CONTEXT)),
        KeyBinding::new(secondary_keystroke!("o"), OpenFiles, viewer_context),
        KeyBinding::new(secondary_keystroke!("c"), CopyScreenshot, viewer_context),
        KeyBinding::new(secondary_keystroke!("e"), SaveScreenshot, viewer_context),
        KeyBinding::new("up", NextSlice, outside_text_input),
        KeyBinding::new("down", PreviousSlice, outside_text_input),
        KeyBinding::new("shift-up", NextSlicePage, outside_text_input),
        KeyBinding::new("shift-down", PreviousSlicePage, outside_text_input),
        KeyBinding::new("right", NextFrame, outside_text_input),
        KeyBinding::new("left", PreviousFrame, outside_text_input),
        KeyBinding::new("space", TogglePlayback, outside_text_input),
        KeyBinding::new("h", CenterCursor, outside_text_input),
        KeyBinding::new("a", FitWindow, outside_text_input),
        KeyBinding::new("m", NextColormap, outside_text_input),
        KeyBinding::new("l", ToggleLabelMode, outside_text_input),
        KeyBinding::new("s", ToggleCursorSynchronization, outside_text_input),
        KeyBinding::new("r", ToggleRadiological, outside_text_input),
        KeyBinding::new("c", ToggleCrosshair, outside_text_input),
        KeyBinding::new("i", CycleCornerText, outside_text_input),
        KeyBinding::new("p", ToggleMip, outside_text_input),
        KeyBinding::new("?", ShowHelp, outside_text_input),
        KeyBinding::new("g", ShowHistogramWindow, outside_text_input),
        KeyBinding::new("shift-g", ShowTimeSeriesWindow, outside_text_input),
        KeyBinding::new("t", ToggleTheme, outside_text_input),
        KeyBinding::new("o", ToggleOverlay, outside_text_input),
        KeyBinding::new("shift-l", ToggleOutline, outside_text_input),
        KeyBinding::new("v", ToggleLayerVisibility, outside_text_input),
        KeyBinding::new("j", NextLayer, outside_text_input),
        KeyBinding::new("k", PreviousLayer, outside_text_input),
        KeyBinding::new("1", LayoutOrthogonal, outside_text_input),
        KeyBinding::new("2", LayoutCompare, outside_text_input),
        KeyBinding::new("3", LayoutGrid, outside_text_input),
        KeyBinding::new("4", LayoutSingle, outside_text_input),
        KeyBinding::new("5", LayoutLightbox, outside_text_input),
        KeyBinding::new(secondary_keystroke!("backspace"), CloseLayer, viewer_context),
        KeyBinding::new(secondary_keystroke!("r"), ReloadLayer, viewer_context),
        KeyBinding::new("alt-k", MoveLayerUp, viewer_context),
        KeyBinding::new("alt-j", MoveLayerDown, viewer_context),
        KeyBinding::new("=", ZoomIn, outside_text_input),
        KeyBinding::new("-", ZoomOut, outside_text_input),
        KeyBinding::new("0", ResetZoom, outside_text_input),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &ToggleTheme, cx| crate::theme::toggle_theme_mode(cx));
}

pub fn help_entries() -> Vec<(String, &'static str)> {
    vec![
        ("Click+drag".into(), "Move cursor"),
        ("Right+drag".into(), "Layer Window/level"),
        ("Scroll".into(), "Change slice"),
        (format!("{SECONDARY_MODIFIER}scroll"), "Zoom at the mouse"),
        (
            format!("Middle+drag/{ALT_MODIFIER}drag"),
            "Pan a zoomed view",
        ),
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
        (format!("{ALT_MODIFIER}J/K"), "Move layer down/up"),
        (format!("{SECONDARY_MODIFIER}R"), "Reload layer from disk"),
        (format!("{SECONDARY_MODIFIER}\u{232b}"), "Close layer"),
        ("Drag layer".into(), "Reorder layers"),
        (format!("{SECONDARY_MODIFIER}O"), "Open volumes"),
        (
            format!("{SECONDARY_MODIFIER}C/{SECONDARY_MODIFIER}E"),
            "Copy/save the view as an image",
        ),
        (
            format!("{SECONDARY_MODIFIER}W/{SECONDARY_MODIFIER}Q"),
            "Close window/quit",
        ),
    ]
}
