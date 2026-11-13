//! The Browse page of a collection, top to bottom: one row with the connection header and the
//! Documents | Indexes tabs; the find bar; one wrapping row with the two global switches and
//! the index verdict; the guard's notice (only while it has something to say); then the
//! documents with their one-row toolbar. Kept dense so the data starts high on the window.
use super::panels::UiEvent;
use super::reducer::Input;
use super::state::AppState;
use crate::I18n;
use crate::viewmodel::verdict::badge;
use crate::viewmodel::{CollectionTab, copy_blocked};
use crate::views::collection_tabs::show_collection_tabs;
use crate::views::documents::{
    DocTarget, show_connection_header_with, show_documents_pending, show_documents_switchable,
};
use crate::views::findbar::{FindAction, show_automatic_verdict, show_findbar, show_guard_setting};
use crate::views::guard_notice::{GuardNoticeAction, show_guard_notice};
use crate::views::indexes::show_indexes;
use crate::views::verdict_badge::{VerdictAction, show_verdict_row};
use eframe::egui;

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
    let connection = state.active_connection_label(i18n);
    let badge = state.active_badge();
    let picked = show_connection_header_with(ui, &connection, &badge, i18n, |ui| {
        show_collection_tabs(ui, state.tab, i18n)
    });
    if let Some(tab) = picked {
        out.push(UiEvent::Input(Input::Tab(tab)));
    }
    if state.tab == CollectionTab::Indexes {
        show_indexes_tab(ui, state, badge.read_only, i18n, out, now_secs);
        return;
    }
    let (has_prev, has_next) = state.paging();
    if let Some(action) = show_findbar(ui, &mut state.find, has_prev, has_next, i18n) {
        out.push(UiEvent::Input(Input::Find(action)));
    }
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
            let switches = [
                show_automatic_verdict(ui, state.explain_enabled(), i18n),
                show_guard_setting(ui, state.guard_enabled(), i18n),
            ];
            show_verdict(ui, state, i18n);
            switches
        })
        .inner;
    out.extend(
        actions
            .into_iter()
            .flatten()
            .map(|action| UiEvent::Input(Input::Find(action))),
    );
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
/// on, so nothing shifts); its copy button goes to the clipboard here.
fn show_verdict(ui: &mut egui::Ui, state: &AppState, i18n: &I18n) {
    if !state.explain_enabled() {
        return;
    }
    let model = badge(&super::explain_actions::shown(state), state.selected_ns());
    if let Some(VerdictAction::CopySuggestion(text)) = show_verdict_row(ui, model.as_ref(), i18n) {
        ui.ctx().copy_text(text);
    }
}

/// The Indexes tab; "copy as JSON" goes to the clipboard here (`IndexesAction::clipboard_text`),
/// the reducer needs no record of it.
fn show_indexes_tab(
    ui: &mut egui::Ui,
    state: &AppState,
    read_only: bool,
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
                read_only,
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
