//! Keyboard shortcut handlers.

use gpui_kit::component::{ActiveTheme, WindowExt, h_flex, v_flex};
use gpui_kit::{Context, ParentElement, Styled, Window, div, px};

use crate::shortcuts::{
    self, AutoContrast, CenterCursor, CloseLayer, CloseWindow, LayoutCompare, LayoutGrid,
    LayoutLightbox, LayoutOrtho, LayoutSingle, MoveLayerDown, MoveLayerUp, NextColormap, NextFrame,
    NextLayer, NextSlice, NextSlices, OpenFiles, PreviousFrame, PreviousLayer, PreviousSlice,
    PreviousSlices, ReloadLayer, ShowHelp, ToggleCrosshair, ToggleInfo, ToggleLayer, ToggleMip,
    ToggleOutline, ToggleOverlay, ToggleRadiological, ToggleSegmentation, ToggleSync, ZoomIn,
    ZoomOut, ZoomReset,
};

use super::{Layout, Viewer};

impl Viewer {
    pub(super) fn close_window(
        &mut self,
        _: &CloseWindow,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.remove_window();
    }

    pub(super) fn open_files_action(
        &mut self,
        _: &OpenFiles,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_files(cx);
    }

    pub(super) fn next_slice(&mut self, _: &NextSlice, _: &mut Window, cx: &mut Context<Self>) {
        self.step_active_slice(1.0, cx);
    }

    pub(super) fn previous_slice(
        &mut self,
        _: &PreviousSlice,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_active_slice(-1.0, cx);
    }

    pub(super) fn next_slices(&mut self, _: &NextSlices, _: &mut Window, cx: &mut Context<Self>) {
        self.step_active_slice(self.page_steps(), cx);
    }

    pub(super) fn previous_slices(
        &mut self,
        _: &PreviousSlices,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_active_slice(-self.page_steps(), cx);
    }

    pub(super) fn next_frame(
        &mut self,
        _: &NextFrame,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_frame(1, window, cx);
    }

    pub(super) fn previous_frame(
        &mut self,
        _: &PreviousFrame,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_frame(-1, window, cx);
    }

    pub(super) fn center_cursor(
        &mut self,
        _: &CenterCursor,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.selected() {
            self.change(
                index,
                |display| {
                    display.reset_cursor();
                    Ok(())
                },
                cx,
            );
        }
    }

    pub(super) fn auto_contrast(
        &mut self,
        _: &AutoContrast,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.selected() {
            self.auto_window(index, cx);
        }
    }

    pub(super) fn next_colormap(
        &mut self,
        _: &NextColormap,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.selected() {
            let display = &mut self.layers[index].display;
            display.colormap = display.colormap.next();
            cx.notify();
        }
    }

    pub(super) fn toggle_segmentation(
        &mut self,
        _: &ToggleSegmentation,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.selected() {
            let enabled = !self.layers[index].display.label_mode();
            self.change(index, |display| display.set_label_mode(enabled), cx);
        }
    }

    pub(super) fn close_selected(
        &mut self,
        _: &CloseLayer,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.selected() {
            self.close_layer(index, cx);
        }
    }

    pub(super) fn reload_selected(
        &mut self,
        _: &ReloadLayer,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        if let Some(index) = self.selected() {
            self.reload_layer(index);
        }
    }

    pub(super) fn move_selected_up(
        &mut self,
        _: &MoveLayerUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.selected()
            && index > 0
        {
            self.move_layer(index, index - 1, cx);
        }
    }

    pub(super) fn move_selected_down(
        &mut self,
        _: &MoveLayerDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.selected() {
            self.move_layer(index, index + 1, cx);
        }
    }

    pub(super) fn toggle_outline(
        &mut self,
        _: &ToggleOutline,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.selected() {
            let display = &mut self.layers[index].display;
            display.outline = !display.outline;
            cx.notify();
        }
    }

    pub(super) fn toggle_overlay(
        &mut self,
        _: &ToggleOverlay,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let layout = if self.layout.overlaid() {
            Layout::Compare
        } else {
            Layout::Ortho
        };
        self.set_layout(layout, cx);
    }

    pub(super) fn layout_ortho(&mut self, _: &LayoutOrtho, _: &mut Window, cx: &mut Context<Self>) {
        self.set_layout(Layout::Ortho, cx);
    }

    pub(super) fn layout_compare(
        &mut self,
        _: &LayoutCompare,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_layout(Layout::Compare, cx);
    }

    pub(super) fn layout_grid(&mut self, _: &LayoutGrid, _: &mut Window, cx: &mut Context<Self>) {
        let axis = self.layout.axis().unwrap_or(self.active_panel.1);
        self.set_layout(Layout::Grid(axis), cx);
    }

