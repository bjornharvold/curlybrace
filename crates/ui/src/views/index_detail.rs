//! The right-hand side of the Indexes tab: one index with its chart and facts.
use super::index_chart::show_chart;
use super::indexes::hidden_marker;
use crate::I18n;
use crate::viewmodel::IndexesAction;
use crate::viewmodel::index_detail::{
    Age, definition_text, last_used_secs, option_phrases, size_parts, tracked_since_secs,
};
use curlybrace_core::history::ChartRange;
use curlybrace_core::runtime::IndexSeries;
use curlybrace_core::store::CollectionIndex;
use eframe::egui;

/// What the detail pane needs besides the index: its series (if one matches) and whether the
/// collection has any series at all.
pub struct DetailInput<'a> {
    pub index: &'a CollectionIndex,
    pub series: Option<&'a IndexSeries>,
    pub any_series: bool,
    pub range: ChartRange,
    /// The chart's right edge, in Unix seconds.
    pub now_secs: i64,
}

pub fn show_detail(
    ui: &mut egui::Ui,
    input: &DetailInput<'_>,
    i18n: &I18n,
) -> Option<IndexesAction> {
    let spec = &input.index.spec;
    let mut action = None;
    ui.horizontal(|ui| {
        ui.heading(&spec.name);
        if spec.options.hidden {
            hidden_marker(ui, i18n);
        }
    });
    definition_rows(ui, input.index, i18n);
    if ui.button(i18n.t("indexes.copy_json")).clicked() {
        action = Some(IndexesAction::CopyJson(spec.to_json()));
    }
    ui.separator();
    match input.series {
        Some(series) => series_rows(ui, series, input, i18n),
        None => {
            let key = if input.any_series {
                "indexes.no_series"
            } else {
                "indexes.no_history"
            };
            ui.label(i18n.t(key));
        }
    }
    action
}

fn definition_rows(ui: &mut egui::Ui, index: &CollectionIndex, i18n: &I18n) {
    let spec = &index.spec;
    let options: Vec<String> = option_phrases(&spec.options)
        .into_iter()
        .map(|(key, n)| match n {
            Some(n) => i18n.t_with(key, &[("n", &n.to_string())]),
            None => i18n.t(key),
        })
        .collect();
    let (size_key, size) = size_parts(index.size_bytes);
    egui::Grid::new("index_definition")
        .num_columns(2)
        .show(ui, |ui| {
            ui.strong(i18n.t("indexes.definition"));
            ui.vertical(|ui| {
                ui.monospace(definition_text(spec));
                if !options.is_empty() {
                    ui.label(options.join(", "));
                }
            });
            ui.end_row();
            ui.strong(i18n.t("indexes.size"));
            ui.label(i18n.t_with(size_key, &[("n", &size)]));
            ui.end_row();
        });
}

fn series_rows(ui: &mut egui::Ui, series: &IndexSeries, input: &DetailInput<'_>, i18n: &I18n) {
    ui.label(
        egui::RichText::new(i18n.t_with("indexes.chart_caption", &[("name", &series.index_name)]))
            .strong(),
    );
    show_chart(ui, series, input.range, input.now_secs, i18n);
    ui.add_space(ui.spacing().item_spacing.y);
    let ago = |at: i64| {
        let age = Age::of(input.now_secs - at);
        i18n.t_with(age.key(), &[("n", &age.count().to_string())])
    };
    egui::Grid::new("index_facts")
        .num_columns(2)
        .show(ui, |ui| {
            ui.strong(i18n.t("indexes.uses_since_restart"));
            ui.label(match series.latest {
                Some(sample) if sample.ops > 0 => sample.ops.to_string(),
                Some(_) => i18n.t("indexes.never_since_restart"),
                None => i18n.t("indexes.no_data"),
            });
            ui.end_row();
            ui.strong(i18n.t("indexes.last_used"));
            ui.label(match last_used_secs(series) {
                Some(at) => ago(at),
                None => i18n.t("indexes.not_in_range"),
            });
            ui.end_row();
            if let Some(start) = tracked_since_secs(series) {
                ui.strong(i18n.t("indexes.tracked_since"));
                ui.label(ago(start));
                ui.end_row();
            }
        });
    ui.label(i18n.t_with("indexes.node", &[("host", &series.host)]));
    ui.weak(i18n.t("indexes.node_note"));
}
