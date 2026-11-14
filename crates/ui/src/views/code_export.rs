//! The Copy as code dialog: the language picker, the setup and write-stage options, the three
//! variable names and the generated code in a read-only, selectable box that scrolls, with Copy
//! and Close always in view below it. An unusable variable name shows its message in place of the
//! code and turns Copy off. Enter copies, Escape closes, focus starts on the language picker.
//! Draws what it is given and returns what the person did; Copy puts exactly the previewed text
//! on the clipboard here, the reducer keeps the dialog's state.
use super::code_export_text::{error_text, language_label_key, name_label_key};
use super::index_dialogs::focus_once;
use super::tone_notice::tone_notice;
use crate::I18n;
use crate::app::CodeAction;
use crate::theme::{
    CONTROL_HEIGHT, Tone, filled_button, gate, modal_frame, text_button, typography,
};
use crate::viewmodel::code_export::{CodeDialog, CodeExportUi, NameField};
use curlybrace_core::codegen::Language;
use eframe::egui::{self, WidgetInfo, WidgetType};

const MODAL_WIDTH: f32 = 720.0;
/// What the dialog keeps for everything but the code: title, rows, buttons and the frame.
const MODAL_CHROME: f32 = 380.0;
const MIN_CODE_HEIGHT: f32 = 90.0;
/// The room kept between the dialog and the window's side edges.
const SIDE_ROOM: f32 = 80.0;
const NAME_FIELDS: [NameField; 3] = [NameField::Client, NameField::Db, NameField::Collection];

/// The dialog, if one is open; the actions of this frame in order.
pub fn show_code_dialog(ctx: &egui::Context, code: &CodeExportUi, i18n: &I18n) -> Vec<CodeAction> {
    let Some(dialog) = &code.dialog else {
        return Vec::new();
    };
    let mut actions = Vec::new();
    let screen = ctx.screen_rect();
    let width = MODAL_WIDTH.min(screen.width() - SIDE_ROOM);
    let modal = egui::Modal::new(egui::Id::new("code_export_dialog"))
        .frame(modal_frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(width);
            ui.label(typography::dialog_title(ui, i18n.t("codegen.title")));
            ui.label(i18n.t("codegen.note"));
            let mut own_enter = options_rows(ui, dialog, &mut actions, i18n);
            let max_height = (screen.height() - MODAL_CHROME).max(MIN_CODE_HEIGHT);
            show_code(ui, dialog, max_height, i18n);
            let controls = buttons(ui, dialog, i18n);
            actions.extend(controls.actions);
            own_enter.extend(controls.activating);
            let ready = dialog.clipboard_text().is_some();
            if ready && enter_copies(ui, &own_enter) && !actions.contains(&CodeAction::Close) {
                actions.push(CodeAction::Copy);
            }
        });
    if modal.should_close() && !actions.contains(&CodeAction::Close) {
        actions.push(CodeAction::Close);
    }
    put_on_clipboard(ctx, dialog, &actions);
    dedupe_copy(actions)
}

/// What the buttons row did, and the buttons Enter already activates.
struct Controls {
    actions: Vec<CodeAction>,
    activating: Vec<egui::Id>,
}

/// The language picker and options row, then the three names. Returns the widgets that act on
/// Enter themselves.
fn options_rows(
    ui: &mut egui::Ui,
    dialog: &CodeDialog,
    actions: &mut Vec<CodeAction>,
    i18n: &I18n,
) -> Vec<egui::Id> {
    let mut activating = Vec::new();
    ui.horizontal_wrapped(|ui| {
        ui.set_min_height(CONTROL_HEIGHT);
        let combo = language_picker(ui, dialog.language, actions, i18n);
        focus_once(ui, &combo, ("code_export_focus", "language"));
        activating.push(combo.id);
        let mut setup = dialog.options.setup;
        let setup_box = ui.checkbox(&mut setup, i18n.t("codegen.setup"));
        activating.push(setup_box.id);
        if setup_box.changed() {
            actions.push(CodeAction::SetSetup(setup));
        }
        if dialog.is_aggregate() {
            let mut keep = dialog.options.write_stage;
            let write_box = ui.checkbox(&mut keep, i18n.t("codegen.write_stage"));
            activating.push(write_box.id);
            if write_box.changed() {
                actions.push(CodeAction::SetWriteStage(keep));
            }
        }
    });
    // The three fields share the row equally and start at the same height.
    let gaps = ui.spacing().item_spacing.x * (NAME_FIELDS.len() - 1) as f32;
    let field_width = ((ui.available_width() - gaps) / NAME_FIELDS.len() as f32).floor();
    ui.horizontal_top(|ui| {
        for field in NAME_FIELDS {
            let mut name = name_of(dialog, field).to_string();
            let changed = ui
                .allocate_ui(egui::vec2(field_width, 0.0), |ui| {
                    let label = i18n.t(name_label_key(field));
                    crate::theme::outlined_field(ui, &label, &mut name, "").changed()
                })
                .inner;
            if changed {
                actions.push(CodeAction::SetName(field, name));
            }
        }
    });
    activating
}

