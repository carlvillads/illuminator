//! Keyboard shortcut handlers.

use gpui_kit::component::{ActiveTheme, WindowExt, h_flex, v_flex};
use gpui_kit::{Context, ParentElement, Styled, Window, div, px};

use crate::shortcuts::{
    self, CenterCursor, CloseLayer, CloseWindow, CycleCornerText, FitWindow, LayoutCompare,
    LayoutGrid, LayoutLightbox, LayoutOrthogonal, LayoutSingle, MoveLayerDown, MoveLayerUp,
    NextColormap, NextFrame, NextLayer, NextSlice, NextSlicePage, OpenFiles, PreviousFrame,
    PreviousLayer, PreviousSlice, PreviousSlicePage, ReloadLayer, ResetZoom, ShowHelp,
    ToggleCrosshair, ToggleCursorSynchronization, ToggleLabelMode, ToggleLayerVisibility,
    ToggleMip, ToggleOutline, ToggleOverlay, ToggleRadiological, ZoomIn, ZoomOut,
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

    pub(super) fn next_slice_page(
        &mut self,
        _: &NextSlicePage,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_active_slice(self.slice_page_length(), cx);
    }

    pub(super) fn previous_slice_page(
        &mut self,
        _: &PreviousSlicePage,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_active_slice(-self.slice_page_length(), cx);
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
        if let Some(index) = self.selected_layer() {
            self.modify_display(
                index,
                |display| {
                    display.reset_cursor();
                    Ok(())
                },
                cx,
            );
        }
    }

    pub(super) fn fit_window_action(
        &mut self,
        _: &FitWindow,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.selected_layer() {
            self.fit_window(index, cx);
        }
    }

    pub(super) fn next_colormap(
        &mut self,
        _: &NextColormap,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_selected(cx, |layer| {
            layer.display.colormap = layer.display.colormap.next();
        });
    }

    pub(super) fn toggle_label_mode(
        &mut self,
        _: &ToggleLabelMode,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.selected_layer() {
            let enabled = !self.layers[index].display.label_mode();
            self.modify_display(index, |display| display.set_label_mode(enabled), cx);
        }
    }

    pub(super) fn close_selected(
        &mut self,
        _: &CloseLayer,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.selected_layer() {
            self.close_layer(index, cx);
        }
    }

    pub(super) fn reload_selected(
        &mut self,
        _: &ReloadLayer,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        if let Some(index) = self.selected_layer() {
            self.reload_layer(index);
        }
    }

    pub(super) fn move_selected_up(
        &mut self,
        _: &MoveLayerUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.selected_layer()
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
        if let Some(index) = self.selected_layer() {
            self.move_layer(index, index + 1, cx);
        }
    }

    pub(super) fn toggle_outline(
        &mut self,
        _: &ToggleOutline,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_selected(cx, |layer| layer.display.outline = !layer.display.outline);
    }

    pub(super) fn toggle_overlay(
        &mut self,
        _: &ToggleOverlay,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let layout = if self.layout.is_overlaid() {
            Layout::Compare
        } else {
            Layout::Orthogonal
        };
        self.set_layout(layout, cx);
    }

    pub(super) fn layout_orthogonal(
        &mut self,
        _: &LayoutOrthogonal,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_layout(Layout::Orthogonal, cx);
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
        self.set_layout(Layout::Grid(self.current_axis()), cx);
    }

    pub(super) fn layout_single(
        &mut self,
        _: &LayoutSingle,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let axis = self.current_axis();
        self.set_layout(Layout::Single { axis, layer: None }, cx);
    }

    pub(super) fn layout_lightbox(
        &mut self,
        _: &LayoutLightbox,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_layout(Layout::Lightbox(self.current_axis()), cx);
    }

    pub(super) fn toggle_layer_visibility(
        &mut self,
        _: &ToggleLayerVisibility,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_selected(cx, |layer| layer.visible = !layer.visible);
    }

    pub(super) fn next_layer(&mut self, _: &NextLayer, _: &mut Window, cx: &mut Context<Self>) {
        if !self.layers.is_empty() {
            self.select((self.selected_layer + 1) % self.layers.len(), cx);
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
            self.select((self.selected_layer + count - 1) % count, cx);
        }
    }

    pub(super) fn toggle_curson_synchronization(
        &mut self,
        _: &ToggleCursorSynchronization,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cursor_synchronization = !self.cursor_synchronization;
        self.synchronize_cursors_from_first();
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

    pub(super) fn cycle_corner_text(
        &mut self,
        _: &CycleCornerText,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.corner_text = self.corner_text.next();
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

    pub(super) fn reset_zoom(&mut self, _: &ResetZoom, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom = Default::default();
        cx.notify();
    }

    fn step_active_slice(&mut self, steps: f64, cx: &mut Context<Self>) {
        let Some((index, axis)) = self.active_panel() else {
            return;
        };
        let steps = steps * self.slice_stride();
        if self.layers[index].display.step_slice(axis, steps) {
            self.synchronize_cursors_from(index);
            cx.notify();
        }
    }

    /// Zooms active view plane at the middle
    fn zoom_active(&mut self, factor: f64, window: &Window, cx: &mut Context<Self>) {
        let Some(active) = self.active_panel() else {
            return;
        };
        let views = self.views(window);
        let Some(view) = views
            .iter()
            .find(|view| (view.base_layer, view.axis) == active)
        else {
            return;
        };
        let zoom = &mut self.zoom[view.axis.index()];
        *zoom = zoom.zoom_at(factor, (0.5, 0.5), view.fit_scale);
        cx.notify();
    }

    pub(super) fn show_help(&mut self, _: &ShowHelp, window: &mut Window, cx: &mut Context<Self>) {
        let cache = format!(
            "Decompressed volumes are cached in {} >= {:.1} GiB",
            self.cache.directory().display(),
            self.cache.capacity() as f64 / (1u64 << 30) as f64
        );
        window.open_dialog(cx, move |dialog, _, cx| {
            let theme = cx.theme();
            let (muted, border) = (theme.muted_foreground, theme.border);
            let title = format!("Illuminator {} shortcuts", env!("CARGO_PKG_VERSION"));
            dialog.title(title).width(px(440.0)).child(
                v_flex()
                    .gap_1()
                    .text_sm()
                    .children(shortcuts::help_entries().into_iter().map(|(keys, action)| {
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
