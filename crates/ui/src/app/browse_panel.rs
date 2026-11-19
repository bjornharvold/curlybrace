//! The Browse page of a collection, top to bottom: one row with the connection header and the
//! Documents | Indexes | Aggregate tabs; the find bar; one wrapping row with the two global switches and
//! the index verdict; the guard's notice (only while it has something to say); then the
//! documents with their one-row toolbar. Kept dense so the data starts high on the window.
use super::create_index_state::create_index_state;
use super::panels::UiEvent;
use super::reducer::Input;
use super::state::{AppState, ConnStatus};
use crate::I18n;
use crate::app::AggregateAction;
use crate::viewmodel::verdict::badge;
use crate::viewmodel::{CollectionTab, copy_blocked};
use crate::views::aggregate::{flush_typing, show_aggregate};
use crate::views::aggregate_run::RunContext;
use crate::views::code_export::show_code_dialog;
use crate::views::collection_tabs::show_collection_tabs;
use crate::views::documents::{
    DocTarget, show_connection_header_with, show_documents_pending, show_documents_switchable,
};
use crate::views::findbar::{
    FindAction, show_automatic_verdict, show_findbar_with, show_guard_setting,
};
use crate::views::findbar_options::show_page_options_status;
use crate::views::guard_notice::{GuardNoticeAction, show_guard_notice};
use crate::views::indexes::show_indexes;
use crate::views::verdict_badge::{
    VerdictAction, fits_beside, show_verdict_row, verdict_row_height,
};
use eframe::egui;

/// What the two switches take of a row (English labels, with the gap between them) until a frame
/// has measured them, for deciding before they are drawn whether the verdict shares their row.
const SWITCHES_GUESS: f32 = 280.0;

pub(super) fn show_collection(
    ui: &mut egui::Ui,
    state: &mut AppState,
    i18n: &I18n,
    out: &mut Vec<UiEvent>,
    now_secs: i64,
) {
    if state.selected_ns().is_none() {
        ui.label(i18n.t("browser.select_collection"));
        return;
    }
    // The Copy as code dialog belongs to both tabs; it copies to the clipboard itself.
    out.extend(
        show_code_dialog(ui.ctx(), &state.code_export, i18n)
            .into_iter()
            .map(|action| UiEvent::Input(Input::Aggregate(AggregateAction::Code(action)))),
    );
    let connection = state.active_connection_label(i18n);
    let badge = state.active_badge();
    let picked = show_connection_header_with(ui, &connection, &badge, i18n, |ui| {
        show_collection_tabs(ui, state.tab, i18n)
    });
    if let Some(tab) = picked {
        out.push(UiEvent::Input(Input::Tab(tab)));
    }
    let leaving = picked.is_some_and(|tab| tab != state.tab);
    match state.tab {
        CollectionTab::Indexes => {
            show_indexes_tab(ui, state, i18n, out, now_secs);
            return;
        }
        CollectionTab::Aggregate => {
            show_aggregate_tab(ui, state, leaving, i18n, out);
            return;
        }
        CollectionTab::Documents => {}
    }
    let (has_prev, has_next) = state.paging();
    let hints = &state.aggregate.options_ui.hint;
    let actions = show_findbar_with(ui, &mut state.find, (has_prev, has_next), hints, i18n);
    out.extend(
        actions
            .into_iter()
            .map(|action| UiEvent::Input(Input::Find(action))),
    );
    show_switches_and_verdict(ui, state, i18n, out);
    show_guard(ui, state, i18n, out);
    let target = DocTarget {
        connection: &connection,
        production: badge.production,
        read_only: badge.read_only,
        bulk: state.bulk_entry(),
    };
    let (Some(page), Some(key)) = (&state.page, &state.page_key) else {
        // The toolbar row stays in its place (and the view can still be switched) without a page.
        let empty = if state.loading_page {
            "browser.loading"
        } else {
            "browser.load_failed"
        };
        show_documents_pending(
            ui,
            &mut state.view_mode,
            &mut state.tree,
            target,
            empty,
            i18n,
        );
        return;
    };
    state.tree.projected = state.page_is_projected();
    state.tree.copy_blocked = state.selected_ns().is_some_and(copy_blocked);
    let action = show_documents_switchable(
        ui,
        &mut state.view_mode,
        page,
        key,
        &mut state.tree,
        target,
        i18n,
    );
    if let Some(action) = action {
        out.push(UiEvent::Input(Input::Doc(action)));
    }
}

/// The two global switches and, while explain is on, the verdict in one wrapping row. The
/// verdict keeps a fixed size whether or not there is one, so the documents never move when it
/// appears; whether the row wraps depends on the window alone.
fn show_switches_and_verdict(
    ui: &mut egui::Ui,
    state: &AppState,
    i18n: &I18n,
    out: &mut Vec<UiEvent>,
) {
    let actions = ui
        .horizontal_wrapped(|ui| {
            // With the verdict's area beside them the switches are centred on it, not on top;
            // when it wraps to a line of its own the switches' row stays as short as they are.
            let memory = ui.id().with("switches_width");
            let known = ui.data(|d| d.get_temp::<f32>(memory));
            if state.explain_enabled() && fits_beside(ui, known.unwrap_or(SWITCHES_GUESS)) {
                // The row's room is only as tall as its first widget until something taller
                // has been placed, so the row is stretched first (a zero-width rect).
                let height = verdict_row_height(ui);
                ui.set_row_height(height);
                ui.expand_to_include_rect(egui::Rect::from_min_size(
                    ui.cursor().min,
                    egui::vec2(0.0, height),
                ));
            }
            let start = ui.cursor().min;
            let switches = [
                show_automatic_verdict(ui, state.explain_enabled(), i18n),
                show_guard_setting(ui, state.guard_enabled(), i18n),
            ];
            remember_switches_width(ui, memory, known, start);
            let created = show_verdict(ui, state, i18n);
            show_page_options_status(ui, &state.page_query.options, i18n);
            (switches, created)
        })
        .inner;
    let (switches, created) = actions;
    out.extend(
        switches
            .into_iter()
            .flatten()
            .chain(created)
            .map(|action| UiEvent::Input(Input::Find(action))),
    );
}

