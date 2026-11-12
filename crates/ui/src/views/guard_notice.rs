//! The index-miss guard's notice under the find bar. While the plan is asked for, a quiet status
//! line without buttons; when the query would scan a collection (or sort in memory), the problem,
//! its reasons, the suggested index with its copy button, and Run anyway / Cancel. Cancel takes
//! the first focus, so a stray Enter never runs the query the notice just warned about.
use super::index_dialogs::focus_once;
use super::verdict_badge::{VerdictAction, reason_line, show_suggestion};
use crate::I18n;
use crate::app::GuardState;
use crate::theme::{Tone, current_ui, filled_button, shape, tonal_button};
use crate::viewmodel::guard_notice::{GuardNotice, WarnModel, guard_notice};
use eframe::egui::{self, accesskit};

/// What the notice asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum GuardNoticeAction {
    /// Send the held-back find (`FindAction::GuardRunAnyway`).
    RunAnyway,
    /// Drop the held-back find (`FindAction::GuardCancel`).
    Cancel,
    /// Put the suggested `createIndex` command on the clipboard.
    Copy(String),
}

/// Draws the notice for `guard` (nothing for no guard state).
pub fn show_guard_notice(
    ui: &mut egui::Ui,
    guard: &GuardState,
    i18n: &I18n,
) -> Option<GuardNoticeAction> {
    match guard_notice(guard)? {
        GuardNotice::Checking => {
            show_checking(ui, i18n);
            None
        }
        GuardNotice::Warn(model) => show_warning(ui, &model, i18n),
    }
}

fn show_checking(ui: &mut egui::Ui, i18n: &I18n) {
    let text = i18n.t("query.guard.checking");
    announced(ui, Tone::Neutral, Announce::Status, &text, |ui| {
        on_container(ui, Tone::Neutral, |ui| {
            ui.label(&text);
        });
    });
}

/// `content` with its text in the tone's `on_container` color, for the words that sit on the
/// notice's container. The buttons are drawn outside it: they bring their own colors.
pub(super) fn on_container<R>(
    ui: &mut egui::Ui,
    tone: Tone,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let color = current_ui(ui).tone(tone).on_container;
    ui.scope(|ui| {
        ui.visuals_mut().override_text_color = Some(color);
        content(ui)
    })
    .inner
}

fn show_warning(ui: &mut egui::Ui, model: &WarnModel, i18n: &I18n) -> Option<GuardNoticeAction> {
    let title = format!("{} {}", model.icon, i18n.t(model.title_key));
    announced(ui, Tone::Warning, Announce::Alert, &title, |ui| {
        let copy = on_container(ui, Tone::Warning, |ui| {
            ui.strong(&title);
            for reason in &model.reasons {
                ui.label(reason_line(reason.key, &reason.params, i18n));
            }
            model
                .suggestion
                .as_deref()
                .and_then(|command| show_suggestion(ui, command, i18n))
                .map(|VerdictAction::CopySuggestion(command)| GuardNoticeAction::Copy(command))
        });
        buttons(ui, i18n).or(copy)
    })
}

/// Run anyway and Cancel. Cancel is the one with the first focus and the filled look.
fn buttons(ui: &mut egui::Ui, i18n: &I18n) -> Option<GuardNoticeAction> {
    ui.horizontal(|ui| {
        let anyway = tonal_button(ui, i18n.t("query.guard.run_anyway"));
        let cancel = filled_button(ui, i18n.t("query.guard.cancel"));
        focus_once(ui, &cancel, ("guard_notice_focus", "cancel"));
        if cancel.clicked() {
            Some(GuardNoticeAction::Cancel)
        } else if anyway.clicked() {
            Some(GuardNoticeAction::RunAnyway)
        } else {
            None
        }
    })
    .inner
}

/// How a screen reader is told about the notice.
#[derive(Clone, Copy)]
pub(super) enum Announce {
    /// Said when the reader is idle (the check running).
    Status,
    /// Said at once (the query was held back and waits for an answer).
    Alert,
}

/// `content` on the container of `tone`, as one live region named `label` whose children are
/// the notice's own widgets.
pub(super) fn announced<R>(
    ui: &mut egui::Ui,
    tone: Tone,
    announce: Announce,
    label: &str,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let region = ui.id().with("guard_notice");
    let (role, live) = match announce {
        Announce::Status => (accesskit::Role::Status, accesskit::Live::Polite),
        Announce::Alert => (accesskit::Role::Alert, accesskit::Live::Assertive),
    };
    let ctx = ui.ctx().clone();
    ctx.accesskit_node_builder(region, |node| {
        node.set_role(role);
        node.set_live(live);
        node.set_label(label);
    });
    let colors = current_ui(ui).tone(tone);
    ctx.with_accessibility_parent(region, || {
        egui::Frame::new()
            .fill(colors.container)
            .corner_radius(shape::radius(shape::MEDIUM))
            .inner_margin(egui::Margin::symmetric(
                shape::SPACE_3 as i8,
                shape::SPACE_2 as i8,
            ))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                content(ui)
            })
            .inner
    })
}
