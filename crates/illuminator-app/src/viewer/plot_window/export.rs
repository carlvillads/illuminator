//! Copying and saving the shown values as CSV.

use gpui_kit::{AppContext, ClipboardItem, Context, SharedString};
use illuminator_core::metadata::strip_extension;
use std::path::PathBuf;
use std::time::Duration;

use super::{Plot, PlotLayer, PlotWindow, ViewerSnapshot};

const NOTICE_DURATION: Duration = Duration::from_secs(4);

impl<P: Plot> PlotWindow<P> {
    pub(super) fn can_export(&self, snapshot: &ViewerSnapshot<P::Key>) -> bool {
        self.measurements
            .is_export_ready(&snapshot.layers, &self.hidden)
    }

    /// The current values of the unhidden layers, in layer order
    fn exportable_values<'a>(
        &'a self,
        snapshot: &'a ViewerSnapshot<P::Key>,
    ) -> Vec<(&'a PlotLayer<P::Key>, &'a P::Value)> {
        self.legend_visible_layers(snapshot)
            .filter_map(|layer| Some((layer, self.measurements.current(layer)?.value.as_ref()?)))
            .collect()
    }

    fn csv(&self, snapshot: &ViewerSnapshot<P::Key>) -> Option<String> {
        self.can_export(snapshot)
            .then(|| P::csv(&self.exportable_values(snapshot)))
    }

    pub(super) fn copy_csv(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot(cx) else {
            return;
        };
        if let Some(csv) = self.csv(&snapshot) {
            cx.write_to_clipboard(ClipboardItem::new_string(csv));
            self.show_notice("Copied.".into(), cx);
        }
    }

    fn show_notice(&mut self, notice: SharedString, cx: &mut Context<Self>) {
        self.notice = Some(notice);
        // replacing the timer drops the one of an earlier notice
        self.notice_timer = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(NOTICE_DURATION).await;
            let _ = this.update(cx, |this, cx| {
                this.notice = None;
                cx.notify();
            });
        });
        cx.notify();
    }

    /// Saves next to the selected layer's file or the first shown
    pub(super) fn save_csv(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot(cx) else {
            return;
        };
        let Some(csv) = self.csv(&snapshot) else {
            return;
        };
        let exported = self.exportable_values(&snapshot);
        let Some((layer, _)) = exported
            .iter()
            .find(|(layer, _)| layer.selected)
            .or(exported.first())
        else {
            return;
        };
        let directory = layer
            .volume
            .path()
            .parent()
            .map(PathBuf::from)
            .unwrap_or_default();
        let name = format!(
            "{}_{}.csv",
            strip_extension(&layer.name),
            P::STRINGS.csv_name
        );
        let chosen = cx.prompt_for_new_path(&directory, Some(&name));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = chosen.await else {
                return; // cancelled
            };
            let file = path.clone();
            let saved = cx
                .background_spawn(async move { std::fs::write(&file, csv) })
                .await;
            let notice = match saved {
                Ok(()) => format!("Saved to {}", path.display()),
                Err(error) => {
                    eprintln!("Illuminator: saving {}: {error}", path.display());
                    format!("Cannot save: {error}")
                }
            };
            let _ = this.update(cx, |this, cx| this.show_notice(notice.into(), cx));
        })
        .detach();
    }
}
