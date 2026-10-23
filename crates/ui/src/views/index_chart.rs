//! Draws the usage chart of one index: bars of uses per bucket, a line where the server
//! restarted and a shaded band where nothing was recorded. The numbers behind it are also
//! written as a text line, because a drawn chart is not read by screen readers.
use crate::I18n;
use crate::viewmodel::index_chart::{chart_model, chart_summary};
use crate::viewmodel::index_detail::range_key;
use curlybrace_core::history::ChartRange;
use curlybrace_core::runtime::IndexSeries;
use eframe::egui;
use egui_plot::{Bar, BarChart, Legend, Plot, PlotPoints, Polygon, VLine, uniform_grid_spacer};

const CHART_HEIGHT: f32 = 220.0;
/// Share of a bucket's width its bar fills, so neighbouring bars stay apart.
const BAR_FILL: f64 = 0.85;

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
    plot.show(ui, |plot_ui| {
        for (from, to) in &model.gaps {
            let corners = vec![[*from, 0.0], [*to, 0.0], [*to, top], [*from, top]];
            plot_ui.polygon(
                Polygon::new(i18n.t("indexes.no_data"), PlotPoints::from(corners))
                    .fill_color(
                        visuals
                            .widgets
                            .noninteractive
                            .bg_stroke
                            .color
                            .gamma_multiply(0.35),
                    )
                    .stroke(egui::Stroke::NONE),
            );
        }
        plot_ui.bar_chart(
            BarChart::new(i18n.t("indexes.uses_series"), bars).color(visuals.selection.bg_fill),
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

/// The text alternative of the chart.
pub fn summary_line(series: &IndexSeries, range: ChartRange, now_secs: i64, i18n: &I18n) -> String {
    let summary = chart_summary(series, range, now_secs);
    i18n.t_with(
        "indexes.summary",
        &[
            ("uses", &summary.uses.to_string()),
            ("range", &i18n.t(range_key(range))),
            ("restarts", &summary.restarts.to_string()),
            ("hours", &summary.gap_hours.to_string()),
        ],
    )
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
