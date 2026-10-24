//! Draws the usage chart of one index: bars of uses per bucket, a line where the server
//! restarted and a shaded band where nothing was recorded. The numbers behind it are also
//! written as a text line, because a drawn chart is not read by screen readers.
use crate::I18n;
use crate::viewmodel::index_chart::{chart_model, chart_summary, observed_uses};
use crate::viewmodel::index_detail::range_key;
use curlybrace_core::history::ChartRange;
use curlybrace_core::runtime::IndexSeries;
use eframe::egui;
use egui_plot::{Bar, BarChart, Legend, Plot, PlotPoints, Polygon, VLine, uniform_grid_spacer};

const CHART_HEIGHT: f32 = 220.0;
/// Share of a bucket's width its bar fills, so neighbouring bars stay apart.
const BAR_FILL: f64 = 0.85;
/// Opacity of the band over a bucket that is wholly unobserved, and over one that is only partly.
const GAP_SHADE: f32 = 0.35;
const PARTIAL_GAP_SHADE: f32 = 0.15;

/// Draws `series` over `range` ending at `now_secs`, then its text summary.
pub fn show_chart(
    ui: &mut egui::Ui,
    series: &IndexSeries,
    range: ChartRange,
    now_secs: i64,
    i18n: &I18n,
) {
    let model = chart_model(series, range, now_secs);
    let top = model.points.iter().map(|p| p[1]).fold(1.0_f64, f64::max);
    let bar_width = range.bucket_width_secs() as f64 * BAR_FILL;
    let bars = model
        .points
        .iter()
        .map(|p| Bar::new(p[0], p[1]).width(bar_width))
        .collect();
    let visuals = ui.visuals().clone();
    let steps = grid_steps(range);
    let plot = Plot::new(("index_usage", &series.index_name, &series.host))
        .height(CHART_HEIGHT)
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        .include_x(-(range.secs() as f64))
        .include_x(0.0)
        .include_y(0.0)
        .include_y(top)
        .legend(Legend::default())
        .x_grid_spacer(uniform_grid_spacer(move |_| steps))
        .x_axis_formatter(|mark, _| axis_label(mark.value, range, i18n))
        .y_axis_formatter(|mark, _| {
            // Uses are counts: no tick between two whole numbers.
            if mark.value.fract() == 0.0 {
                whole_number(mark.value)
            } else {
                String::new()
            }
        })
        .label_formatter(|name, point| format!("{name}: {}", whole_number(point.y)));
    let tooltip_i18n = i18n.clone();
    let lower_bounds = model.lower_bounds.clone();
    let band = visuals.widgets.noninteractive.bg_stroke.color;
    plot.show(ui, |plot_ui| {
        let name = i18n.t("indexes.no_data");
        for (from, to) in &model.gaps {
            plot_ui.polygon(gap_band(
                &name,
                (*from, *to),
                top,
                band.gamma_multiply(GAP_SHADE),
            ));
        }
        // Only part of the bucket is unobserved: a lighter band, its bar is still drawn.
        for (from, to) in &model.partial_gaps {
            let shade = band.gamma_multiply(PARTIAL_GAP_SHADE);
            plot_ui.polygon(gap_band(&name, (*from, *to), top, shade));
        }
        plot_ui.bar_chart(
            BarChart::new(i18n.t("indexes.uses_series"), bars)
                .color(visuals.selection.bg_fill)
                .element_formatter(Box::new(move |bar, _| {
                    let lower = lower_bounds.contains(&bar.argument);
                    bar_tooltip(&tooltip_i18n, bar.value.round() as i64, lower)
                })),
        );
        for x in &model.restarts {
            plot_ui.vline(
                VLine::new(i18n.t("indexes.restart"), *x)
                    .color(visuals.warn_fg_color)
                    .width(1.5),
            );
        }
    });
    ui.label(summary_line(series, range, now_secs, i18n));
}

/// A translucent rectangle over `(from, to)`, as tall as the chart.
fn gap_band(name: &str, (from, to): (f64, f64), top: f64, fill: egui::Color32) -> Polygon<'static> {
    let corners = vec![[from, 0.0], [to, 0.0], [to, top], [from, top]];
    Polygon::new(name, PlotPoints::from(corners))
        .fill_color(fill)
        .stroke(egui::Stroke::NONE)
}

/// What hovering a bar says; a bucket with unobserved time says its count may be too low.
pub fn bar_tooltip(i18n: &I18n, uses: i64, lower_bound: bool) -> String {
    let key = if lower_bound {
        "indexes.lower_bound_tooltip"
    } else {
        "indexes.uses_count"
    };
    i18n.t_with(key, &[("n", &uses.to_string())])
}

/// The text alternative of the chart. With no observed bucket it says so instead of "0 uses".
pub fn summary_line(series: &IndexSeries, range: ChartRange, now_secs: i64, i18n: &I18n) -> String {
    let summary = chart_summary(series, range, now_secs);
    let range_text = i18n.t(range_key(range));
    let hours = summary.gap_hours.to_string();
    match observed_uses(series, range, now_secs) {
        Some(uses) => i18n.t_with(
            "indexes.summary",
            &[
                ("uses", &uses.to_string()),
                ("range", &range_text),
                ("restarts", &summary.restarts.to_string()),
                ("hours", &hours),
            ],
        ),
        None => i18n.t_with(
            "indexes.summary_unobserved",
            &[("range", &range_text), ("hours", &hours)],
        ),
    }
}

/// Spacing of the small, medium and large grid lines, in seconds.
fn grid_steps(range: ChartRange) -> [f64; 3] {
    match range {
        ChartRange::Day => [3_600.0, 10_800.0, 21_600.0],
        ChartRange::Week => [43_200.0, 86_400.0, 172_800.0],
        ChartRange::Month => [86_400.0, 432_000.0, 864_000.0],
    }
}

/// An x tick: how long before now it is, in hours for a day and in days otherwise.
fn axis_label(x: f64, range: ChartRange, i18n: &I18n) -> String {
    let (unit, key) = match range {
        ChartRange::Day => (3_600.0, "indexes.axis_hours"),
        ChartRange::Week | ChartRange::Month => (86_400.0, "indexes.axis_days"),
    };
    let ago = (-x / unit).round();
    if ago == 0.0 {
        return i18n.t("indexes.axis_now");
    }
    i18n.t_with(key, &[("n", &whole_number(ago))])
}

fn whole_number(value: f64) -> String {
    format!("{}", value.round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bar_with_unobserved_time_says_its_count_may_be_too_low() {
        let i18n = I18n::english();
        assert_eq!(bar_tooltip(&i18n, 40, false), "40 uses");
        let lower = bar_tooltip(&i18n, 40, true);
        assert_eq!(
            lower,
            i18n.t_with("indexes.lower_bound_tooltip", &[("n", "40")])
        );
        assert_ne!(lower, bar_tooltip(&i18n, 40, false));
    }
}
