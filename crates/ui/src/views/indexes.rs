//! The Indexes tab of a collection (layout B): the indexes on the left with their usage in the
//! selected range, the selected index on the right. Draws state and reports what the person
//! did; the reducer decides what happens.
use super::index_detail::{DetailInput, show_detail};
use super::index_sampling::{sampling_toggle, show_sampling_body};
use super::queries::show_queries_led;
use crate::I18n;
use crate::app::sampling_settings::SamplingSettings;
use crate::theme::{PILL_HEIGHT, filled_button, gate, pill, segmented, tonal_button};
use crate::viewmodel::connection_tag::READ_ONLY_REASON_KEY;
use crate::viewmodel::index_chart::observed_uses;
use crate::viewmodel::index_detail::{RANGES, anchored_now, range_key};
use crate::viewmodel::{IndexWrite, IndexesAction, IndexesState, IndexesView};
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
    if state.view == IndexesView::Queries {
        return queries_view(ui, state, read_only, i18n, now_secs);
    }
    // The toggle, the toolbar and the Sampling disclosure share one wrapping row.
    let (mut action, sampling_open, status_below) = ui
        .horizontal_wrapped(|ui| {
            let toggled = view_toggle(ui, state.view, i18n);
            let action = toolbar(ui, state, read_only, i18n).or(toggled);
            let open = sampling_toggle(ui, i18n);
            let status = state.sampler.map(|status| i18n.t(sampler_key(status)));
            let below = match status {
                Some(text) if fits_in_row(ui, &text) => {
                    ui.weak(text);
                    None
                }
                other => other,
            };
            (action, open, below)
        })
        .inner;
    // A status that does not fit beside the buttons gets a line of its own, a text line high
    // (a wrapped row would be a whole control row high), never split across the two.
    if let Some(text) = status_below {
        ui.weak(text);
    }
    if sampling_open {
        action = show_sampling_body(ui, sampling, i18n).or(action);
    }
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
                            series: state.series_of(&index.spec),
                            any_series: !state.series.is_empty(),
                            sampler: state.sampler,
                            range: state.range,
                            chart_end_secs: now,
                            now_secs,
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

/// The Indexes | Queries switch. The selected side is marked by the selection state, which
/// assistive technology reads, not by colour alone.
fn view_toggle(ui: &mut egui::Ui, view: IndexesView, i18n: &I18n) -> Option<IndexesAction> {
    let items = [
        (IndexesView::Indexes, i18n.t("indexes.queries.view_indexes")),
        (IndexesView::Queries, i18n.t("indexes.queries.view_queries")),
    ];
    segmented(ui, &items, view).map(IndexesAction::ShowView)
}

/// The Queries view of the collection the tab is on, the toggle leading its toolbar row.
fn queries_view(
    ui: &mut egui::Ui,
    state: &IndexesState,
    read_only: bool,
    i18n: &I18n,
    now_secs: i64,
) -> Option<IndexesAction> {
    let toggle = |ui: &mut egui::Ui| view_toggle(ui, state.view, i18n);
    let Some(ns) = state.ns.as_ref() else {
        return ui.horizontal_wrapped(toggle).inner;
    };
    let (queries, toggled) =
        show_queries_led(ui, &state.queries, ns, read_only, i18n, now_secs, toggle);
    queries.map(IndexesAction::Queries).or(toggled)
}

/// The selected index, or the first one while nothing is selected.
fn shown_index(state: &IndexesState) -> Option<&CollectionIndex> {
    state
        .selected
        .as_ref()
        .and_then(|name| state.indexes.iter().find(|i| i.spec.name == *name))
        .or_else(|| state.indexes.first())
}

/// Range switch, refresh and Create index, in the caller's row.
fn toolbar(
    ui: &mut egui::Ui,
    state: &IndexesState,
    read_only: bool,
    i18n: &I18n,
) -> Option<IndexesAction> {
    let mut action = None;
    let ranges = RANGES.map(|range| (range, i18n.t(range_key(range))));
    if let Some(range) = segmented(ui, &ranges, state.range) {
        action = Some(IndexesAction::SetRange(range));
    }
    if tonal_button(ui, i18n.t("indexes.refresh")).clicked() {
        action = Some(IndexesAction::Refresh);
    }
    if create_button(ui, read_only, state.writing.as_ref(), i18n) {
        action = Some(IndexesAction::OpenBuilder);
    }
    action
}

