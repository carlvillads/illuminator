//! Viewer background tasks.

use gpui_kit::{AppContext, Context, RenderImage, Window};
use illuminator_core::display::SliceJob;
use illuminator_core::histogram::Histogram;
use illuminator_core::slicer::{SampledSlice, SliceImage};
use image::{Frame, RgbaImage};
use std::sync::Arc;

use super::{ImageKey, PanelImage, PanelSamples, TimeCurve, View, Viewer};

const SIDEBAR_HISTOGRAM_BINS: usize = 128;

impl Viewer {
    /// Starts a background render for each panel whose image is out of date since last draw
    pub(super) fn schedule_renders(&mut self, window: &Window, cx: &mut Context<Self>) {
        for view in self.views(window) {
            // overlaid layers are all drawn on the base layers plane
            for index in view.layers.clone() {
                let Some(render) = self.pending_render_request(&view, index) else {
                    continue;
                };
                spawn_render(render, window, cx);
                self.layers[index].panel_mut(view.slot).render_pending = true;
            }
        }
    }

    /// What to render for layer `index` in `view`. None if its image is up to date
    fn pending_render_request(&self, view: &View, index: usize) -> Option<RenderRequest> {
        let (width, height) = view.image_pixels;
        let layer = &self.layers[index];
        let panel = layer.panel(view.slot);
        let rendering = panel.is_some_and(|panel| panel.render_pending);
        if width == 0 || height == 0 || !layer.visible || rendering {
            return None;
        }
        let plane = view.visible_region.apply(view.plane);
        let job = layer.display.slice_job(plane, width, height, view.mip);
        let key = ImageKey {
            sample_key: job.key.clone(),
            coloring: layer.display.coloring(),
        };
        if panel
            .and_then(|panel| panel.image.as_ref())
            .is_some_and(|image| image.key == key)
        {
            return None;
        }
        let reusable_samples = panel
            .and_then(|panel| panel.samples.as_ref())
            .filter(|cached| cached.key == key.sample_key)
            .map(|cached| Arc::clone(&cached.samples));
        Some(RenderRequest {
            load_id: layer.load_id,
            slot: view.slot,
            job,
            key,
            reusable_samples,
        })
    }

    /// Counts the selected layers values in the background once per frame
    pub(super) fn schedule_histogram(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected_layer() else {
            return;
        };
        let layer = &mut self.layers[index];
        let key = layer.histogram_key();
        // counting all voxels of every frame would slow a movie down; it counts when it stops
        if self.playback_task.is_some()
            || layer.display.label_mode()
            || !layer.histogram.needs_count(key)
        {
            return;
        }
        layer.histogram.pending = Some(key);
        let volume = layer.display.shared_volume();
        let load_id = layer.load_id;
        cx.spawn(async move |viewer, cx| {
            let histogram = cx
                .background_spawn(async move {
                    let progress = illuminator_core::progress::Progress::default();
                    Histogram::of_frame(&volume, key.frame, SIDEBAR_HISTOGRAM_BINS, &progress)
                })
                .await;
            let _ = viewer.update(cx, |viewer, cx| {
                let Some(layer) = viewer.layer_by_id(load_id) else {
                    return;
                };
                let current = layer.histogram_key();
                let histogram = histogram.map_err(|error| {
                    eprintln!("Illuminator: Histogram: {error}");
                    error.to_string()
                });
                if layer.histogram.complete(key, current, histogram) {
                    // a discarded frame/scale result must still wake scheduler
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn schedule_time_curve(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected_layer() else {
            return;
        };
        let layer = &mut self.layers[index];
        let volume = layer.display.shared_volume();
        if volume.metadata().timepoints() < 2 || layer.time_curve_pending {
            return;
        }
        let Some((voxel, _)) = layer.display.value_at_cursor() else {
            layer.time_curve = None;
            return;
        };
        if layer
            .time_curve
            .as_ref()
            .is_some_and(|curve| curve.volume == volume.id() && curve.voxel == voxel)
        {
            return;
        }
        layer.time_curve_pending = true;
        let load_id = layer.load_id;
        cx.spawn(async move |viewer, cx| {
            let curve = cx
                .background_spawn(async move {
                    let values = volume.time_curve(voxel)?;
                    Some(TimeCurve {
                        volume: volume.id(),
                        voxel,
                        values,
                    })
                })
                .await;
            let _ = viewer.update(cx, |viewer, cx| {
                if let Some(layer) = viewer.layer_by_id(load_id) {
                    layer.time_curve_pending = false;
                    layer.time_curve = curve;
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

/// One panel image to render
struct RenderRequest {
    load_id: u64,
    slot: usize,
    job: SliceJob,
    key: ImageKey,
    /// The samples of the last render when only the coloring changed
    reusable_samples: Option<Arc<SampledSlice>>,
}

/// Samples and colors a panels image
fn spawn_render(render: RenderRequest, window: &Window, cx: &mut Context<Viewer>) {
    let RenderRequest {
        load_id,
        slot,
        job,
        key,
        reusable_samples,
    } = render;
    cx.spawn_in(window, async move |viewer, cx| {
        let coloring = key.coloring;
        let (samples, image) = cx
            .background_spawn(async move {
                let samples = reusable_samples.unwrap_or_else(|| Arc::new(job.sample()));
                let image = to_render_image(samples.colorize(coloring));
                (samples, image)
            })
            .await;
        let _ = viewer.update_in(cx, |viewer, window, cx| {
            let Some(layer) = viewer.layer_by_id(load_id) else {
                return;
            };
            let panel = layer.panel_mut(slot);
            panel.render_pending = false;
            panel.samples = Some(PanelSamples {
                key: key.sample_key.clone(),
                samples,
            });
            if let Some(old) = panel.image.replace(PanelImage { key, image }) {
                cx.drop_image(old.image, Some(window));
            }
            cx.notify();
        });
    })
    .detach();
}

/// Slice to GPUI image (BGRA)
fn to_render_image(image: SliceImage) -> Arc<RenderImage> {
    let mut pixels = image.pixels;
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let buffer = RgbaImage::from_raw(image.width as u32, image.height as u32, pixels)
        .expect("a slice image has width * height * 4 btyes");
    Arc::new(RenderImage::new(vec![Frame::new(buffer)]))
}
