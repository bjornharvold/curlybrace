//! The Queries view of the Indexes tab: a one-line status row, the range and refresh, the miss
//! trend and the recorded shapes. Draws state and reports what the person did; the reducer
//! decides what happens. The status row has a fixed height, so nothing below it moves when the
//! profiler or the poller changes state.
use super::queries_chart::show_miss_chart;
use super::queries_rows::show_rows;
use crate::I18n;
use crate::viewmodel::connection_tag::READ_ONLY_REASON_KEY;
use crate::viewmodel::index_detail::{RANGES, anchored_now, range_key};
use crate::viewmodel::queries_status::{Body, Line, ProfilerControl, StatusRow, body, status_row};
use crate::viewmodel::{QueriesAction, QueriesState, rows};
use curlybrace_core::store::Namespace;
use eframe::egui;

const READ_ONLY_NOTE_KEY: &str = "indexes.queries.read_only_note";

/// Draws the view for `ns` at the wall-clock time `now_secs` (Unix seconds, read once per frame
/// by the caller). `read_only` turns the profiler button off and says why.
pub fn show_queries(
    ui: &mut egui::Ui,
    state: &QueriesState,
    ns: &Namespace,
    read_only: bool,
    i18n: &I18n,
    now_secs: i64,
) -> Option<QueriesAction> {
    let mut action = show_status(ui, &status_row(state, read_only), i18n);
    action = toolbar(ui, state, i18n).or(action);
    // The chart ends at a bucket boundary, so it holds still between frames.
    let end = anchored_now(state.range, now_secs);
    show_miss_chart(ui, &state.shapes, state.range, end, i18n);
    if state.error.is_some() {
        ui.colored_label(ui.visuals().error_fg_color, i18n.t("indexes.queries.error"));
    }
    ui.separator();
    let list = rows(state, ns);
    match body(state, list.len()) {
        Body::Rows => {
            egui::ScrollArea::vertical()
                .id_salt("queries_rows")
                .auto_shrink([false; 2])
                .show(ui, |ui| show_rows(ui, &list, i18n));
        }
        other => {
            ui.label(i18n.t(empty_key(other)));
        }
    }
    action
}

fn empty_key(body: Body) -> &'static str {
    match body {
        Body::Loading | Body::Rows => "indexes.queries.loading",
        Body::ProfilerOff => "indexes.queries.empty_off",
        Body::ProfilerOnEmpty => "indexes.queries.empty_on",
        Body::Unavailable => "indexes.queries.empty_unavailable",
    }
}

fn text(line: &Line, i18n: &I18n) -> String {
    let params: Vec<(&str, &str)> = line.params.iter().map(|(n, v)| (*n, v.as_str())).collect();
    i18n.t_with(line.key, &params)
}

/// The one-line status: profiler state, the button, why it is off, and the poller's status.
fn show_status(ui: &mut egui::Ui, row: &StatusRow, i18n: &I18n) -> Option<QueriesAction> {
    let height = ui.spacing().interact_size.y;
    let mut action = None;
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), height), layout, |ui| {
        ui.set_min_height(height);
        ui.strong(text(&row.profiler, i18n));
        action = row
            .control
            .and_then(|control| button(ui, row, control, i18n));
        if let Some(key) = row.disabled_reason {
            ui.label(i18n.t(key));
        }
        let poller = egui::RichText::new(text(&row.poller, i18n)).weak();
        ui.add(egui::Label::new(poller).truncate());
    });
    action
}

fn button(
    ui: &mut egui::Ui,
    row: &StatusRow,
    control: ProfilerControl,
    i18n: &I18n,
) -> Option<QueriesAction> {
    let (label_key, action) = match control {
        ProfilerControl::Enable => ("indexes.queries.enable", QueriesAction::AskEnable),
        ProfilerControl::TurnOff => ("indexes.queries.turn_off", QueriesAction::AskDisable),
    };
    let response = ui.add_enabled(row.enabled, egui::Button::new(i18n.t(label_key)));
    // The reason is visible next to the button; hovering gives the full sentence.
    if let Some(key) = row.disabled_reason {
        let full = if key == READ_ONLY_NOTE_KEY {
            READ_ONLY_REASON_KEY
        } else {
            key
        };
        response.clone().on_disabled_hover_text(i18n.t(full));
    }
    (response.clicked() && row.enabled).then_some(action)
}

/// The range switch and refresh.
fn toolbar(ui: &mut egui::Ui, state: &QueriesState, i18n: &I18n) -> Option<QueriesAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        for range in RANGES {
            let label = i18n.t(range_key(range));
            if ui.selectable_label(state.range == range, label).clicked() {
                action = Some(QueriesAction::SetRange(range));
            }
        }
        if ui.button(i18n.t("indexes.refresh")).clicked() {
            action = Some(QueriesAction::Refresh);
        }
    });
    action
}
