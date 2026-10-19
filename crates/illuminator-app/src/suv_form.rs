//! The dialog for entering or editing a layer's SUV inputs.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::{ActiveTheme, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::*;
use illuminator_core::suv::{BiologicalSex, Normalization, SuvInputs, TRACERS};

use crate::ui::{choice_items, parse_value};
use crate::viewer::Viewer;

pub struct SuvForm {
    load_id: u64,
    viewer: WeakEntity<Viewer>,
    weight: Entity<InputState>,
    height: Entity<InputState>,
    dose: Entity<InputState>,
    minutes: Entity<InputState>,
    biological_sex: Option<BiologicalSex>,
    normalization: Normalization,
    half_life_minutes: f64,
    bq_ml_per_unit: f64,
    error: Option<SharedString>,
}

impl SuvForm {
    pub fn new(
        load_id: u64,
        known: Option<SuvInputs>,
        viewer: WeakEntity<Viewer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut input = |value: Option<f64>| {
            let text = value.map(|value| value.to_string()).unwrap_or_default();
            cx.new(|cx| InputState::new(window, cx).default_value(text))
        };
        let weight = input(known.map(|k| k.weight_kg));
        let height = input(known.and_then(|k| k.height_cm));
        let dose = input(known.map(|k| k.dose_mbq));
        let minutes = input(known.map(|k| k.minutes_after_injection));
        Self {
            load_id,
            viewer,
            weight,
            height,
            dose,
            minutes,
            biological_sex: known.and_then(|k| k.biological_sex),
            normalization: known.map(|k| k.normalization).unwrap_or_default(),
            half_life_minutes: known.map_or(TRACERS[0].1, |k| k.half_life_minutes),
            bq_ml_per_unit: known.map_or(1.0, |k| k.bq_ml_per_unit),
            error: None,
        }
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let number = |state: &Entity<InputState>| parse_value(&state.read(cx).value());
        let inputs = match (
            number(&self.weight),
            number(&self.dose),
            number(&self.minutes),
        ) {
            (Some(weight_kg), Some(dose_mbq), Some(minutes_after_injection)) => SuvInputs {
                weight_kg,
                height_cm: number(&self.height),
                biological_sex: self.biological_sex,
                normalization: self.normalization,
                dose_mbq,
                minutes_after_injection,
                half_life_minutes: self.half_life_minutes,
                bq_ml_per_unit: self.bq_ml_per_unit,
            },
            _ => {
                self.error = Some("Please enter all numbers".into());
                cx.notify();
                return;
            }
        };
        if let Err(error) = inputs.factor() {
            self.error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        let load_id = self.load_id;
        let _ = self.viewer.update(cx, |viewer, cx| {
            viewer.set_suv_inputs(load_id, inputs, cx);
        });
        window.close_dialog(cx);
    }

    fn tracer_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let form = cx.entity().downgrade();
        let label = match tracer_name(self.half_life_minutes) {
            Some(name) => name.to_string(),
            None => format!("{:.2} min", self.half_life_minutes),
        };
        Button::new("tracer")
            .xsmall()
            .label(label)
            .dropdown_menu(move |menu, _, _| {
                let tracers =
                    TRACERS.map(|(name, minutes)| (format!("{name} ({minutes} min)"), minutes));
                choice_items(menu, &form, tracers, |form, minutes, _, cx| {
                    form.half_life_minutes = minutes;
                    cx.notify();
                })
            })
    }

    fn biological_sex_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let form = cx.entity().downgrade();
        Button::new("biological_sex")
            .xsmall()
            .label(self.biological_sex.map_or("Unknown", BiologicalSex::label))
            .dropdown_menu(move |menu, _, _| {
                let sexes =
                    [BiologicalSex::Male, BiologicalSex::Female].map(|sex| (sex.label(), sex));
                choice_items(menu, &form, sexes, |form, biological_sex, _, cx| {
                    form.biological_sex = Some(biological_sex);
                    cx.notify();
                })
            })
    }

    fn normalization_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let form = cx.entity().downgrade();
        Button::new("normalization")
            .xsmall()
            .label(self.normalization.label())
            .dropdown_menu(move |menu, _, _| {
                let choices = Normalization::ALL.map(|n| (n.label(), n));
                choice_items(menu, &form, choices, |form, normalization, _, cx| {
                    form.normalization = normalization;
                    form.error = None;
                    cx.notify();
                })
            })
    }
}

impl Render for SuvForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        let row = |label: &'static str, input: &Entity<InputState>, unit: &'static str| {
            h_flex()
                .gap_2()
                .child(div().w(px(150.0)).child(label))
                .child(div().flex_1().child(Input::new(input).small()))
                .child(div().w(px(32.0)).text_color(muted).child(unit))
        };
        v_flex()
            .gap_2()
            .text_sm()
            .child(row("Body weight", &self.weight, "kg"))
            .child(row("Height", &self.height, "cm"))
            .child(choice_row("Sex", self.biological_sex_button(cx)))
            .child(row("Injected dose", &self.dose, "MBq"))
            .child(row("Injection to scan", &self.minutes, "min"))
            .child(choice_row("Tracer", self.tracer_button(cx)))
            .child(choice_row("Normalize by", self.normalization_button(cx)))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(div().pt_1().child("The image must be in Bq/mL"))
                    .child(div().child("Height and sex are only needed for lean body mass")),
            )
            .children(
                self.error
                    .clone()
                    .map(|error| div().text_xs().text_color(danger).child(error)),
            )
            .child(
                h_flex().justify_end().child(
                    Button::new("apply")
                        .small()
                        .primary()
                        .label("Show SUV")
                        .on_click(cx.listener(|form, _, window, cx| form.apply(window, cx))),
                ),
            )
    }
}

fn tracer_name(half_life: f64) -> Option<&'static str> {
    TRACERS
        .iter()
        .find(|&&(_, minutes)| (minutes - half_life).abs() <= minutes * 0.01)
        .map(|&(name, _)| name)
}

fn choice_row(label: &'static str, button: impl IntoElement) -> Div {
    h_flex()
        .gap_2()
        .child(div().w(px(150.0)).child(label))
        .child(button)
}

#[cfg(test)]
mod tests {
    use super::tracer_name;

    #[test]
    fn tracers_are_recognized_despite_rounding() {
        assert_eq!(tracer_name(109.77), Some("F-18"));
        assert_eq!(tracer_name(1221.66 / 60.0), Some("C-11"));
        assert_eq!(tracer_name(360.4), None);
    }
}
