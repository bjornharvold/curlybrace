//! The connection edit dialog: name, folder, From URI, the form body, Test connection,
//! To URI, the validation error and the Cancel / Save footer.
use super::connections::{ConnectionAction, ConnectionsState, TestState};
use super::connections_edit::{show_fields, text_row};
use super::error_label::show_error;
use crate::I18n;
use crate::app::tools_state::ToolsState;
use crate::theme::widgets::CONTROL_HEIGHT;
use crate::theme::{
    FieldOptions, Tone, chip, filled_button, modal_frame, outlined_field_opts, shape, text_button,
    tonal_button, typography,
};
use curlybrace_core::runtime::TestReport;
use eframe::egui;

const DIALOG_WIDTH: f32 = 720.0;
const BODY_MAX_HEIGHT: f32 = 460.0;
/// What the dialog needs besides the scrolling body (title, result lines, footer, padding), so
/// the body shrinks on a short window and the footer stays on screen.
const CHROME_HEIGHT: f32 = 320.0;
const BODY_MIN_HEIGHT: f32 = 160.0;
/// Room the folder drop-down keeps beside the folder field.
const FOLDER_COMBO_WIDTH: f32 = 160.0;
/// Room a Copy button keeps beside the built string.
const COPY_BUTTON_WIDTH: f32 = 96.0;
/// Height of a row holding an outlined field: the control plus its floating label's overhang.
const CONTROL_ROW_HEIGHT: f32 = CONTROL_HEIGHT + shape::SPACE_2;

/// Draws the dialog while `state.editing`. Cancel and Escape close it (and reset the tools); a
/// click outside does NOT, so a stray click never discards a typed form. Save returns the
/// action once the form validates.
///
/// Returns what was done, in order. A tool path typed just before Save (or Cancel) is applied in
/// the frame its box loses focus, so it comes first and Save never drops it. Tool paths are kept
/// apart from the form and apply at once, so Cancel keeps them too.
pub(super) fn show_edit_dialog(
    ctx: &egui::Context,
    state: &mut ConnectionsState,
    tools: &ToolsState,
    i18n: &I18n,
) -> Vec<ConnectionAction> {
    if !state.editing {
        return Vec::new();
    }
    let mut action = None;
    let mut tool_action = None;
    let mut cancelled = false;
    let modal = egui::Modal::new(egui::Id::new("connection_edit"))
        .frame(modal_frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(DIALOG_WIDTH);
            ui.label(typography::dialog_title(
                ui,
                i18n.t(state.edit_mode.title_key()),
            ));
            egui::ScrollArea::vertical()
                .max_height(body_height(ctx))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    (action, tool_action) = show_body(ui, state, tools, i18n);
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
                cancelled = text_button(ui, i18n.t("common.cancel")).clicked();
                if filled_button(ui, i18n.t("common.save")).clicked() {
                    action = save_action(state);
                }
            });
        });
    let closing = cancelled || closed_by_escape(&modal);
    if closing {
        state.close_edit();
    }
    let mut actions: Vec<ConnectionAction> = tool_action.into_iter().collect();
    if !closing {
        actions.extend(action);
    }
    actions
}

/// The body's height: [`BODY_MAX_HEIGHT`], less on a window too short for it.
fn body_height(ctx: &egui::Context) -> f32 {
    (ctx.screen_rect().height() - CHROME_HEIGHT).clamp(BODY_MIN_HEIGHT, BODY_MAX_HEIGHT)
}

/// `Modal::should_close` also fires on a backdrop click; only Escape (with no popup open, and
/// this being the top modal) or a close request from inside the dialog may close it.
fn closed_by_escape<T>(modal: &egui::ModalResponse<T>) -> bool {
    let ctx = &modal.response.ctx;
    modal.response.should_close()
        || (modal.is_top_modal
            && !modal.any_popup_open
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)))
}

fn show_body(
    ui: &mut egui::Ui,
    state: &mut ConnectionsState,
    tools: &ToolsState,
    i18n: &I18n,
) -> (Option<ConnectionAction>, Option<ConnectionAction>) {
    text_row(ui, &i18n.t("connections.name"), &mut state.form.name, false);
    show_folder_row(ui, state, i18n);
    super::connections_from_uri::show_from_uri(ui, state, i18n);
    show_fields(ui, state, i18n);
    ui.separator();
    let tool_paths = super::connections_edit_tools::show_tools_section(ui, state, tools, i18n);
    ui.separator();
    let tested = show_test(ui, state, i18n);
    let built = show_to_uri(ui, state, i18n);
    (tested.or(built), tool_paths)
}

/// Validates the form; a failure stays in `state.error` and nothing is emitted.
fn save_action(state: &mut ConnectionsState) -> Option<ConnectionAction> {
    match state.form.to_checked_config() {
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
    let label = i18n.t("connections.folder");
    ui.horizontal(|ui| {
        let reserved = if names.is_empty() {
            0.0
        } else {
            FOLDER_COMBO_WIDTH
        };
        let width = (ui.available_width() - reserved - ui.spacing().item_spacing.x).max(120.0);
        let height = CONTROL_ROW_HEIGHT;
        ui.allocate_ui_with_layout(
            egui::vec2(width, height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                outlined_field_opts(
                    ui,
                    &label,
                    &mut state.form.folder,
                    "",
                    FieldOptions::default(),
                )
            },
        );
        if names.is_empty() {
            return;
        }
        let selected = if state.form.folder.trim().is_empty() {
            i18n.t("connections.no_folder")
        } else {
            state.form.folder.clone()
        };
        let combo = egui::ComboBox::from_id_salt("connection_folder")
            .selected_text(selected.clone())
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
        // egui names a label-less combo ""; give it the field's label, and the shown folder as
        // its value (egui reads only `current_text_value` into AccessKit).
        combo.response.widget_info(|| {
            let mut info =
                egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, label.clone());
            info.current_text_value = Some(selected.clone());
            info
        });
    });
}

fn show_test(
    ui: &mut egui::Ui,
    state: &mut ConnectionsState,
    i18n: &I18n,
) -> Option<ConnectionAction> {
    let clicked = tonal_button(ui, i18n.t("connections.test")).clicked();
    clicked.then(|| ConnectionAction::Test(Box::new(state.form.clone())))
}

/// The outcome of Test connection, below the scrolling body.
fn show_test_result(ui: &mut egui::Ui, state: &ConnectionsState, i18n: &I18n) {
    match &state.test {
        TestState::Idle => {}
        TestState::Running { .. } => {
            chip(ui, Tone::Neutral, &i18n.t("connections.test_running"));
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
    chip(ui, Tone::Good, &text);
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
        if text_button(ui, i18n.t("connections.to_uri")).clicked() {
            action = Some(build(false));
        }
        if text_button(ui, i18n.t("connections.include_password")).clicked() {
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
        let width = (ui.available_width() - COPY_BUTTON_WIDTH).max(120.0);
        // A `&str` buffer is read-only but still selectable.
        let mut shown: &str = built.expose();
        let label = i18n.t("connections.built_uri");
        ui.allocate_ui_with_layout(
            egui::vec2(width, CONTROL_ROW_HEIGHT),
            egui::Layout::top_down(egui::Align::Min),
            |ui| outlined_field_opts(ui, &label, &mut shown, "", FieldOptions::default()),
        );
        if text_button(ui, i18n.t("connections.copy")).clicked() {
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
