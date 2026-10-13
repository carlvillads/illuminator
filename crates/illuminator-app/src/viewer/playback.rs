//! 4D time series, frame slider, playback

use super::*;

impl Viewer {
    pub(super) fn step_frame(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        self.go_to_frame(self.frame.saturating_add_signed(delta), window, cx);
    }

    pub(super) fn play(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.frame_slider.is_none() {
            return;
        }
        let interval = Duration::from_secs_f64(1.0 / f64::from(self.time_fps));
        self.time_playing = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                let Ok((next, volumes)) =
                    this.update_in(cx, |viewer, window, _| viewer.next_movie_frame(window))
                else {
                    break; // viewer is gone here
                };
                let timer = cx.background_executor().timer(interval);
                let read_ahead = cx.background_spawn(async move {
                    for volume in volumes {
                        if let Err(error) = volume.read_ahead(next) {
                            eprintln!("Illuminator: reading frame {}: {error}", next + 1);
                        }
                    }
                });
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(5))
                        .await;
                    match this.update_in(cx, |viewer, window, _| viewer.frame_drawn(window)) {
                        Ok(true) => break,
                        Ok(false) => continue,
                        Err(_) => return, // viewer is gone here
                    }
                }
                futures::join!(timer, read_ahead);
                let stepped = this.update_in(cx, |viewer, window, cx| {
                    viewer.go_to_frame(next, window, cx);
                });
                if stepped.is_err() {
                    break;
                }
            }
        }));
    }

    pub(super) fn set_frames_per_second(
        &mut self,
        rate: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.time_fps = rate;
        if self.time_playing.is_some() {
            self.play(window, cx);
        }
        cx.notify();
    }

    pub(super) fn pick_frame(&mut self, x: Pixels, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.selected() else {
            return;
        };
        let bounds = self.time_curve_bounds.get();
        let frames = self.layers[index].display.volume().meta().timepoints();
        if bounds.size.width <= px(0.0) || frames < 2 {
            return;
        }
        let fraction = ((x - bounds.left()) / bounds.size.width).clamp(0.0, 1.0);
        let frame = (fraction * (frames - 1) as f32).round() as usize;
        self.go_to_frame(frame, window, cx);
    }

    pub(super) fn update_frame_slider(&mut self, cx: &mut Context<Self>) {
        let frames = self
            .layers
            .iter()
            .map(|column| column.display.volume().meta().timepoints())
            .max()
            .unwrap_or(1);
        let last_frame = frames - 1;
        if last_frame == 0 {
            // no 4d layers left
            self.frame_slider = None;
            self.time_playing = None;
            return;
        }
        if self
            .frame_slider
            .as_ref()
            .is_some_and(|s| s.last_frame == last_frame)
        {
            return;
        }
        let frame = self.frame.min(last_frame) as f32;
        let state = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(last_frame as f32)
                .step(1.0)
                .default_value(frame)
        });
        let changes = cx.subscribe(&state, |viewer, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(value) = event {
                viewer.set_frame(value.start().round() as usize, cx);
            }
        });
        self.frame_slider = Some(FrameSlider {
            state,
            last_frame,
            _changes: changes,
        });
    }

    fn go_to_frame(&mut self, frame: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(slider) = &self.frame_slider else {
            return;
        };
        let frame = frame.min(slider.last_frame);
        if frame != self.frame {
            let state = slider.state.clone();
            state.update(cx, |state, cx| state.set_value(frame as f32, window, cx));
            self.set_frame(frame, cx);
        }
    }

    fn next_movie_frame(&self, window: &Window) -> (usize, Vec<Arc<Volume>>) {
        let last = self.frame_slider.as_ref().map_or(0, |s| s.last_frame);
        let next = if self.frame >= last {
            0
        } else {
            self.frame + 1
        };
        let mut shown_sagittally: Vec<usize> = self
            .views(window)
            .iter()
            .filter(|view| view.axis == ViewPlane::Sagittal)
            .flat_map(|view| view.layers.clone())
            .collect();
        shown_sagittally.sort_unstable();
        shown_sagittally.dedup();
        let volumes = shown_sagittally
            .into_iter()
            .map(|index| &self.layers[index])
            .filter(|layer| layer.visible && layer.display.volume().meta().timepoints() > 1)
            .map(|layer| layer.display.shared_volume())
            .collect();
        (next, volumes)
    }

    fn frame_drawn(&self, window: &Window) -> bool {
        self.views(window).iter().all(|view| {
            view.layers.clone().all(|index| {
                let layer = &self.layers[index];
                !layer.visible
                    || layer.panel(view.slot).is_some_and(|panel| {
                        !panel.rendering
                            && panel.image.as_ref().is_some_and(|image| {
                                image.key.sample.frame == layer.display.frame()
                            })
                    })
            })
        })
    }

    fn set_frame(&mut self, frame: usize, cx: &mut Context<Self>) {
        self.frame = frame;
        for index in 0..self.layers.len() {
            if let Err(error) = self.layers[index].display.set_frame(frame) {
                eprintln!("Illuminator: layer {}: {error}", index + 1);
            }
        }
        cx.notify();
    }
}
