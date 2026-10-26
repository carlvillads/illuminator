//! Drawing a plot window. Controls, plot, and legend

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, Bounds, Context, Div, FontWeight, Hsla, InteractiveElement, IntoElement,
    MouseMoveEvent, ParentElement, PathBuilder, Pixels, Render, Stateful,
    StatefulInteractiveElement, Styled, Window, canvas, div, fill, point, px, relative, size,
};
use std::rc::Rc;

use crate::shortcuts::{self, CloseWindow};
use crate::ui::{title_bar, uses_client_manager_decorations};

use super::super::STALE_OPACITY;
use super::{Axes, Chart, Curve, Plot, PlotLayer, PlotWindow, Plotted, Snapshot};

const PLOT_AXIS_WIDTH: f32 = 56.0;
const PLOT_AXIS_HEIGHT: f32 = 20.0;

/// How a layers curve is drawn
struct Stroke {
    color: Hsla,
    width: f32,
    curve: Curve,
}

impl<P: Plot> Render for PlotWindow<P> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.snapshot(cx);
        if let Some(snapshot) = &snapshot {
            self.forget_closed_region(snapshot, cx);
            self.schedule(snapshot, cx);
        }
        let theme = cx.theme();
        let (background, foreground, muted) =
            (theme.background, theme.foreground, theme.muted_foreground);
        let content = match &snapshot {
            Some(snapshot) => v_flex()
                .flex_1()
                .min_h_0()
                .p_3()
                .gap_2()
                .child(self.render_controls(snapshot, cx))
                .child(self.render_plot(snapshot, cx))
                .child(self.render_legend(snapshot, cx))
                .into_any_element(),
            None => message("The viewer is closed", muted),
        };
        v_flex()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .text_sm()
            .track_focus(&self.focus_handle)
            .key_context(shortcuts::PLOT_CONTEXT)
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .when(uses_client_manager_decorations(window), |root| {
                root.child(title_bar(P::TEXTS.title))
            })
            .child(content)
    }
}

