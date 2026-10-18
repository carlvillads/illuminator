//! Adding, closing, moving, and reloading layers. One layer's contrast and SUV.

use gpui_kit::component::WindowExt;
use gpui_kit::{AppContext, Bounds, Context, ParentElement, Pixels, Point, Window, px};
use illuminator_core::display::VolumeDisplay;
use illuminator_core::suv::SuvInputs;
use std::cmp::Ordering;

use crate::label_statistics::{LayerSnapshot, StatisticsView};
use crate::suv_form::SuvForm;

use super::{DragEdge, FrameHistogram, Layer, Layout, Panel, Preset, Suv, Viewer, WindowEdge};

impl Viewer {
    pub(super) fn reindex(&mut self, new_index: impl Fn(usize) -> Option<usize>) {
        let count = self.layers.len();
        if self.selected < count {
            // a removed selected layer passes the selection to the one before it
            self.selected = new_index(self.selected).unwrap_or(self.selected.saturating_sub(1));
        }
        if !self.layout.overlaid() && self.active_panel.0 < count {
            self.active_panel.0 = new_index(self.active_panel.0).unwrap_or(self.selected);
        }
        if let Layout::Single {
            axis,
            layer: Some(layer),
        } = self.layout
            && layer < count
        {
            self.layout = match new_index(layer) {
                Some(layer) => Layout::Single {
                    axis,
                    layer: Some(layer),
                },
                None => self.previous_layout,
            };
        }
    }

    pub(super) fn close_layer(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.layers.len() {
            return;
        }
        self.reindex(|other| match other.cmp(&index) {
            Ordering::Less => Some(other),
            Ordering::Equal => None,
            Ordering::Greater => Some(other - 1),
        });
        let layer = self.layers.remove(index);
        self.retire(layer.panels);
        self.update_frame_slider(cx);
        self.sync_from_first();
        cx.notify();
    }

    pub(super) fn move_layer(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let count = self.layers.len();
        if from >= count || to >= count || from == to {
            return;
        }
        self.reindex(|index| Some(moved(index, from, to)));
        let layer = self.layers.remove(from);
        self.layers.insert(to, layer);
        self.sync_from_first();
        cx.notify();
    }

    pub(super) fn reload_layer(&mut self, index: usize) {
        let Some(layer) = self.layers.get(index) else {
            return;
        };
        let path = layer.display.volume().path().to_path_buf();
        let load_id = self.loader.enqueue(path);
        self.reloads.insert(load_id, layer.load_id);
    }

    pub(super) fn add_layer(
        &mut self,
        load_id: u64,
        mut display: VolumeDisplay,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = display.set_frame(self.frame) {
            eprintln!("Illuminator: {}: {error}", display.name());
        }
        if let Some(colormap) = self.load_colormaps.remove(&load_id) {
            display.colormap = colormap;
        }
        if self.label_loads.remove(&load_id)
            && let Err(error) = display.set_label_mode(true)
        {
            eprintln!("Illuminator: {}: {error}", display.name());
        }
        display.clip_low = true;
        let at = self
            .layers
            .iter()
            .rposition(|layer| layer.load_id < load_id)
            .map_or(0, |index| index + 1);
        self.reindex(|index| Some(if index >= at { index + 1 } else { index }));
        let layer = Layer::new(load_id, display, cx);
        self.layers.insert(at, layer);
    }

    pub(super) fn replace_layer(&mut self, load_id: u64, mut display: VolumeDisplay) {
        let Some(layer) = self.layer_by_id(load_id) else {
            return;
        };
        let kept = display.keep_settings_of(&layer.display);
        let name = display.name();
        layer.display = display;
        layer.histogram = FrameHistogram::default();
        let panels = std::mem::take(&mut layer.panels);
        self.retire(panels);
        if let Err(error) = kept {
            eprintln!("Illuminator: {name}: {error}");
        }
    }

    pub(super) fn retire(&mut self, panels: Vec<Panel>) {
        self.retired.extend(
            panels
                .into_iter()
                .filter_map(|panel| panel.image)
                .map(|image| image.image),
        );
    }

    pub(super) fn auto_window(&mut self, index: usize, cx: &mut Context<Self>) {
        let layer = &mut self.layers[index];
        let counted = layer
            .current_histogram()
            .filter(|histogram| histogram.max > histogram.min);
        match counted {
            Some(histogram) => {
                let (low, high) = (histogram.min, histogram.max);
                layer.display.set_window_limits(low, high);
                cx.notify();
            }
            None => self.change(index, VolumeDisplay::auto_window, cx),
        }
    }

