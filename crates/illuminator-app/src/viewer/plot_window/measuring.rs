//! Measuring layers in the background.

use gpui_kit::{AppContext, Context};
use illuminator_core::Error;
use illuminator_core::progress::Progress;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{Plot, PlotLayer, PlotWindow, Plotted, Snapshot};

/// Measures one layer on a background thread.
pub(in crate::viewer) type Job<V> =
    Box<dyn FnOnce(&Progress) -> illuminator_core::Result<Option<V>> + Send>;

/// A layer's value and the key it was measured for.
pub(super) struct Measured<K, V> {
    pub(super) key: K,
    pub(super) value: Option<V>,
}

/// The values measured of each layer by load id, and the measurements still running.
pub(super) struct Measurements<K, V> {
    done: HashMap<u64, Measured<K, V>>,
    running: HashMap<u64, (K, Arc<AtomicBool>)>,
}

impl<K, V> Default for Measurements<K, V> {
    fn default() -> Self {
        Self {
            done: HashMap::new(),
            running: HashMap::new(),
        }
    }
}

impl<K: PartialEq, V> Measurements<K, V> {
    /// The value of `layer` if it was measured for the key the layer wants now
    pub(super) fn current(&self, layer: &PlotLayer<K>) -> Option<&Measured<K, V>> {
        self.done
            .get(&layer.load_id)
            .filter(|measured| layer.key.as_ref() == Some(&measured.key))
    }

    /// The last value measured of a layer, current or not
    fn latest(&self, load_id: u64) -> Option<&Measured<K, V>> {
        self.done.get(&load_id)
    }

    fn is_running(&self, load_id: u64) -> bool {
        self.running.contains_key(&load_id)
    }

    /// Forgets the layers that are gone, and stops the measurements that are out of date
    fn prune(&mut self, layers: &[PlotLayer<K>], wanted: &[&PlotLayer<K>]) {
        self.done
            .retain(|load_id, _| layers.iter().any(|layer| layer.load_id == *load_id));
        for (load_id, (key, cancelled)) in &self.running {
            if !wanted
                .iter()
                .any(|layer| layer.load_id == *load_id && layer.key.as_ref() == Some(key))
            {
                cancelled.store(true, Ordering::Relaxed);
            }
        }
    }

    /// Starts measuring a layer for `key`.
    fn start(&mut self, load_id: u64, key: K) -> Arc<AtomicBool> {
        let cancelled = Arc::new(AtomicBool::new(false));
        self.running.insert(load_id, (key, Arc::clone(&cancelled)));
        cancelled
    }

    fn stop(&mut self, load_id: u64) {
        self.running.remove(&load_id);
    }

    fn keep(&mut self, load_id: u64, key: K, value: Option<V>) {
        self.done.insert(load_id, Measured { key, value });
    }

    /// Whether every unhidden layer with something to measure has a current value
    pub(super) fn export_ready(&self, layers: &[PlotLayer<K>], hidden: &HashSet<u64>) -> bool {
        let mut has_values = false;
        for layer in layers
            .iter()
            .filter(|layer| layer.key.is_some() && !hidden.contains(&layer.load_id))
        {
            let Some(measured) = self.current(layer) else {
                return false;
            };
            has_values |= measured.value.is_some();
        }
        has_values
    }
}

impl<P: Plot> PlotWindow<P> {
    /// Measures the layers whose values are missing or out of date in the background
    pub(super) fn schedule(&mut self, snapshot: &Snapshot<P::Key>, cx: &mut Context<Self>) {
        // layers hidden in the legend are measured when they are shown again
        let wanted: Vec<&PlotLayer<P::Key>> = self
            .unhidden(snapshot)
            .filter(|layer| layer.key.is_some())
            .collect();
        self.measurements.prune(&snapshot.layers, &wanted);
        if self.waiting(snapshot) {
            return;
        }
        for layer in wanted {
            if self.measurements.current(layer).is_none()
                && !self.measurements.is_running(layer.load_id)
            {
                self.measure(layer, snapshot, cx);
            }
        }
    }

