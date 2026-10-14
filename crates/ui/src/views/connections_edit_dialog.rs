//! The connection edit dialog: name, folder, From URI, the form body, Test connection,
//! To URI, the validation error and the Cancel / Save footer.
use super::connections::{ConnectionAction, ConnectionsState, TestState};
use super::connections_edit::{show_fields, text_row};
use super::error_label::show_error;
use crate::I18n;
use curlybrace_core::runtime::TestReport;
use eframe::egui;

const DIALOG_WIDTH: f32 = 520.0;
const BODY_MAX_HEIGHT: f32 = 460.0;

/// Draws the dialog while `state.editing`. Cancel, Escape and a click outside close it (and
/// reset the tools); Save returns the action once the form validates.
pub(super) fn show_edit_dialog(
    ctx: &egui::Context,
    state: &mut ConnectionsState,
    i18n: &I18n,
) -> Option<ConnectionAction> {
    if !state.editing {
        return None;
    }
    let mut action = None;
    let mut cancelled = false;
    let modal = egui::Modal::new(egui::Id::new("connection_edit")).show(ctx, |ui| {
        ui.set_width(DIALOG_WIDTH);
        ui.heading(i18n.t("connections.edit_title"));
        egui::ScrollArea::vertical()
            .max_height(BODY_MAX_HEIGHT)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                action = show_body(ui, state, i18n);
            });
        // Outside the clipped scroll area, so a long result or error is always fully visible.
        show_test_result(ui, state, i18n);
        if let Some(error) = &state.error {
            show_error(ui, error, i18n);
        }
        if let Some((headline, details)) = state.dialog_banner.split_first() {
            ui.colored_label(ui.visuals().error_fg_color, headline);
            for detail in details {
                ui.weak(detail);
            }
        }
        ui.separator();
        ui.horizontal(|ui| {
            cancelled = ui.button(i18n.t("common.cancel")).clicked();
            if ui.button(i18n.t("common.save")).clicked() {
                action = save_action(state);
            }
        });
    });
    if cancelled || modal.should_close() {
        state.close_edit();
        return None;
    }
    action
}

fn show_body(
    ui: &mut egui::Ui,
    state: &mut ConnectionsState,
    i18n: &I18n,
) -> Option<ConnectionAction> {
    text_row(ui, &i18n.t("connections.name"), &mut state.form.name, false);
    show_folder_row(ui, state, i18n);
    show_from_uri(ui, state, i18n);
    show_fields(ui, state, i18n);
    ui.separator();
    let tested = show_test(ui, state, i18n);
    let built = show_to_uri(ui, state, i18n);
    tested.or(built)
}

/// Validates the form; a failure stays in `state.error` and nothing is emitted.
fn save_action(state: &mut ConnectionsState) -> Option<ConnectionAction> {
    match state.form.to_config() {
        Ok(_) => {
            state.error = None;
            Some(ConnectionAction::Save(Box::new(state.form.clone())))
        }
        Err(error) => {
            state.error = Some(error);
            None
        }
    }
}

/// Free text for the folder plus a drop-down of the folders already in use that fills it.
fn show_folder_row(ui: &mut egui::Ui, state: &mut ConnectionsState, i18n: &I18n) {
    let names = state.folder_names();
    ui.horizontal(|ui| {
        let caption = ui.label(i18n.t("connections.folder"));
        ui.add(egui::TextEdit::singleline(&mut state.form.folder))
            .labelled_by(caption.id);
        if names.is_empty() {
            return;
        }
        let combo = egui::ComboBox::from_id_salt("connection_folder")
            .selected_text("")
            .show_ui(ui, |ui| {
                for name in &names {
                    if ui
                        .selectable_label(state.form.folder == *name, name)
                        .clicked()
                    {
                        state.form.folder.clone_from(name);
                    }
                }
            });
        // egui names a label-less combo ""; give it the caption text (and the relation).
        let name = i18n.t("connections.folder");
        combo.response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, name.clone())
        });
        combo.response.labelled_by(caption.id);
    });
}

fn show_from_uri(ui: &mut egui::Ui, state: &mut ConnectionsState, i18n: &I18n) {
    ui.horizontal(|ui| {
        let caption = ui.label(i18n.t("connections.from_uri_hint"));
        ui.add(egui::TextEdit::singleline(&mut state.uri_input))
            .labelled_by(caption.id);
        if ui.button(i18n.t("connections.from_uri")).clicked() {
            state.apply_uri_input();
        }
    });
}

fn show_test(
    ui: &mut egui::Ui,
    state: &mut ConnectionsState,
    i18n: &I18n,
) -> Option<ConnectionAction> {
    let clicked = ui.button(i18n.t("connections.test")).clicked();
    clicked.then(|| ConnectionAction::Test(Box::new(state.form.clone())))
}

/// The outcome of Test connection, below the scrolling body.
fn show_test_result(ui: &mut egui::Ui, state: &ConnectionsState, i18n: &I18n) {
    match &state.test {
        TestState::Idle => {}
        TestState::Running { .. } => {
            ui.label(i18n.t("connections.test_running"));
        }
        TestState::Done {
            outcome: Ok(report),
            ..
        } => show_report(ui, report, i18n),
        TestState::Done {
            outcome: Err(error),
            ..
        } => show_error(ui, error, i18n),
    }
}

/// `tls` and `authenticated` say what is configured, never that it was verified.
fn show_report(ui: &mut egui::Ui, report: &TestReport, i18n: &I18n) {
    let text = match &report.server_version {
        Some(version) => i18n.t_with("connections.test_ok", &[("version", version)]),
        None => i18n.t("connections.test_ok_no_version"),
    };
    ui.label(text);
    if report.tls {
        ui.label(i18n.t("connections.test_configured_tls"));
    }
    if report.authenticated {
        ui.label(i18n.t("connections.test_configured_auth"));
    }
}

fn show_to_uri(
    ui: &mut egui::Ui,
    state: &mut ConnectionsState,
    i18n: &I18n,
) -> Option<ConnectionAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let build = |include_password| ConnectionAction::BuildUri {
            form: Box::new(state.form.clone()),
            include_password,
        };
        if ui.button(i18n.t("connections.to_uri")).clicked() {
            action = Some(build(false));
        }
        if ui.button(i18n.t("connections.include_password")).clicked() {
            action = Some(build(true));
        }
    });
    show_built_uri(ui, state, i18n);
    action
}

/// The built string in a read-only box with a Copy button. It may hold the real password, so
/// it goes nowhere else (no label text, no log).
fn show_built_uri(ui: &mut egui::Ui, state: &ConnectionsState, i18n: &I18n) {
    let Some(built) = &state.built_uri else {
        return;
    };
    ui.horizontal(|ui| {
        let caption = ui.label(i18n.t("connections.built_uri"));
        // A `&str` buffer is read-only but still selectable.
        let mut shown: &str = built.expose();
        ui.add(egui::TextEdit::singleline(&mut shown))
            .labelled_by(caption.id);
        if ui.button(i18n.t("connections.copy")).clicked() {
            ui.ctx().copy_text(built.expose().to_owned());
        }
    });
    if state.built_with_password {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            i18n.t("connections.uri_password_warning"),
        );
    }
}
