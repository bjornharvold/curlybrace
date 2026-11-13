//! The Queries view of the Indexes tab: the range and refresh (after whatever the caller leads
//! that row with), a one-line status row, the miss trend and the recorded shapes. Draws state and reports what the person did; the reducer
//! decides what happens. The status row has a fixed height, so nothing below it moves when the
//! profiler or the poller changes state.
use super::queries_chart::show_miss_chart;
use super::queries_rows::show_rows;
use super::tone_notice::tone_notice;
use crate::I18n;
use crate::theme::{CONTROL_HEIGHT, Tone, filled_button, gate, segmented, tonal_button};
use crate::viewmodel::connection_tag::READ_ONLY_REASON_KEY;
use crate::viewmodel::index_detail::{RANGES, anchored_now, range_key};
use crate::viewmodel::queries_status::{
    Body, Line, ProfilerControl, StatusRow, body, level_note, status_row,
};
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
    show_queries_led(ui, state, ns, read_only, i18n, now_secs, |_| ()).0
}

/// [`show_queries`] with `leading` drawn at the start of the range and refresh row (the Indexes
/// tab puts its Indexes | Queries toggle there); returns what `leading` returned too.
pub fn show_queries_led<R>(
    ui: &mut egui::Ui,
    state: &QueriesState,
    ns: &Namespace,
    read_only: bool,
    i18n: &I18n,
    now_secs: i64,
    leading: impl FnOnce(&mut egui::Ui) -> R,
) -> (Option<QueriesAction>, R) {
    let (toolbar_action, led) = toolbar(ui, state, i18n, leading);
    let status = show_status(ui, &status_row(state, read_only), i18n);
    let action = toolbar_action.or(status);
    // The chart ends at a bucket boundary, so it holds still between frames.
    let end = anchored_now(state.range, now_secs);
    show_miss_chart(ui, &state.shapes, state.range, end, i18n);
    // Two independent failures, each with its own text; neither hides the other.
    if state.shapes_error.is_some() {
        tone_notice(ui, Tone::Bad, &i18n.t("indexes.queries.error"), false);
    }
    if state.profiler_error.is_some() {
        tone_notice(
            ui,
            Tone::Bad,
            &i18n.t("indexes.queries.profiler_error"),
            false,
        );
    }
    ui.separator();
    if let Some(note) = level_note(state) {
        ui.weak(text(&note, i18n));
    }
    let list = rows(state, ns);
    match body(state, list.len()) {
        Body::Rows => {
            egui::ScrollArea::vertical()
                .id_salt("queries_rows")
                .auto_shrink([false; 2])
                .show(ui, |ui| show_rows(ui, &list, i18n));
        }
        // The inline error above says why there is nothing; "no queries yet" would be untrue.
        Body::ReadFailed => {}
        other => {
            ui.label(i18n.t(empty_key(other)));
        }
    }
    (action, led)
}

fn empty_key(body: Body) -> &'static str {
    match body {
        Body::Loading | Body::Rows | Body::ReadFailed => "indexes.queries.loading",
        Body::ProfilerOff => "indexes.queries.empty_off",
        Body::ProfilerOnEmpty => "indexes.queries.empty_on",
        Body::Unavailable => "indexes.queries.empty_unavailable",
    }
}

fn text(line: &Line, i18n: &I18n) -> String {
    let params: Vec<(&str, &str)> = line.params.iter().map(|(n, v)| (*n, v.as_str())).collect();
    i18n.t_with(line.key, &params)
}

/// The status: profiler state, the buttons, why they are off, and the poller's status. One line of
/// constant height at normal widths; when the window is too narrow it wraps onto more lines so
/// nothing is pushed past the right edge.
fn show_status(ui: &mut egui::Ui, row: &StatusRow, i18n: &I18n) -> Option<QueriesAction> {
    // The height of the tallest thing the row holds (a button), so no state is shorter.
    let height = ui.spacing().interact_size.y.max(CONTROL_HEIGHT);
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        ui.set_min_height(height);
        ui.strong(text(&row.profiler, i18n));
        for control in &row.controls {
            action = button(ui, row, *control, i18n).or(action.take());
        }
        if let Some(key) = row.disabled_reason {
            ui.label(i18n.t(key));
        }
        ui.weak(text(&row.poller, i18n));
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
        ProfilerControl::RecordAll => ("indexes.queries.record_all", QueriesAction::AskEnable),
    };
    let label = i18n.t(label_key);
    let response = gate(ui, row.enabled, |ui| match control {
        ProfilerControl::Enable => filled_button(ui, label),
        ProfilerControl::TurnOff | ProfilerControl::RecordAll => tonal_button(ui, label),
    });
    // The reason is visible next to the buttons; hovering gives the full sentence.
    if let Some(key) = row.disabled_reason {
        let full = if key == READ_ONLY_NOTE_KEY {
            READ_ONLY_REASON_KEY
        } else {
            key
        };
        response.clone().on_hover_text(i18n.t(full));
    }
    (response.clicked() && row.enabled).then_some(action)
}

/// `leading`, the range switch and refresh on one wrapping row.
fn toolbar<R>(
    ui: &mut egui::Ui,
    state: &QueriesState,
    i18n: &I18n,
    leading: impl FnOnce(&mut egui::Ui) -> R,
) -> (Option<QueriesAction>, R) {
    let mut action = None;
    let led = ui
        .horizontal_wrapped(|ui| {
            let led = leading(ui);
            let ranges = RANGES.map(|range| (range, i18n.t(range_key(range))));
            if let Some(range) = segmented(ui, &ranges, state.range) {
                action = Some(QueriesAction::SetRange(range));
            }
            if tonal_button(ui, i18n.t("indexes.refresh")).clicked() {
                action = Some(QueriesAction::Refresh);
            }
            led
        })
        .inner;
    (action, led)
}
