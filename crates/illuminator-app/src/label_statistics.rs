//! The label statistics window.
//!
//! Shows the size and values of each label, which can be copied or saved as CSV.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, h_flex, v_flex};
use gpui_kit::*;
use illuminator_core::geometry::VolumeGeometry;
use illuminator_core::labels::label_color;
use illuminator_core::meta::strip_extension;
use illuminator_core::progress::Progress;
use illuminator_core::stats::{self, Region, Source};
use illuminator_core::volume::Volume;
use std::path::PathBuf;
use std::sync::Arc;

use crate::ui::{choice_items, format_value, rgb_to_rgba};

const COLUMN: f32 = 76.0;
const NO_VALUES: &str = "None (volumes only)";

#[derive(Clone)]
pub struct LayerSnapshot {
    pub name: SharedString,
    pub volume: Arc<Volume>,
    pub geometry: VolumeGeometry,
    pub frame: usize,
}

impl LayerSnapshot {
    fn source(&self) -> Source<'_> {
        Source {
            volume: &self.volume,
            geometry: &self.geometry,
            frame: self.frame,
        }
    }
}

pub struct StatisticsView {
    labels: LayerSnapshot,
    /// The layers whose values can be measured
    candidates: Vec<LayerSnapshot>,
    chosen: Option<usize>,
    /// None while measuring
    result: Option<Result<Vec<Region>, String>>,
    notice: Option<SharedString>,
    _measuring: Task<()>,
}

impl StatisticsView {
    pub fn new(
        labels: LayerSnapshot,
        candidates: Vec<LayerSnapshot>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            labels,
            chosen: (!candidates.is_empty()).then_some(0),
            candidates,
            result: None,
            notice: None,
            _measuring: Task::ready(()),
        };
        view.measure(cx);
        view
    }

    fn choose(&mut self, chosen: Option<usize>, cx: &mut Context<Self>) {
        self.chosen = chosen;
        self.measure(cx);
    }

    fn measure(&mut self, cx: &mut Context<Self>) {
        self.result = None;
        self.notice = None;
        let labels = self.labels.clone();
        let values = self.chosen.map(|index| self.candidates[index].clone());
        // replacing the task drops the one measuring the previous choice
        self._measuring = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let values = values.as_ref().map(LayerSnapshot::source);
                    stats::label_statistics(labels.source(), values, &Progress::default())
                        .map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.result = Some(result);
                cx.notify();
            });
        });
        cx.notify();
    }

    fn csv(&self) -> Option<String> {
        match &self.result {
            Some(Ok(regions)) => Some(stats::to_csv(regions)),
            _ => None,
        }
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(csv) = self.csv() {
            cx.write_to_clipboard(ClipboardItem::new_string(csv));
            self.notice = Some("Copied.".into());
            cx.notify();
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(csv) = self.csv() else {
            return;
        };
        let path = self.labels.volume.path();
        let directory = path.parent().map(PathBuf::from).unwrap_or_default();
        let name = strip_extension(&self.labels.name);
        let chosen = cx.prompt_for_new_path(&directory, Some(&format!("{name}_stats.csv")));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = chosen.await else {
                return; // cancelled
            };
            let notice = match std::fs::write(&path, csv) {
                Ok(()) => format!("Saved to {}", path.display()),
                Err(error) => {
                    eprintln!("Illuminator: saving {}: {error}", path.display());
                    format!("Cannot save: {error}")
                }
            };
            let _ = this.update(cx, |view, cx| {
                view.notice = Some(notice.into());
                cx.notify();
            });
        })
        .detach();
    }

    fn values_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let names: Vec<SharedString> = self.candidates.iter().map(|c| c.name.clone()).collect();
        let label = match self.chosen {
            Some(index) => names[index].clone(),
            None => NO_VALUES.into(),
        };
        Button::new("values")
            .xsmall()
            .label(label)
            .dropdown_menu(move |menu, _, _| {
                let candidates = names.iter().cloned().zip((0..).map(Some));
                let menu = choice_items(menu, &view, candidates, |view, chosen, _, cx| {
                    view.choose(chosen, cx);
                });
                let none = [(NO_VALUES, None)];
                choice_items(menu.separator(), &view, none, |view, chosen, _, cx| {
                    view.choose(chosen, cx);
                })
            })
    }
}

