//! The consent step before the profiler is switched: what the new level does, the exact command
//! the app will run (monospace, with a copy button) and Cancel / Confirm. Cancel is the focused
//! default, so a stray Enter changes nothing; Escape and a click away cancel. Confirm is off
//! while a change is in flight or the connection is read-only, with the reason in plain text.
use super::index_dialogs::focus_once;
use crate::I18n;
use crate::viewmodel::connection_tag::READ_ONLY_REASON_KEY;
use crate::viewmodel::{PROFILER_LEVEL_ON, ProfilerConsent, QueriesAction};
use eframe::egui;

const MODAL_WIDTH: f32 = 520.0;

/// Draws the dialog for `consent`; `production` adds the notice every guarded write carries.
pub fn show_profiler_consent(
    ctx: &egui::Context,
    consent: &ProfilerConsent,
    busy: bool,
    read_only: bool,
    production: bool,
    i18n: &I18n,
) -> Option<QueriesAction> {
    let turning_on = consent.level == PROFILER_LEVEL_ON;
    let mut action = None;
    let modal = egui::Modal::new(egui::Id::new("profiler_consent")).show(ctx, |ui| {
        ui.set_width(MODAL_WIDTH);
        let (title, message) = match turning_on {
            true => (
                "indexes.queries.consent.title_on",
                "indexes.queries.consent.message_on",
            ),
            false => (
                "indexes.queries.consent.title_off",
                "indexes.queries.consent.message_off",
            ),
        };
        ui.heading(i18n.t(title));
        ui.label(i18n.t_with(message, &[("db", &consent.db)]));
        if production {
            ui.label(
                egui::RichText::new(i18n.t("write_warning.production_line"))
                    .strong()
                    .color(ui.visuals().error_fg_color),
            );
        }
        show_command(ui, &consent.command_text(), i18n);
        if read_only {
            ui.label(i18n.t(READ_ONLY_REASON_KEY));
        } else if busy {
            ui.label(i18n.t("indexes.queries.busy"));
        }
        action = buttons(ui, consent, turning_on, busy || read_only, busy, i18n);
    });
    let dismissed = modal.should_close() && !busy;
    action.or(dismissed.then_some(QueriesAction::CancelEnable))
}

fn show_command(ui: &mut egui::Ui, command: &str, i18n: &I18n) {
    ui.label(i18n.t("indexes.queries.consent.command"));
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(command).monospace());
        if ui.button(i18n.t("indexes.queries.consent.copy")).clicked() {
            ui.ctx().copy_text(command.to_string());
        }
    });
}

fn buttons(
    ui: &mut egui::Ui,
    consent: &ProfilerConsent,
    turning_on: bool,
    confirm_off: bool,
    busy: bool,
    i18n: &I18n,
) -> Option<QueriesAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let cancel = ui.add_enabled(!busy, egui::Button::new(i18n.t("common.cancel")));
        // Cancel owns the first focus, so a stray Enter changes nothing.
        focus_once(ui, &cancel, ("profiler_consent_focus", &consent.db));
        let (label, confirmed) = match turning_on {
            true => (
                "indexes.queries.consent.confirm_on",
                QueriesAction::ConfirmEnable,
            ),
            false => (
                "indexes.queries.consent.confirm_off",
                QueriesAction::ConfirmDisable,
            ),
        };
        let confirm = ui.add_enabled(!confirm_off, egui::Button::new(i18n.t(label)));
        if cancel.clicked() {
            action = Some(QueriesAction::CancelEnable);
        }
        if confirm.clicked() {
            action = Some(confirmed);
        }
    });
    action
}
