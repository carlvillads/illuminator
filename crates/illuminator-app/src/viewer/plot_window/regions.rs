//! Measuring inside the labels of a label map layer.

use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::{Context, IntoElement, SharedString, px};
use illuminator_core::geometry::VolumeGeometry;
use illuminator_core::histogram::LabelMask;
use illuminator_core::statistics::VolumeFrame;
use illuminator_core::volume::Volume;
use std::rc::Rc;
use std::sync::Arc;

use crate::ui::add_choice_items;

use super::super::Layer;
use super::{Plot, PlotWindow, ViewerSnapshot};

/// The labels of a label map layer to measure inside of or one of them
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LabelRegion {
    load_id: u64,
    label: Option<i64>,
}

/// What a label region is measured from
#[derive(Clone, PartialEq)]
pub(in crate::viewer) struct RegionKey {
    pub(in crate::viewer) volume: u64,
    pub(in crate::viewer) frame: usize,
    pub(in crate::viewer) geometry: VolumeGeometry,
    pub(in crate::viewer) label: Option<i64>,
}

impl RegionKey {
    /// The voxels of this region in its label map to measure
    pub(in crate::viewer) fn resolve(
        &self,
        label_maps: &[LabelMap],
    ) -> Option<ResolvedLabelRegion> {
        let map = label_maps
            .iter()
            .find(|map| map.volume.id() == self.volume)?;
        Some(ResolvedLabelRegion {
            volume: Arc::clone(&map.volume),
            geometry: self.geometry.clone(),
            frame: self.frame,
            label: self.label,
        })
    }
}

/// A label region together with the label map volume it is in
pub(in crate::viewer) struct ResolvedLabelRegion {
    volume: Arc<Volume>,
    geometry: VolumeGeometry,
    frame: usize,
    label: Option<i64>,
}

impl ResolvedLabelRegion {
    pub(in crate::viewer) fn mask(&self) -> LabelMask<'_> {
        LabelMask {
            label_volume: VolumeFrame {
                volume: &self.volume,
                geometry: &self.geometry,
                frame: self.frame,
            },
            label: self.label,
        }
    }
}

/// A label map layer to measure inside of
pub(in crate::viewer) struct LabelMap {
    load_id: u64,
    name: SharedString,
    labels: Vec<i64>,
    volume: Arc<Volume>,
    geometry: VolumeGeometry,
    frame: usize,
}

impl LabelMap {
    pub(super) fn of(layer: &Layer) -> Self {
        let display = &layer.display;
        Self {
            load_id: layer.load_id,
            name: display.name().into(),
            labels: display.labels().to_vec(),
            volume: display.shared_volume(),
            geometry: display.geometry().clone(),
            frame: display.frame(),
        }
    }
}

impl<P: Plot> PlotWindow<P> {
    /// The key of the chosen label region
    pub(super) fn region_key(&self, label_maps: &[LabelMap]) -> Option<RegionKey> {
        let region = self.region?;
        let map = label_maps
            .iter()
            .find(|map| map.load_id == region.load_id)?;
        Some(RegionKey {
            volume: map.volume.id(),
            frame: map.frame,
            geometry: map.geometry.clone(),
            label: region.label,
        })
    }

    /// Goes back to the plot's own region when the region's label map was closed or isn't a label map anymore
    pub(super) fn clear_closed_region(
        &mut self,
        snapshot: &ViewerSnapshot<P::Key>,
        cx: &mut Context<Self>,
    ) {
        if let Some(region) = self.region
            && !snapshot
                .label_maps
                .iter()
                .any(|map| map.load_id == region.load_id)
        {
            self.region = None;
            cx.notify();
        }
    }

    /// A menu button choosing between the plot's `own_regions` and the labels of the label maps.
    pub(in crate::viewer) fn region_button<T: Copy + 'static>(
        &self,
        caption: &str,
        own_region_label: String,
        own_regions: Vec<(String, T)>,
        on_choose: impl Fn(&mut P, T) + 'static,
        snapshot: &ViewerSnapshot<P::Key>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let maps: Vec<(SharedString, u64, Vec<i64>)> = snapshot
            .label_maps
            .iter()
            .map(|map| (map.name.clone(), map.load_id, map.labels.clone()))
            .collect();
        let chosen = match self.region {
            None => own_region_label,
            Some(region) => {
                let name = maps
                    .iter()
                    .find(|(_, load_id, _)| *load_id == region.load_id)
                    .map_or_else(SharedString::default, |(name, _, _)| name.clone());
                match region.label {
                    None => format!("{name}, all labels"),
                    Some(label) => format!("{name}, label {label}"),
                }
            }
        };
        let choose = Rc::new(on_choose);
        Button::new("region")
            .xsmall()
            .outline()
            .label(format!("{caption}: {chosen}"))
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                let choose = Rc::clone(&choose);
                let mut menu = add_choice_items(
                    menu.max_h(px(360.0)).scrollable(true),
                    &this,
                    own_regions.clone(),
                    move |this: &mut Self, region, _, cx| {
                        choose(&mut this.plot, region);
                        this.region = None;
                        cx.notify();
                    },
                );
                for (name, load_id, labels) in &maps {
                    let load_id = *load_id;
                    let all = std::iter::once(("All labels".to_string(), None));
                    let each = labels
                        .iter()
                        .map(|&label| (format!("Label {label}"), Some(label)));
                    let choices = all
                        .chain(each)
                        .map(|(text, label)| (text, LabelRegion { load_id, label }));
                    menu = add_choice_items(
                        menu.separator().label(name.clone()),
                        &this,
                        choices,
                        |this, region, _, cx| {
                            this.region = Some(region);
                            cx.notify();
                        },
                    );
                }
                menu
            })
    }
}