fn cell(text: impl Into<SharedString>) -> Div {
    div()
        .w(px(COLUMN))
        .flex_none()
        .text_right()
        .child(text.into())
}

impl Render for StatisticsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, border, danger) = (theme.muted_foreground, theme.border, theme.danger);
        let header = h_flex()
            .py_1()
            .border_b_1()
            .border_color(border)
            .text_color(muted)
            .child(div().flex_1().child("Label"))
            .child(cell("Voxels"))
            .child(cell("Volume ml"))
            .child(cell("Mean"))
            .child(cell("SD"))
            .child(cell("Min"))
            .child(cell("Max"));
        let body = match &self.result {
            None => div()
                .py_4()
                .text_color(muted)
                .child("Measuring\u{2026}")
                .into_any_element(),
            Some(Err(error)) => div()
                .py_4()
                .text_color(danger)
                .child(error.clone())
                .into_any_element(),
            Some(Ok(regions)) if regions.is_empty() => div()
                .py_4()
                .text_color(muted)
                .child("This layer has no labels other than 0")
                .into_any_element(),
            Some(Ok(regions)) => div()
                .id("regions")
                .max_h(px(380.0))
                .overflow_y_scroll()
                .children(regions.iter().map(|region| {
                    let [r, g, b, _] = label_color(region.label);
                    let values = region.values;
                    let value = |pick: fn(&stats::Summary) -> f64| {
                        values
                            .as_ref()
                            .map(pick)
                            .map(format_value)
                            .unwrap_or_default()
                    };
                    h_flex()
                        .py_0p5()
                        .child(
                            h_flex()
                                .flex_1()
                                .gap_2()
                                .child(div().size(px(10.0)).rounded_sm().bg(rgb_to_rgba([r, g, b])))
                                .child(region.label.to_string()),
                        )
                        .child(cell(region.voxels.to_string()))
                        .child(cell(format!("{:.2}", region.volume_ml)))
                        .child(cell(value(|v| v.mean)))
                        .child(cell(value(|v| v.std)))
                        .child(cell(value(|v| v.min)))
                        .child(cell(value(|v| v.max)))
                }))
                .into_any_element(),
        };
        let has_result = matches!(self.result, Some(Ok(_)));
        let frame = |snapshot: &LayerSnapshot| {
            if snapshot.volume.meta().timepoints() > 1 {
                format!(", frame {}", snapshot.frame + 1)
            } else {
                String::new()
            }
        };
        let values_frame = self
            .chosen
            .map(|index| frame(&self.candidates[index]))
            .unwrap_or_default();
        v_flex()
            .gap_2()
            .text_sm()
            .child(
                h_flex()
                    .gap_2()
                    .child(div().text_color(muted).child("Labels"))
                    .child(format!("{}{}", self.labels.name, frame(&self.labels))),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(div().text_color(muted).child("Values of"))
                    .child(self.values_button(cx))
                    .child(div().text_color(muted).child(values_frame)),
            )
            .child(v_flex().child(header).child(body))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(muted)
                            .children(self.notice.clone()),
                    )
                    .child(
                        Button::new("copy")
                            .xsmall()
                            .ghost()
                            .label("Copy")
                            .disabled(!has_result)
                            .on_click(cx.listener(|view, _, _, cx| view.copy(cx))),
                    )
                    .child(
                        Button::new("save")
                            .xsmall()
                            .label("Save CSV\u{2026}")
                            .disabled(!has_result)
                            .on_click(cx.listener(|view, _, _, cx| view.save(cx))),
                    ),
            )
    }
}
