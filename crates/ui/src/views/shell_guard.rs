//! The Shell tab's index-miss guard in the view: the quiet "checking" line beside the toolbar
//! while the script's queries are planned, and the notice that lists the queries that would scan
//! a collection or sort in memory, with Run anyway and Cancel. The buttons share the title's row
//! and the findings scroll in a short area, so a long list never takes the output pane's room.
//! Cancel takes the first focus, so a stray Enter never runs the script the notice warned about.
use super::guard_notice::{Announce, GuardNoticeAction, announced, on_container};
use super::index_dialogs::focus_once;
use super::verdict_badge::{VerdictAction, reason_line, show_suggestion};
use crate::I18n;
use crate::theme::{Tone, filled_button, tonal_button};
use crate::viewmodel::shell_guard_notice::{
    FindingModel, IndexMissModel, index_miss_model, unchecked_key,
};
use curlybrace_core::guard::GuardReport;
use eframe::egui::{self, accesskit};

/// The window height (points) up to which the findings area keeps its minimum.
const SMALL_WINDOW: f32 = 500.0;
/// The findings area is one row tall in a small window (points); more scroll.
const FINDINGS_MIN_HEIGHT: f32 = 24.0;
/// ... and grows by this share of the window height above [`SMALL_WINDOW`] ...
const FINDINGS_GROWTH: f32 = 0.4;
/// ... up to this (points).
const FINDINGS_MAX_HEIGHT: f32 = 160.0;

/// The line beside Run and Cancel while the queries are checked: a status for a screen reader,
/// not a running script.
pub fn show_checking(ui: &mut egui::Ui, i18n: &I18n) {
    let text = i18n.t("shell.guard.checking");
    let response = ui.label(&text);
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_role(accesskit::Role::Status);
        node.set_live(accesskit::Live::Polite);
        node.set_label(text.as_str());
    });
}

/// Draws the notice for `report`; `unchecked` is the total of calls that could not be checked.
pub fn show_index_miss(
    ui: &mut egui::Ui,
    report: &GuardReport,
    unchecked: usize,
    i18n: &I18n,
) -> Option<GuardNoticeAction> {
    let model = index_miss_model(report, unchecked);
    let title = i18n.t("shell.guard.title");
    announced(ui, Tone::Warning, Announce::Alert, &title, |ui| {
        // Snug rows: the notice shares the tab with the editor, History and the output.
        ui.spacing_mut().item_spacing.y = 0.0;
        let buttons = ui
            .horizontal_wrapped(|ui| {
                on_container(ui, Tone::Warning, |ui| ui.strong(&title));
                buttons(ui, i18n)
            })
            .inner;
        let copy = findings(ui, &model, i18n);
        buttons.or(copy)
    })
}

fn findings(ui: &mut egui::Ui, model: &IndexMissModel, i18n: &I18n) -> Option<GuardNoticeAction> {
    on_container(ui, Tone::Warning, |ui| {
        let height = findings_height(ui.ctx().screen_rect().height());
        egui::ScrollArea::vertical()
            .id_salt("shell_guard_findings")
            .min_scrolled_height(height)
            .max_height(height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                // Every finding is drawn, whatever a click in this frame did.
                let copies: Vec<_> = model
                    .findings
                    .iter()
                    .map(|finding| show_finding(ui, finding, i18n))
                    .collect();
                if model.unchecked > 0 {
                    let count = model.unchecked.to_string();
                    let key = unchecked_key(model.unchecked);
                    ui.label(i18n.t_with(key, &[("count", &count)]));
                }
                copies.into_iter().flatten().next()
            })
            .inner
    })
}

/// How tall the findings area is in a window `window` points tall: one row in a small window,
/// where the output pane needs the room (the area scrolls), more as the window grows.
fn findings_height(window: f32) -> f32 {
    let spare = (window - SMALL_WINDOW).max(0.0);
    (FINDINGS_MIN_HEIGHT + spare * FINDINGS_GROWTH).min(FINDINGS_MAX_HEIGHT)
}

/// One finding: the collection and the problem, the reasons, the suggested index with Copy.
fn show_finding(
    ui: &mut egui::Ui,
    finding: &FindingModel,
    i18n: &I18n,
) -> Option<GuardNoticeAction> {
    let problem = i18n.t(finding.warn.title_key);
    let line = i18n.t_with(
        "shell.guard.finding",
        &[
            ("db", &finding.db),
            ("collection", &finding.collection),
            ("problem", &problem),
        ],
    );
    ui.label(egui::RichText::new(format!("{} {line}", finding.warn.icon)).strong());
    for reason in &finding.warn.reasons {
        ui.label(reason_line(reason.key, &reason.params, i18n));
    }
    finding
        .warn
        .suggestion
        .as_deref()
        .and_then(|command| show_suggestion(ui, command, i18n))
        .map(|VerdictAction::CopySuggestion(command)| GuardNoticeAction::Copy(command))
}

/// Run anyway and Cancel. Cancel is the one with the first focus and the filled look.
fn buttons(ui: &mut egui::Ui, i18n: &I18n) -> Option<GuardNoticeAction> {
    let anyway = tonal_button(ui, i18n.t("shell.guard.run_anyway"));
    let cancel = filled_button(ui, i18n.t("shell.guard.cancel"));
    focus_once(ui, &cancel, ("shell_guard_focus", "cancel"));
    if cancel.clicked() {
        Some(GuardNoticeAction::Cancel)
    } else if anyway.clicked() {
        Some(GuardNoticeAction::RunAnyway)
    } else {
        None
    }
}