    fn measure(
        &mut self,
        layer: &PlotLayer<P::Key>,
        snapshot: &Snapshot<P::Key>,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = layer.key.clone() else {
            return;
        };
        let Some(job) = P::job(&key, Arc::clone(&layer.volume), &snapshot.label_maps) else {
            return;
        };
        let load_id = layer.load_id;
        let cancelled = self.measurements.start(load_id, key.clone());
        cx.spawn(async move |this, cx| {
            let value = cx
                .background_spawn(async move { job(&Progress::cancellable(cancelled)) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.measurements.stop(load_id);
                let wanted = this
                    .snapshot(cx)
                    .is_some_and(|snapshot| snapshot.wants(load_id, &key));
                match value {
                    Err(Error::Cancelled) => {}
                    value if wanted => {
                        let value = value.unwrap_or_else(|error| {
                            eprintln!("Illuminator: {}: {error}", P::TEXTS.title);
                            None
                        });
                        this.measurements.keep(load_id, key, value);
                    }
                    _ => {}
                }
                // Even obsolete work must wake rendering!
                cx.notify();
            });
        })
        .detach();
    }

    /// The values to plot of the unhidden layers in layer order
    pub(super) fn plotted<'a>(&'a self, snapshot: &'a Snapshot<P::Key>) -> Vec<Plotted<'a, P>> {
        self.unhidden(snapshot)
            .filter_map(|layer| {
                let key = layer.key.as_ref()?;
                let measured = self.measurements.latest(layer.load_id)?;
                let stale = measured.key != *key;
                if stale && !P::shows_stale(&measured.key, key) {
                    return None;
                }
                Some(Plotted {
                    layer,
                    value: measured.value.as_ref()?,
                    stale,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::PlotLayer;
    use super::super::test_util::loaded_display;
    use super::Measurements;
    use gpui_kit::rgb;
    use illuminator_core::display::VolumeDisplay;
    use std::collections::HashSet;

    fn layer(load_id: u64, key: Option<u32>, display: &VolumeDisplay) -> PlotLayer<u32> {
        PlotLayer {
            load_id,
            number: load_id as usize,
            name: "values".into(),
            color: rgb(0xffffff).into(),
            volume: display.shared_volume(),
            key,
            limits: display.window_limits(),
            selected: false,
        }
    }

    #[test]
    fn values_only_count_for_the_key_they_were_measured_for() {
        let (_directory, display) = loaded_display();
        let mut layer = layer(1, Some(1), &display);
        let mut measurements = Measurements::default();
        measurements.keep(1, 1, Some("values"));
        assert!(measurements.current(&layer).is_some());
        layer.key = Some(2);
        assert!(measurements.current(&layer).is_none());
        layer.key = None;
        assert!(measurements.current(&layer).is_none());
        layer.key = Some(1);
        layer.load_id = 2;
        assert!(measurements.current(&layer).is_none());
        layer.load_id = 1;
        measurements.keep(1, 2, None);
        assert!(measurements.current(&layer).is_none());
        layer.key = Some(2);
        assert!(measurements.current(&layer).unwrap().value.is_none());
    }

    #[test]
    fn export_waits_for_every_unhidden_layer_with_something_to_measure() {
        let (_directory, display) = loaded_display();
        let layers = [layer(1, Some(1), &display), layer(2, Some(1), &display)];
        let mut measurements = Measurements::default();
        measurements.keep(1, 1, Some("values"));
        let mut hidden = HashSet::new();
        assert!(!measurements.export_ready(&layers, &hidden));
        hidden.insert(2);
        assert!(measurements.export_ready(&layers, &hidden));
        hidden.clear();
        measurements.keep(2, 2, None);
        assert!(!measurements.export_ready(&layers, &hidden));
        measurements.keep(2, 2, Some("values"));
        assert!(!measurements.export_ready(&layers, &hidden));
        measurements.keep(2, 1, None);
        assert!(measurements.export_ready(&layers, &hidden));
        measurements.keep(1, 1, None);
        assert!(!measurements.export_ready(&layers, &hidden));
        hidden.extend([1, 2]);
        assert!(!measurements.export_ready(&layers, &hidden));
        assert!(!Measurements::<u32, &str>::default().export_ready(&[], &HashSet::new()));
        let layers = [layer(1, Some(1), &display), layer(2, None, &display)];
        measurements.keep(1, 1, Some("values"));
        assert!(measurements.export_ready(&layers, &HashSet::new()));
    }
}