/// Whether `text` fits in what is left of the current row.
fn fits_in_row(ui: &egui::Ui, text: &str) -> bool {
    let width = egui::WidgetText::from(text)
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Body,
        )
        .size()
        .x;
    width <= ui.available_size_before_wrap().x
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
    let button = gate(ui, reason.is_none(), |ui| {
        filled_button(ui, i18n.t("indexes.create"))
    });
    if let Some(key) = reason {
        button.on_hover_text(i18n.t(key));
        ui.label(i18n.t(key));
        return false;
    }
    button.clicked()
}

fn sampler_key(status: SamplerStatus) -> &'static str {
    match status {
        SamplerStatus::Running => "indexes.sampler_running",
        SamplerStatus::Failing => "indexes.sampler_failing",
        SamplerStatus::Paused(SamplerPause::NotAuthorized) => "indexes.pause_not_authorized",
        SamplerStatus::Paused(SamplerPause::ConnectionLost) => "indexes.pause_connection_lost",
        SamplerStatus::Paused(SamplerPause::HistoryBusy) => "indexes.pause_history_busy",
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
        let uses = state
            .series_of(spec)
            .and_then(|series| observed_uses(series, state.range, now));
        let is_shown = shown.is_some_and(|s| s.spec.name == spec.name);
        // A never-used index is dimmed: it is the one worth a second look.
        let name = match uses {
            Some(0) => egui::RichText::new(&spec.name).weak(),
            _ => egui::RichText::new(&spec.name),
        };
        let count = match uses {
            Some(n) => i18n.t_with("indexes.uses_count", &[("n", &n.to_string())]),
            None => i18n.t("indexes.no_data"),
        };
        if row(ui, (name, is_shown, spec.options.hidden), &count, i18n) {
            action = Some(IndexesAction::Select(spec.name.clone()));
        }
    }
    action
}

/// One index: its name as a pill that elides when it is long, and the usage count (with the
/// hidden marker) kept readable at the right edge. Returns whether the row was clicked.
fn row(
    ui: &mut egui::Ui,
    (name, selected, hidden): (egui::RichText, bool, bool),
    count: &str,
    i18n: &I18n,
) -> bool {
    let gap = ui.spacing().item_spacing.x;
    let width_of = |ui: &egui::Ui, text: egui::RichText| {
        egui::WidgetText::from(text)
            .into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                egui::TextStyle::Body,
            )
            .size()
            .x
    };
    let count_width = width_of(ui, egui::RichText::new(count));
    let marker = i18n.t("indexes.hidden");
    let marker_width = width_of(ui, egui::RichText::new(&marker).small());
    let trailing = count_width + if hidden { marker_width + gap } else { 0.0 } + gap;
    let mut clicked = false;
    ui.horizontal(|ui| {
        let pill_size = egui::vec2((ui.available_width() - trailing).max(0.0), PILL_HEIGHT);
        let left_to_right = egui::Layout::left_to_right(egui::Align::Center);
        ui.allocate_ui_with_layout(pill_size, left_to_right, |ui| {
            clicked = pill(ui, selected, name).clicked();
        });
        let layout = egui::Layout::right_to_left(egui::Align::Center);
        let size = egui::vec2(ui.available_width(), PILL_HEIGHT);
        ui.allocate_ui_with_layout(size, layout, |ui| {
            ui.label(count);
            if hidden {
                hidden_marker(ui, i18n);
            }
        });
    });
    clicked
}

/// The small "hidden" badge next to the name of a hidden index.
pub(super) fn hidden_marker(ui: &mut egui::Ui, i18n: &I18n) {
    ui.label(
        egui::RichText::new(i18n.t("indexes.hidden"))
            .small()
            .color(ui.visuals().warn_fg_color),
    );
}