    pub(super) fn layout_single(
        &mut self,
        _: &LayoutSingle,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let axis = self.layout.axis().unwrap_or(self.active_panel.1);
        self.set_layout(Layout::Single { axis, layer: None }, cx);
    }

    pub(super) fn layout_lightbox(
        &mut self,
        _: &LayoutLightbox,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let axis = self.layout.axis().unwrap_or(self.active_panel.1);
        self.set_layout(Layout::Lightbox(axis), cx);
    }

    pub(super) fn toggle_layer(&mut self, _: &ToggleLayer, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.selected() {
            let layer = &mut self.layers[index];
            layer.visible = !layer.visible;
            cx.notify();
        }
    }

    pub(super) fn next_layer(&mut self, _: &NextLayer, _: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.is_empty() {
            self.select((self.selected + 1) % self.layers.len(), cx);
        }
    }

    pub(super) fn previous_layer(
        &mut self,
        _: &PreviousLayer,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.layers.is_empty() {
            let count = self.layers.len();
            self.select((self.selected + count - 1) % count, cx);
        }
    }

    pub(super) fn toggle_sync(&mut self, _: &ToggleSync, _: &mut Window, cx: &mut Context<Self>) {
        self.sync = !self.sync;
        self.sync_from_first();
        cx.notify();
    }

    pub(super) fn toggle_radiological(
        &mut self,
        _: &ToggleRadiological,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.radiological = !self.radiological;
        self.zoom = Default::default();
        cx.notify();
    }

    pub(super) fn toggle_info(&mut self, _: &ToggleInfo, _: &mut Window, cx: &mut Context<Self>) {
        self.info = self.info.next();
        cx.notify();
    }

    pub(super) fn toggle_mip(&mut self, _: &ToggleMip, _: &mut Window, cx: &mut Context<Self>) {
        self.show_mip = !self.show_mip;
        cx.notify();
    }

    pub(super) fn toggle_crosshair(
        &mut self,
        _: &ToggleCrosshair,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_crosshair = !self.show_crosshair;
        cx.notify();
    }

    pub(super) fn zoom_in(&mut self, _: &ZoomIn, window: &mut Window, cx: &mut Context<Self>) {
        self.zoom_active(2.0_f64.sqrt(), window, cx);
    }

    pub(super) fn zoom_out(&mut self, _: &ZoomOut, window: &mut Window, cx: &mut Context<Self>) {
        self.zoom_active(0.5_f64.sqrt(), window, cx);
    }

    pub(super) fn reset_zoom(&mut self, _: &ZoomReset, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom = Default::default();
        cx.notify();
    }

    fn step_active_slice(&mut self, steps: f64, cx: &mut Context<Self>) {
        let Some((index, axis)) = self.active() else {
            return;
        };
        let steps = steps * self.slice_stride();
        if self.layers[index].display.step_slice(axis, steps) {
            self.sync_from(index);
            cx.notify();
        }
    }

    /// Zooms active view plane at the middle
    fn zoom_active(&mut self, factor: f64, window: &Window, cx: &mut Context<Self>) {
        let Some(active) = self.active() else {
            return;
        };
        let views = self.views(window);
        let Some(view) = views.iter().find(|view| (view.base, view.axis) == active) else {
            return;
        };
        let zoom = &mut self.zoom[view.axis.index()];
        *zoom = zoom.zoom_at(factor, (0.5, 0.5), view.fit);
        cx.notify();
    }

    pub(super) fn show_help(&mut self, _: &ShowHelp, window: &mut Window, cx: &mut Context<Self>) {
        let cache = format!(
            "Decompressed volumes are cached in {} >= {:.1} GiB",
            self.cache.directory().display(),
            self.cache.budget() as f64 / (1u64 << 30) as f64
        );
        window.open_dialog(cx, move |dialog, _, cx| {
            let theme = cx.theme();
            let (muted, border) = (theme.muted_foreground, theme.border);
            let title = format!("Illuminator {} shortcuts", env!("CARGO_PKG_VERSION"));
            dialog.title(title).width(px(440.0)).child(
                v_flex()
                    .gap_1()
                    .text_sm()
                    .children(shortcuts::help().into_iter().map(|(keys, action)| {
                        h_flex()
                            .gap_3()
                            .child(
                                div().w(px(120.0)).child(
                                    div()
                                        .px_1()
                                        .rounded_sm()
                                        .border_1()
                                        .border_color(border)
                                        .text_xs()
                                        .child(keys),
                                ),
                            )
                            .child(action)
                    }))
                    .child(
                        div()
                            .pt_3()
                            .text_xs()
                            .text_color(muted)
                            .child(cache.clone()),
                    ),
            )
        });
    }
}