/// Stores what the switches took of the row (from `start` to the cursor, less the gap after
/// them) for the next frame's decision, and repeats this frame when it was not what the decision
/// assumed. Nothing is stored when they wrapped onto another line: that width is not theirs.
fn remember_switches_width(ui: &egui::Ui, memory: egui::Id, known: Option<f32>, start: egui::Pos2) {
    let end = ui.cursor().min;
    if (end.y - start.y).abs() > 1.0 {
        return;
    }
    let measured = end.x - start.x - ui.spacing().item_spacing.x;
    if known.is_none_or(|was| (was - measured).abs() > 0.5) {
        ui.data_mut(|d| d.insert_temp(memory, measured));
        ui.ctx().request_discard("switches measured");
    }
}

/// The index-miss guard's notice under the switches (nothing unless a Run is being checked or
/// held back); Run anyway and Cancel go to the reducer, the copy button to the clipboard here.
fn show_guard(ui: &mut egui::Ui, state: &AppState, i18n: &I18n, out: &mut Vec<UiEvent>) {
    let find = match show_guard_notice(ui, &state.guard, i18n) {
        Some(GuardNoticeAction::RunAnyway) => Some(FindAction::GuardRunAnyway),
        Some(GuardNoticeAction::Cancel) => Some(FindAction::GuardCancel),
        Some(GuardNoticeAction::Copy(text)) => {
            ui.ctx().copy_text(text);
            None
        }
        None => None,
    };
    out.extend(find.map(|action| UiEvent::Input(Input::Find(action))));
}

/// The index verdict of the current run in its fixed-size area (reserved whenever explain is
/// on, so nothing shifts); its copy button goes to the clipboard here, and Create index... goes
/// to the reducer, which opens the index builder.
fn show_verdict(ui: &mut egui::Ui, state: &AppState, i18n: &I18n) -> Option<FindAction> {
    if !state.explain_enabled() {
        return None;
    }
    let model = badge(&super::explain_actions::shown(state), state.selected_ns());
    match show_verdict_row(ui, model.as_ref(), create_index_state(state), i18n) {
        Some(VerdictAction::CopySuggestion(text)) => {
            ui.ctx().copy_text(text);
            None
        }
        Some(VerdictAction::CreateIndex) => Some(FindAction::CreateSuggestedIndex),
        None => None,
    }
}

/// The Aggregate tab of the active session: its draft, drawn with the tab's own view state (per
/// connection, like the Indexes tab's egui memory). When the tab is being left (`leaving`), the
/// typing that still waits is sent too.
fn show_aggregate_tab(
    ui: &mut egui::Ui,
    state: &mut AppState,
    leaving: bool,
    i18n: &I18n,
    out: &mut Vec<UiEvent>,
) {
    let saved = state
        .active
        .map_or(&[][..], |connection| state.pipelines.list(connection));
    let run = RunContext {
        availability: state.run_availability(),
        badge: state.active_badge(),
        read_only: state.active_is_read_only(),
        disconnected: state.status != ConnStatus::Connected,
        ns: state.selected_ns().cloned(),
    };
    let mut actions = ui
        .push_id(("aggregate_tab", state.active), |ui| {
            let view = &mut state.aggregate_view;
            show_aggregate(ui, &state.aggregate, view, saved, &run, i18n)
        })
        .inner;
    if leaving {
        actions.extend(flush_typing(&mut state.aggregate_view));
    }
    // One input for the whole frame, so the frame sends at most one preview.
    out.extend(
        AggregateAction::batched(actions).map(|action| UiEvent::Input(Input::Aggregate(action))),
    );
}

/// The Indexes tab; "copy as JSON" goes to the clipboard here (`IndexesAction::clipboard_text`),
/// the reducer needs no record of it.
fn show_indexes_tab(
    ui: &mut egui::Ui,
    state: &AppState,
    i18n: &I18n,
    out: &mut Vec<UiEvent>,
    now_secs: i64,
) {
    // Per connection: what the tab keeps in egui's memory (the Sampling section's open state)
    // is not shared with the same collection on another connection.
    let shown = ui
        .push_id(("indexes_tab", state.active), |ui| {
            show_indexes(
                ui,
                &state.indexes,
                create_index_state(state),
                i18n,
                now_secs,
                &state.sampling,
            )
        })
        .inner;
    let Some(action) = shown else {
        return;
    };
    if let Some(text) = action.clipboard_text() {
        ui.ctx().copy_text(text.to_string());
    }
    out.push(UiEvent::Input(Input::Indexes(action)));
}