    /// Moves one window edge to the value under the mouse in the histogram
    pub(super) fn drag_edge(
        &mut self,
        drag: DragEdge,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(layer) = self.layer_by_id(drag.load_id) else {
            return;
        };
        let Some(histogram) = layer.current_histogram() else {
            return;
        };
        if bounds.size.width <= px(0.0) {
            return;
        }
        let fraction =
            f64::from(f32::from(position.x - bounds.left()) / f32::from(bounds.size.width));
        let value = histogram.min + fraction * (histogram.max - histogram.min);
        self.move_window_edge(drag, value, cx);
    }

    /// Moves the window of a layer to `value`.
    pub(super) fn move_window_edge(&mut self, drag: DragEdge, value: f64, cx: &mut Context<Self>) {
        let Some(layer) = self.layer_by_id(drag.load_id) else {
            return;
        };
        let (low, high) = layer.display.window_limits();
        match drag.edge {
            WindowEdge::Low => layer.display.set_window_limits(value, high),
            WindowEdge::High => layer.display.set_window_limits(low, value),
        }
        cx.notify();
    }

    pub(super) fn apply_preset(&mut self, load_id: u64, preset: Preset, cx: &mut Context<Self>) {
        let Some(layer) = self.layer_by_id(load_id) else {
            return;
        };
        let limits = match preset {
            Preset::Limits(low, high) => Some((low, high)),
            Preset::Percentiles(low, high) => layer
                .current_histogram()
                .map(|histogram| (histogram.percentile(low), histogram.percentile(high))),
        };
        if let Some((low, high)) = limits {
            layer.display.set_window_limits(low, high);
            cx.notify();
        }
    }

    pub(super) fn set_suv(&mut self, index: usize, on: bool, cx: &mut Context<Self>) {
        let layer = &mut self.layers[index];
        let scale = match (on, layer.suv) {
            (false, _) => 1.0,
            (true, Some(suv)) => match suv.inputs.factor() {
                Ok(factor) => factor,
                Err(error) => {
                    eprintln!("Illuminator: {error}");
                    return;
                }
            },
            (true, None) => return,
        };
        if let Err(error) = layer.display.set_scale(scale) {
            eprintln!("Illuminator: {}: {error}", layer.display.name());
        }
        cx.notify();
    }

    pub fn set_suv_inputs(&mut self, load_id: u64, inputs: SuvInputs, cx: &mut Context<Self>) {
        if let Some(index) = self
            .layers
            .iter()
            .position(|layer| layer.load_id == load_id)
        {
            self.layers[index].suv = Some(Suv {
                inputs,
                source: "manual entry",
            });
            self.set_suv(index, true, cx);
        }
    }

    pub(super) fn edit_suv(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let layer = &self.layers[index];
        let (load_id, known) = (layer.load_id, layer.suv.map(|suv| suv.inputs));
        let viewer = cx.entity().downgrade();
        let form = cx.new(|cx| SuvForm::new(load_id, known, viewer, window, cx));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("SUV (bw)")
                .width(px(420.0))
                .child(form.clone())
        });
    }

    pub(super) fn show_statistics(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let snapshot = |layer: &Layer| LayerSnapshot {
            name: layer.display.name().into(),
            volume: layer.display.shared_volume(),
            geometry: layer.display.geometry().clone(),
            frame: layer.display.frame(),
        };
        let labels = snapshot(&self.layers[index]);
        let candidates = self
            .layers
            .iter()
            .enumerate()
            .filter(|&(other, layer)| other != index && !layer.display.label_mode())
            .map(|(_, layer)| snapshot(layer))
            .collect();
        let view = cx.new(|cx| StatisticsView::new(labels, candidates, cx));
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title("Label statistics")
                .width(px(700.0))
                .child(view.clone())
        });
    }
}

/// Where the item at `index` ends up when the item at `from` is moved to `to`
fn moved(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        to
    } else if from < index && index <= to {
        index - 1
    } else if to <= index && index < from {
        index + 1
    } else {
        index
    }
}

#[cfg(test)]
mod tests {
    use super::moved;

    #[test]
    fn moving_shifts_the_items_between() {
        let after = |from, to| {
            (0..4)
                .map(|index| moved(index, from, to))
                .collect::<Vec<_>>()
        };
        assert_eq!(after(0, 2), [2, 0, 1, 3]);
        assert_eq!(after(2, 0), [1, 2, 0, 3]);
        assert_eq!(after(1, 1), [0, 1, 2, 3]);
    }
}
