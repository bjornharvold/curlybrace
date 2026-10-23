//! The Indexes tab of a collection (layout B): the indexes on the left with their usage in the
//! selected range, the selected index on the right. Draws state and reports what the person
//! did; the reducer decides what happens.
use super::index_detail::{DetailInput, show_detail};
use super::index_sampling::show_sampling;
use crate::I18n;
use crate::app::sampling_settings::SamplingSettings;
use crate::viewmodel::connection_tag::READ_ONLY_REASON_KEY;
use crate::viewmodel::index_chart::observed_uses;
use crate::viewmodel::index_detail::{RANGES, anchored_now, range_key, series_of};
use crate::viewmodel::{IndexWrite, IndexesAction, IndexesState};
use curlybrace_core::runtime::{SamplerPause, SamplerStatus};
use curlybrace_core::store::CollectionIndex;
use eframe::egui;

const LIST_MIN_WIDTH: f32 = 180.0;
const LIST_MAX_WIDTH: f32 = 320.0;
/// Share of the tab the index list takes.
const LIST_SHARE: f32 = 0.3;

/// Draws the tab at the wall-clock time `now_secs` (Unix seconds), which the caller reads once
/// per frame; nothing in here reads the clock, so the output depends only on the arguments.
/// `read_only` disables the write controls (Create index) and says why. `sampling` is the list of
/// databases the sampler skips, shown in the Sampling section.
pub fn show_indexes(
    ui: &mut egui::Ui,
    state: &IndexesState,
    read_only: bool,
    i18n: &I18n,
    now_secs: i64,
    sampling: &SamplingSettings,
) -> Option<IndexesAction> {
    let mut action = toolbar(ui, state, read_only, i18n);
    action = show_sampling(ui, sampling, i18n).or(action);
    ui.separator();
    if state.indexes.is_empty() {
        let key = if state.loading {
            "indexes.loading"
        } else {
            "indexes.no_indexes"
        };
        ui.label(i18n.t(key));
        return action;
    }
    // The chart ends at a bucket boundary, so it holds still between frames.
    let now = anchored_now(state.range, now_secs);
    let shown = shown_index(state);
    let list_width = (ui.available_width() * LIST_SHARE).clamp(LIST_MIN_WIDTH, LIST_MAX_WIDTH);
    ui.horizontal_top(|ui| {
        let layout = egui::Layout::top_down(egui::Align::Min);
        let size = egui::vec2(list_width, ui.available_height());
        ui.allocate_ui_with_layout(size, layout, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("indexes_list")
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    action = list(ui, state, shown, now, i18n).or(action.take());
                });
        });
        ui.separator();
        ui.vertical(|ui| {
            egui::ScrollArea::vertical()
                .id_salt("index_detail")
                .auto_shrink([false; 2])
                .show(ui, |ui| match shown {
                    Some(index) => {
                        let input = DetailInput {
                            index,
                            series: series_of(&state.series, &index.spec),
                            any_series: !state.series.is_empty(),
                            range: state.range,
                            now_secs: now,
                            read_only,
                            busy: state.writing.is_some(),
                        };
                        action = show_detail(ui, &input, i18n).or(action.take());
                    }
                    None => {
                        ui.label(i18n.t("indexes.select_index"));
                    }
                });
        });
    });
    action
}

/// The selected index, or the first one while nothing is selected.
fn shown_index(state: &IndexesState) -> Option<&CollectionIndex> {
    state
        .selected
        .as_ref()
        .and_then(|name| state.indexes.iter().find(|i| i.spec.name == *name))
        .or_else(|| state.indexes.first())
}

/// Range switch, refresh, Create index and the sampler's status line.
fn toolbar(
    ui: &mut egui::Ui,
    state: &IndexesState,
    read_only: bool,
    i18n: &I18n,
) -> Option<IndexesAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        for range in RANGES {
            let label = i18n.t(range_key(range));
            if ui.selectable_label(state.range == range, label).clicked() {
                action = Some(IndexesAction::SetRange(range));
            }
        }
        if ui.button(i18n.t("indexes.refresh")).clicked() {
            action = Some(IndexesAction::Refresh);
        }
        if create_button(ui, read_only, state.writing.as_ref(), i18n) {
            action = Some(IndexesAction::OpenBuilder);
        }
        if let Some(key) = state.sampler.map(sampler_key) {
            ui.weak(i18n.t(key));
        }
    });
    action
}

/// The Create index button. On a read-only connection it is disabled and the reason is both its
/// hover text and a visible note, so the state is never colour or hover alone.
fn create_button(
    ui: &mut egui::Ui,
    read_only: bool,
    writing: Option<&IndexWrite>,
    i18n: &I18n,
) -> bool {
    let reason = match (read_only, writing) {
        (true, _) => Some(READ_ONLY_REASON_KEY),
        (false, Some(IndexWrite::Create { .. })) => Some("indexes.create_busy"),
        (false, Some(_)) => Some("indexes.write_busy"),
        (false, None) => None,
    };
    let button = ui.add_enabled(
        reason.is_none(),
        egui::Button::new(i18n.t("indexes.create")),
    );
    if let Some(key) = reason {
        button.on_disabled_hover_text(i18n.t(key));
        ui.label(i18n.t(key));
        return false;
    }
    button.clicked()
}

fn sampler_key(status: SamplerStatus) -> &'static str {
    match status {
        SamplerStatus::Running => "indexes.sampler_running",
        SamplerStatus::Paused(SamplerPause::NotAuthorized) => "indexes.pause_not_authorized",
        SamplerStatus::Paused(SamplerPause::ConnectionLost) => "indexes.pause_connection_lost",
        SamplerStatus::Paused(SamplerPause::HistoryUnavailable) => {
            "indexes.pause_history_unavailable"
        }
    }
}

/// Every index with its uses in the range; the shown one is highlighted.
fn list(
    ui: &mut egui::Ui,
    state: &IndexesState,
    shown: Option<&CollectionIndex>,
    now: i64,
    i18n: &I18n,
) -> Option<IndexesAction> {
    let mut action = None;
    ui.strong(i18n.t("indexes.list_heading"));
    ui.horizontal(|ui| {
        ui.weak(i18n.t("indexes.column_name"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.weak(i18n.t("indexes.column_uses"));
        });
    });
    for index in &state.indexes {
        let spec = &index.spec;
        // `None` unless the window was observed: a guessed zero would mark an index unused.
        let uses = series_of(&state.series, spec)
            .and_then(|series| observed_uses(series, state.range, now));
        let is_shown = shown.is_some_and(|s| s.spec.name == spec.name);
        ui.horizontal(|ui| {
            // A never-used index is dimmed: it is the one worth a second look.
            let name = match uses {
                Some(0) => egui::RichText::new(&spec.name).weak(),
                _ => egui::RichText::new(&spec.name),
            };
            if ui.selectable_label(is_shown, name).clicked() {
                action = Some(IndexesAction::Select(spec.name.clone()));
            }
            if spec.options.hidden {
                hidden_marker(ui, i18n);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(match uses {
                    Some(n) => i18n.t_with("indexes.uses_count", &[("n", &n.to_string())]),
                    None => i18n.t("indexes.no_data"),
                });
            });
        });
    }
    action
}

/// The small "hidden" badge next to the name of a hidden index.
pub(super) fn hidden_marker(ui: &mut egui::Ui, i18n: &I18n) {
    ui.label(
        egui::RichText::new(i18n.t("indexes.hidden"))
            .small()
            .color(ui.visuals().warn_fg_color),
    );
}
