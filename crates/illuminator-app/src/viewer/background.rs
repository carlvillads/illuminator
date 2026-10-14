//! Viewer background tasks.

use super::*;

impl Viewer {
    /// Starts a background render for each panel whose image is out of date since last draw
    pub(super) fn schedule_renders(&mut self, window: &Window, cx: &mut Context<Self>) {
        for view in self.views(window) {
            // overlaid layers are all drawn on the base layers plane
            for index in view.layers.clone() {
                let Some(render) = self.render_needed(&view, index) else {
                    continue;
                };
                spawn_render(render, window, cx);
                self.layers[index].panel_mut(view.slot).rendering = true;
            }
        }
    }

    /// What to render for layer `index` in `view`. None if its image is up to date
    fn render_needed(&self, view: &View, index: usize) -> Option<RenderRequest> {
        let (width, height) = view.image_pixels;
        let layer = &self.layers[index];
        let panel = layer.panel(view.slot);
        let rendering = panel.is_some_and(|panel| panel.rendering);
        if width == 0 || height == 0 || !layer.visible || rendering {
            return None;
        }
        let plane = view.visible.apply(view.plane);
        let job = layer.display.job(plane, width, height, view.mip);
        let key = ImageKey {
            sample: job.key.clone(),
            coloring: layer.display.coloring(),
        };
        if panel
            .and_then(|panel| panel.image.as_ref())
            .is_some_and(|image| image.key == key)
        {
            return None;
        }
        let reuse = panel
            .and_then(|panel| panel.samples.as_ref())
            .filter(|cached| cached.key == key.sample)
            .map(|cached| Arc::clone(&cached.samples));
        Some(RenderRequest {
            load_id: layer.load_id,
            slot: view.slot,
            job,
            key,
            reuse,
        })
    }

    /// Counts the selected layers values in the background once per frame
    pub(super) fn schedule_histogram(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected() else {
            return;
        };
        let layer = &mut self.layers[index];
        let frame = layer.display.frame();
        // counting all voxels of every frame would slow a movie down; it counts when it stops
        if self.time_playing.is_some()
            || layer.display.label_mode()
            || layer.histogram_pending
            || layer.histogram_frame == Some(frame)
        {
            return;
        }
        layer.histogram_pending = true;
        let volume = layer.display.shared_volume();
        let volume_id = volume.id();
        let load_id = layer.load_id;
        cx.spawn(async move |viewer, cx| {
            let histogram = cx
                .background_spawn(async move {
                    let progress = illuminator_core::progress::Progress::default();
                    Histogram::of(&volume, frame, HISTOGRAM_BINS, &progress)
                })
                .await;
            let _ = viewer.update(cx, |viewer, cx| {
                let Some(layer) = viewer.layer_by_id(load_id) else {
                    return;
                };
                layer.histogram_pending = false;
                if layer.display.volume().id() != volume_id {
                    return; // the layer was reloaded in the meantime
                }
                layer.histogram_frame = Some(frame);
                match histogram {
                    Ok(histogram) => layer.histogram = histogram,
                    Err(error) => {
                        layer.histogram = None;
                        eprintln!("Illuminator: Histogram: {error}");
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn schedule_curve(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected() else {
            return;
        };
        let layer = &mut self.layers[index];
        let volume = layer.display.shared_volume();
        if volume.meta().timepoints() < 2 || layer.curve_pending {
            return;
        }
        let Some((voxel, _)) = layer.display.cursor_sample() else {
            layer.curve = None;
            return;
        };
        if layer
            .curve
            .as_ref()
            .is_some_and(|curve| curve.volume == volume.id() && curve.voxel == voxel)
        {
            return;
        }
        layer.curve_pending = true;
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
                    layer.curve_pending = false;
                    layer.curve = curve;
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
    reuse: Option<Arc<SampledSlice>>,
}

/// Samples and colors a panels image
fn spawn_render(render: RenderRequest, window: &Window, cx: &mut Context<Viewer>) {
    let RenderRequest {
        load_id,
        slot,
        job,
        key,
        reuse,
    } = render;
    cx.spawn_in(window, async move |viewer, cx| {
        let coloring = key.coloring;
        let (samples, image) = cx
            .background_spawn(async move {
                let samples = reuse.unwrap_or_else(|| Arc::new(job.sample()));
                let image = to_render_image(samples.colorize(coloring));
                (samples, image)
            })
            .await;
        let _ = viewer.update_in(cx, |viewer, window, cx| {
            let Some(layer) = viewer.layer_by_id(load_id) else {
                return;
            };
            let panel = layer.panel_mut(slot);
            panel.rendering = false;
            panel.samples = Some(PanelSamples {
                key: key.sample.clone(),
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