impl<P: Plot> PlotWindow<P> {
    fn render_controls(&self, snapshot: &Snapshot<P::Key>, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let can_export = self.can_export(snapshot);
        P::controls(self, h_flex().gap_3().flex_wrap(), snapshot, cx)
            .child(div().flex_1())
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .children(self.notice.clone()),
            )
            .child(
                Button::new("copy")
                    .xsmall()
                    .ghost()
                    .label("Copy")
                    .tooltip(P::TEXTS.copy_tooltip)
                    .disabled(!can_export)
                    .on_click(cx.listener(|this, _, _, cx| this.copy(cx))),
            )
            .child(
                Button::new("save")
                    .xsmall()
                    .label("Save CSV\u{2026}")
                    .disabled(!can_export)
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            )
    }

    fn render_plot(&self, snapshot: &Snapshot<P::Key>, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        // selected layer last so it is drawn on top
        let mut plotted = self.plotted(snapshot);
        plotted.sort_by_key(|plotted| plotted.layer.selected);
        if plotted.is_empty() {
            return message(self.empty_message(snapshot), muted);
        }
        let Chart {
            axes,
            x_ticks,
            y_ticks,
            curves,
        } = self.plot.chart(&plotted, snapshot);
        let grid = y_ticks.iter().map(|&(at, _)| at).collect();
        let plot = self.plot_area(grid, strokes(&plotted, curves), cx);
        let mut plot = self.plot.overlay(plot, &axes, &plotted, snapshot, cx);
        // out of date values are plotted faded while the current ones are measured
        if plotted.iter().any(|plotted| plotted.stale) {
            plot = plot.child(stale_badge(self.measuring_text(snapshot), cx));
        }
        if let Some(x) = self.hover {
            plot = self.add_readout(plot, x, &axes, &plotted, cx);
        }
        let plot = self.track_hover(plot, axes.x, cx);
        with_axis_labels(plot, x_ticks, y_ticks, muted)
    }

    /// Why nothing is plotted
    fn empty_message(&self, snapshot: &Snapshot<P::Key>) -> &'static str {
        let unhidden: Vec<&PlotLayer<P::Key>> = self.unhidden(snapshot).collect();
        if snapshot.layers.is_empty() {
            P::TEXTS.no_layers
        } else if unhidden.is_empty() {
            "All layers are hidden"
        } else if unhidden.iter().all(|layer| layer.key.is_none()) {
            P::TEXTS.nothing_to_measure
        } else if unhidden
            .iter()
            .any(|layer| layer.key.is_some() && self.measurements.current(layer).is_none())
        {
            self.measuring_text(snapshot)
        } else {
            P::TEXTS.no_values
        }
    }

    fn measuring_text(&self, snapshot: &Snapshot<P::Key>) -> &'static str {
        if self.waiting(snapshot) {
            P::TEXTS.waiting
        } else {
            P::TEXTS.measuring
        }
    }

    /// The plots frame with its grid lines and curves
    fn plot_area(&self, grid: Vec<f32>, strokes: Vec<Stroke>, cx: &App) -> Stateful<Div> {
        let theme = cx.theme();
        let (border, background) = (theme.border, theme.secondary);
        let bounds = Rc::clone(&self.plot_bounds);
        div()
            .id("plot")
            .relative()
            .flex_1()
            .h_full()
            .overflow_hidden()
            .border_1()
            .border_color(border)
            .bg(background)
            .child(
                canvas(
                    move |area, _, _| bounds.set(area),
                    move |bounds: Bounds<Pixels>, _, window: &mut Window, _: &mut App| {
                        paint(bounds, &grid, strokes, border, window);
                    },
                )
                .absolute()
                .size_full(),
            )
    }

    /// A line at `x` and each layer's value there
    fn add_readout(
        &self,
        plot: Stateful<Div>,
        x: f64,
        axes: &Axes,
        plotted: &[Plotted<P>],
        cx: &App,
    ) -> Stateful<Div> {
        let theme = cx.theme();
        let (muted, background) = (theme.muted_foreground, theme.background);
        let at = axes.x(x);
        let rows = plotted
            .iter()
            .rev()
            .filter(|plotted| !plotted.stale)
            .map(|plotted| {
                let layer = plotted.layer;
                h_flex()
                    .gap_1()
                    .child(div().w(px(8.0)).h(px(8.0)).bg(layer.color))
                    .child(format!(
                        "{} {}",
                        layer.number,
                        self.plot.readout(plotted.value, x)
                    ))
            });
        let readout = v_flex()
            .absolute()
            .top_1()
            .p_1()
            .gap_0p5()
            .bg(background.opacity(0.9))
            .text_xs()
            .child(P::readout_title(x))
            .children(rows);
        plot.child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(relative(at))
                .w(px(1.0))
                .bg(muted),
        )
        .child(if at < 0.5 {
            readout.right_1()
        } else {
            readout.left_1()
        })
    }

    /// Keeps the x value under the mouse for the readout
    fn track_hover(
        &self,
        plot: Stateful<Div>,
        x_axis: (f64, f64),
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        plot.on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
            let Some(x) = this.x_at(event.position.x, x_axis) else {
                return;
            };
            let x = P::snap_x(x);
            if this.hover != Some(x) {
                this.hover = Some(x);
                cx.notify();
            }
        }))
        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
            if !hovered {
                this.hover = None;
                cx.notify();
            }
        }))
    }

    fn render_legend(&self, snapshot: &Snapshot<P::Key>, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let waiting = self.waiting(snapshot);
        h_flex()
            .gap_4()
            .flex_wrap()
            .text_xs()
            .children(snapshot.layers.iter().map(|layer| {
                let load_id = layer.load_id;
                let hidden = self.hidden.contains(&load_id);
                let note = match self.measurements.current(layer) {
                    // hidden layers aren't measured until they are shown
                    None if hidden => "",
                    None if layer.key.is_none() => P::TEXTS.nothing_to_measure_note,
                    None if waiting => P::TEXTS.waiting_note,
                    None => P::TEXTS.measuring_note,
                    Some(measured) if measured.value.is_none() => " (no values)",
                    Some(_) => "",
                };
                h_flex()
                    .id(("legend", load_id))
                    .gap_1()
                    .cursor_pointer()
                    .when(hidden, |entry| entry.opacity(0.4))
                    .when(layer.selected, |entry| {
                        entry.font_weight(FontWeight::SEMIBOLD)
                    })
                    .child(div().w(px(14.0)).h(px(3.0)).bg(layer.color))
                    .child(format!("{} {}", layer.number, layer.name))
                    .child(div().text_color(muted).child(note))
                    .tooltip(|window, cx| Tooltip::new("Show or hide").build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.hidden.remove(&load_id) {
                            this.hidden.insert(load_id);
                        }
                        cx.notify();
                    }))
            }))
    }
}