fn name_of(dialog: &CodeDialog, field: NameField) -> &str {
    match field {
        NameField::Client => &dialog.options.client_var,
        NameField::Db => &dialog.options.db_var,
        NameField::Collection => &dialog.options.collection_var,
    }
}

/// The language drop-down, named for assistive technology; the language picked adds an action.
fn language_picker(
    ui: &mut egui::Ui,
    current: Language,
    actions: &mut Vec<CodeAction>,
    i18n: &I18n,
) -> egui::Response {
    let label = i18n.t("codegen.language.label");
    let shown = i18n.t(language_label_key(current));
    ui.label(label.clone());
    ui.spacing_mut().interact_size.y = ui.spacing().interact_size.y.max(CONTROL_HEIGHT);
    let combo = egui::ComboBox::from_id_salt("code_export_language")
        .selected_text(shown.clone())
        .show_ui(ui, |ui| {
            for language in Language::ALL {
                let name = i18n.t(language_label_key(language));
                if ui.selectable_label(language == current, name).clicked() {
                    actions.push(CodeAction::SetLanguage(language));
                }
            }
        });
    combo.response.widget_info(|| {
        let mut info = WidgetInfo::labeled(WidgetType::ComboBox, true, label.clone());
        info.current_text_value = Some(shown.clone());
        info
    });
    combo.response
}

/// The code in a read-only selectable box that scrolls (both ways), or the reason there is none.
fn show_code(ui: &mut egui::Ui, dialog: &CodeDialog, max_height: f32, i18n: &I18n) {
    let caption = ui.strong(i18n.t("codegen.preview"));
    match &dialog.text {
        Ok(code) => {
            let mut shown: &str = code;
            egui::ScrollArea::both()
                .id_salt("code_export_code")
                .max_height(max_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut shown)
                            .font(egui::TextStyle::Monospace)
                            .lock_focus(false)
                            .desired_width(f32::INFINITY),
                    )
                    .labelled_by(caption.id);
                });
        }
        Err(error) => tone_notice(ui, Tone::Bad, &error_text(error, i18n), false),
    }
}

/// Copy (off while there is no code) and Close.
fn buttons(ui: &mut egui::Ui, dialog: &CodeDialog, i18n: &I18n) -> Controls {
    let ready = dialog.clipboard_text().is_some();
    ui.horizontal(|ui| {
        let close = text_button(ui, i18n.t("common.close"));
        let copy = gate(ui, ready, |ui| filled_button(ui, i18n.t("codegen.copy")));
        let mut actions = Vec::new();
        if ready && copy.clicked() {
            actions.push(CodeAction::Copy);
        }
        if close.clicked() {
            actions.push(CodeAction::Close);
        }
        Controls {
            actions,
            activating: vec![close.id, copy.id],
        }
    })
    .inner
}

/// Whether Enter was pressed where it means Copy: not on a widget it already activates (a check
/// box, the language picker, Close or Copy).
fn enter_copies(ui: &egui::Ui, own_enter: &[egui::Id]) -> bool {
    let focused = ui.memory(|m| m.focused());
    let on_activating = focused.is_some_and(|id| own_enter.contains(&id));
    !on_activating && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

/// Puts the previewed text on the clipboard when Copy is among the actions.
fn put_on_clipboard(ctx: &egui::Context, dialog: &CodeDialog, actions: &[CodeAction]) {
    if !actions.contains(&CodeAction::Copy) {
        return;
    }
    if let Some(text) = dialog.clipboard_text() {
        ctx.copy_text(text.to_string());
    }
}

/// One Copy per frame however it was asked for (button and Enter together).
fn dedupe_copy(actions: Vec<CodeAction>) -> Vec<CodeAction> {
    let mut seen_copy = false;
    actions
        .into_iter()
        .filter(|action| {
            let repeat = *action == CodeAction::Copy && seen_copy;
            seen_copy |= *action == CodeAction::Copy;
            !repeat
        })
        .collect()
}

#[cfg(test)]
#[path = "code_export_tests.rs"]
mod tests;
