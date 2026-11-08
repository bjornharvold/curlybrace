//! The miss trend of the Queries view: bars of misses per bucket (summed over all shapes) and a
//! shaded band where nothing was observed. The total is also written as a text line, because a
//! drawn chart is not read by screen readers.
use super::index_chart::{axis_label, gap_band, grid_steps, whole_number};
use crate::I18n;
use crate::viewmodel::index_detail::range_key;
use crate::viewmodel::queries_chart::miss_chart_model;
use curlybrace_core::history::{ChartRange, ShapeSummary};
use eframe::egui;
use egui_plot::{Bar, BarChart, Legend, Plot, uniform_grid_spacer};

const CHART_HEIGHT: f32 = 140.0;
/// Share of a bucket's width its bar fills, so neighbouring bars stay apart.
const BAR_FILL: f64 = 0.85;
const GAP_SHADE: f32 = 0.35;
const PARTIAL_GAP_SHADE: f32 = 0.15;

/// Draws the misses of `summaries` over `range` ending at `now_secs`, then its text summary.
pub fn show_miss_chart(
    ui: &mut egui::Ui,
    summaries: &[ShapeSummary],
    range: ChartRange,
    now_secs: i64,
    i18n: &I18n,
) {
    let model = miss_chart_model(summaries, range, now_secs);
    let top = model.bars.iter().map(|b| b[1]).fold(1.0_f64, f64::max);
    let bar_width = range.bucket_width_secs() as f64 * BAR_FILL;
    let bars = model
        .bars
        .iter()
        .map(|b| Bar::new(b[0], b[1]).width(bar_width))
        .collect();
    let visuals = ui.visuals().clone();
    let steps = grid_steps(range);
    let band = visuals.widgets.noninteractive.bg_stroke.color;
    let lower_bounds = model.lower_bounds.clone();
    let tooltip_i18n = i18n.clone();
    Plot::new("queries_misses")
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
            if mark.value.fract() == 0.0 {
                whole_number(mark.value)
            } else {
                String::new()
            }
        })
        .label_formatter(|name, point| format!("{name}: {}", whole_number(point.y)))
        .show(ui, |plot_ui| {
            let name = i18n.t("indexes.no_data");
            for (from, to) in &model.gaps {
                plot_ui.polygon(gap_band(
                    &name,
                    (*from, *to),
                    top,
                    band.gamma_multiply(GAP_SHADE),
                ));
            }
            for (from, to) in &model.partial_gaps {
                let shade = band.gamma_multiply(PARTIAL_GAP_SHADE);
                plot_ui.polygon(gap_band(&name, (*from, *to), top, shade));
            }
            plot_ui.bar_chart(
                BarChart::new(i18n.t("indexes.queries.misses_series"), bars)
                    .color(visuals.warn_fg_color)
                    .element_formatter(Box::new(move |bar, _| {
                        let key = if lower_bounds.contains(&bar.argument) {
                            "indexes.queries.lower_bound_tooltip"
                        } else {
                            "indexes.queries.misses_tooltip"
                        };
                        let n = (bar.value.round() as i64).to_string();
                        tooltip_i18n.t_with(key, &[("n", &n)])
                    })),
            );
        });
    ui.label(summary_line(
        model.total_misses,
        !model.bars.is_empty(),
        range,
        i18n,
    ));
}

/// The text alternative: with nothing observed it says so instead of "0 misses".
fn summary_line(total: u64, any_bar: bool, range: ChartRange, i18n: &I18n) -> String {
    if !any_bar {
        return i18n.t("indexes.queries.chart_empty");
    }
    let key = match total {
        1 => "indexes.queries.chart_summary_one",
        _ => "indexes.queries.chart_summary_other",
    };
    i18n.t_with(
        key,
        &[
            ("misses", &total.to_string()),
            ("range", &i18n.t(range_key(range))),
        ],
    )
}