fn message(text: &'static str, color: Hsla) -> AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_color(color)
        .child(text)
        .into_any_element()
}

/// Faded for out of date values, thicker for the selected layer
fn strokes<P: Plot>(plotted: &[Plotted<P>], curves: Vec<Curve>) -> Vec<Stroke> {
    plotted
        .iter()
        .zip(curves)
        .map(|(plotted, curve)| Stroke {
            color: if plotted.stale {
                plotted.layer.color.opacity(STALE_OPACITY)
            } else {
                plotted.layer.color
            },
            width: if plotted.layer.selected { 2.0 } else { 1.25 },
            curve,
        })
        .collect()
}

/// Paints the grid lines at the heights in `grid`, and then strokes
fn paint(
    bounds: Bounds<Pixels>,
    grid: &[f32],
    strokes: Vec<Stroke>,
    grid_color: Hsla,
    window: &mut Window,
) {
    let at = |(x, y): (f32, f32)| {
        point(
            bounds.left() + bounds.size.width * x,
            bounds.bottom() - bounds.size.height * y,
        )
    };
    for &y in grid {
        let origin = at((0.0, y));
        window.paint_quad(fill(
            Bounds::new(origin, size(bounds.size.width, px(1.0))),
            grid_color,
        ));
    }
    for stroke in strokes {
        for polyline in stroke.curve {
            let mut path = PathBuilder::stroke(px(stroke.width));
            for (index, &step) in polyline.iter().enumerate() {
                if index == 0 {
                    path.move_to(at(step));
                } else {
                    path.line_to(at(step));
                }
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, stroke.color);
            }
        }
    }
}

/// Says the faded curves are measured again
fn stale_badge(text: &'static str, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .absolute()
        .top_1()
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(
            div()
                .px_1()
                .rounded_sm()
                .border_1()
                .border_color(theme.border)
                .bg(theme.secondary)
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text),
        )
}

/// The tick labels beside and below the plot
fn with_axis_labels(
    plot: Stateful<Div>,
    x_ticks: Vec<(f32, String)>,
    y_ticks: Vec<(f32, String)>,
    color: Hsla,
) -> AnyElement {
    let y_labels =
        div()
            .relative()
            .w(px(PLOT_AXIS_WIDTH))
            .h_full()
            .children(y_ticks.into_iter().map(|(at, label)| {
                div()
                    .absolute()
                    .right(px(6.0))
                    .top(relative(1.0 - at))
                    .mt(px(-7.0))
                    .text_xs()
                    .text_color(color)
                    .child(label)
            }));
    let x_labels = div()
        .relative()
        .h(px(PLOT_AXIS_HEIGHT))
        .ml(px(PLOT_AXIS_WIDTH))
        .children(x_ticks.into_iter().map(|(at, label)| {
            div()
                .absolute()
                .top_0p5()
                .left(relative(at))
                .ml(px(-40.0))
                .w(px(80.0))
                .flex()
                .justify_center()
                .text_xs()
                .text_color(color)
                .child(label)
        }));
    v_flex()
        .flex_1()
        .min_h_0()
        .child(h_flex().flex_1().min_h_0().child(y_labels).child(plot))
        .child(x_labels)
        .into_any_element()
}
