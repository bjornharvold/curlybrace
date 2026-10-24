//! The right-hand side of the Indexes tab: one index with its chart and facts.
use super::index_chart::show_chart;
use super::indexes::hidden_marker;
use crate::I18n;
use crate::viewmodel::IndexesAction;
use crate::viewmodel::connection_tag::READ_ONLY_REASON_KEY;
use crate::viewmodel::index_detail::{
    Age, definition_text, last_used_age, option_phrases, size_parts, tracked_since_age,
};
use crate::viewmodel::index_write::is_id_index;
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
    /// The chart's right edge, in Unix seconds (the next bucket boundary of the range).
    pub chart_end_secs: i64,
    /// The wall-clock time, in Unix seconds: the ages of the facts are measured against it, so
    /// they do not move with the range.
    pub now_secs: i64,
    /// The connection refuses writes: every write control is disabled and says why.
    pub read_only: bool,
    /// An index write is in flight: the write controls wait for it.
    pub busy: bool,
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
    action = buttons(ui, input, i18n).or(action);
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

/// Copy as JSON, and the write controls: hide or unhide, edit a copy, change the TTL (only an
/// index that expires has one) and drop. A write control that cannot be used is disabled and its
/// reason is its hover text; the `_id_` index, which the server refuses to change, says so once
/// in plain text. The reasons that hold for the whole tab (read-only, a write in flight) are
/// already in plain text in the toolbar.
fn buttons(ui: &mut egui::Ui, input: &DetailInput<'_>, i18n: &I18n) -> Option<IndexesAction> {
    let spec = &input.index.spec;
    let name = spec.name.clone();
    let is_id = is_id_index(&name);
    let reason = match (input.read_only, input.busy, is_id) {
        (true, _, _) => Some(READ_ONLY_REASON_KEY),
        (false, true, _) => Some("indexes.write_busy"),
        (false, false, true) => Some("indexes.id_locked"),
        (false, false, false) => None,
    };
    let hidden = spec.options.hidden;
    let hide = match hidden {
        true => (
            "indexes.action_unhide",
            IndexesAction::SetHidden(name.clone(), false),
        ),
        false => (
            "indexes.action_hide",
            IndexesAction::SetHidden(name.clone(), true),
        ),
    };
    let mut writes = vec![
        hide,
        (
            "indexes.action_edit_copy",
            IndexesAction::EditCopy(name.clone()),
        ),
    ];
    if spec.options.expire_after_seconds.is_some() {
        writes.push(("indexes.action_ttl", IndexesAction::ChangeTtl(name.clone())));
    }
    writes.push(("indexes.action_drop", IndexesAction::DropRequested(name)));
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        if ui.button(i18n.t("indexes.copy_json")).clicked() {
            action = Some(IndexesAction::CopyJson(spec.to_json()));
        }
        for (label, write) in writes {
            let button = ui.add_enabled(reason.is_none(), egui::Button::new(i18n.t(label)));
            if let Some(key) = reason {
                button.on_disabled_hover_text(i18n.t(key));
            } else if button.clicked() {
                action = Some(write);
            }
        }
    });
    if is_id {
        ui.label(i18n.t("indexes.id_locked"));
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
    show_chart(ui, series, input.range, input.chart_end_secs, i18n);
    ui.add_space(ui.spacing().item_spacing.y);
    let ago = |age: Age| i18n.t_with(age.key(), &[("n", &age.count().to_string())]);
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
            ui.label(match last_used_age(series, input.now_secs) {
                Some(age) => ago(age),
                None => i18n.t("indexes.not_in_range"),
            });
            ui.end_row();
            if let Some(age) = tracked_since_age(series, input.now_secs) {
                ui.strong(i18n.t("indexes.tracked_since"));
                ui.label(ago(age));
                ui.end_row();
            }
        });
    ui.label(i18n.t_with("indexes.node", &[("host", &series.host)]));
    ui.weak(i18n.t("indexes.node_note"));
}
