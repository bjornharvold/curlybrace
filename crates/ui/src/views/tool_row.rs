//! One tool's editor: the PATH / file choice, the path box, the file dialog button, the places
//! it is usually installed, and the line that says how the check went. Shared by the global
//! dialog and the connection dialog; it draws the row it is given and reports what the person
//! did. The typed path is the only thing it keeps (in egui's memory).
use crate::I18n;
use crate::app::tools_state::{ProbeScope, ToolsAction};
use crate::theme::widgets::CONTROL_HEIGHT;
use crate::theme::{
    FieldOptions, Tone as ChipTone, chip, gate, outlined_field_opts, shape, text_button,
    tonal_button,
};
use crate::viewmodel::tool_rows::{Tone, ToolRow, ToolStatus, status_line};
use curlybrace_core::tools::{ServerVersion, ToolChoice, ToolKind};
use eframe::egui;
use std::path::{Path, PathBuf};

const PATH_FIELD_WIDTH: f32 = 280.0;
/// Height of the path field's row: the control plus its floating label's overhang.
const PATH_ROW_HEIGHT: f32 = CONTROL_HEIGHT + shape::SPACE_2;
/// Room kept beside the status chip for the Check again button.
const CHECK_AGAIN_WIDTH: f32 = 132.0;

/// What a row is drawn for.
pub struct RowContext<'a> {
    pub scope: ProbeScope,
    pub server: Option<ServerVersion>,
    /// Runnable files found in the usual folders.
    pub suggestions: &'a [PathBuf],
}

/// The tool's name as shown: a command, but still looked up so it can be localised.
pub fn tool_name(kind: ToolKind, i18n: &I18n) -> String {
    i18n.t(&format!("tools.name_{}", kind.binary_name()))
}

fn choose(scope: ProbeScope, kind: ToolKind, choice: ToolChoice) -> ToolsAction {
    match scope {
        ProbeScope::Global => ToolsAction::ChooseGlobal { kind, choice },
        ProbeScope::Connection(connection) => ToolsAction::ChooseOverride {
            connection,
            kind,
            choice,
        },
    }
}

fn path_text(choice: &ToolChoice) -> String {
    match choice {
        ToolChoice::Path => String::new(),
        ToolChoice::Executable(path) => path.to_string_lossy().into_owned(),
    }
}

/// Draws the row and returns what the person did, if anything.
pub fn show_tool_row(
    ui: &mut egui::Ui,
    row: &ToolRow,
    context: &RowContext<'_>,
    i18n: &I18n,
) -> Option<ToolsAction> {
    let kind = row.kind;
    let name = tool_name(kind, i18n);
    let mut action = None;
    ui.strong(&name);
    if let ProbeScope::Connection(connection) = context.scope {
        let mut follows = !row.own;
        let toggle = ui.checkbox(&mut follows, i18n.t("tools.use_global"));
        // Three identical boxes would be indistinguishable to a screen reader.
        let named = i18n.t_with("tools.use_global_named", &[("tool", &name)]);
        toggle.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, follows, named.clone())
        });
        if toggle.changed() {
            action = Some(if follows {
                ToolsAction::UseGlobal { connection, kind }
            } else {
                choose(context.scope, kind, row.choice.clone())
            });
        }
    }
    ui.add_enabled_ui(row.own, |ui| {
        action = choice_rows(ui, row, context, &name, i18n).or(action.take());
    });
    let line = status_line(&row.status, context.server, i18n);
    let tone = match (line.tone, &row.status) {
        (Tone::Warning, _) => ChipTone::Warning,
        (Tone::Error, _) => ChipTone::Bad,
        (Tone::Neutral, ToolStatus::Found { .. }) => ChipTone::Good,
        (Tone::Neutral, _) => ChipTone::Neutral,
    };
    ui.horizontal(|ui| {
        // The chip elides what does not fit, so the button beside it always stays in view.
        let room = (ui.available_width() - CHECK_AGAIN_WIDTH).max(PATH_FIELD_WIDTH / 2.0);
        ui.scope(|ui| {
            ui.set_max_width(room);
            chip(ui, tone, &line.text);
        });
        // One check at a time per tool: another click while it runs would only start another
        // program whose answer is thrown away.
        let idle = !matches!(row.status, ToolStatus::Checking);
        let again = gate(ui, idle, |ui| text_button(ui, i18n.t("tools.check_again")));
        let named = i18n.t_with("tools.check_again_named", &[("tool", &name)]);
        again.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, idle, named.clone())
        });
        if again.clicked() {
            action = Some(ToolsAction::Recheck {
                scope: context.scope,
                kind,
            });
        }
    });
    action
}

/// The two ways to get a tool and what goes with the second.
fn choice_rows(
    ui: &mut egui::Ui,
    row: &ToolRow,
    context: &RowContext<'_>,
    name: &str,
    i18n: &I18n,
) -> Option<ToolsAction> {
    let (scope, kind) = (context.scope, row.kind);
    let mut action = None;
    let on_path = matches!(row.choice, ToolChoice::Path);
    let path_radio = ui.radio(on_path, i18n.t("tools.path"));
    let path_name = i18n.t_with("tools.path_named", &[("tool", name)]);
    path_radio.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::RadioButton,
            true,
            on_path,
            path_name.clone(),
        )
    });
    if path_radio.clicked() && !on_path {
        action = Some(choose(scope, kind, ToolChoice::Path));
    }
    ui.horizontal(|ui| {
        let file_radio = ui.radio(!on_path, i18n.t("tools.executable"));
        let file_name = i18n.t_with("tools.executable_named", &[("tool", name)]);
        file_radio.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::RadioButton,
                true,
                !on_path,
                file_name.clone(),
            )
        });
        if file_radio.clicked() {
            // Nothing is chosen until there is a path: send the person to the box that takes it.
            ui.memory_mut(|memory| memory.request_focus(path_box_id(scope, kind)));
        }
        action = path_box(ui, row, scope, name, i18n).or(action.take());
        let browse = tonal_button(ui, i18n.t("tools.choose_file"));
        let named = i18n.t_with("tools.choose_file_named", &[("tool", name)]);
        browse.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, named.clone())
        });
        if browse.clicked() {
            action = Some(ToolsAction::Browse { scope, kind });
        }
    });
    for path in context.suggestions {
        action = suggestion(ui, path, scope, kind, i18n).or(action.take());
    }
    action
}

/// The text box for the path. It follows the saved choice (a file picked in the dialog shows at
/// once) until the person types; Enter or leaving the box with something new in it applies it.
fn path_box(
    ui: &mut egui::Ui,
    row: &ToolRow,
    scope: ProbeScope,
    name: &str,
    i18n: &I18n,
) -> Option<ToolsAction> {
    let id = egui::Id::new(("tool_path_draft", scope, row.kind.binary_name()));
    let saved = path_text(&row.choice);
    let (synced, mut draft): (String, String) = ui
        .data_mut(|d| d.get_temp(id))
        .unwrap_or_else(|| (saved.clone(), saved.clone()));
    if synced != saved {
        draft = saved.clone();
    }
    let label = i18n.t_with("tools.path_field", &[("tool", name)]);
    let options = FieldOptions {
        id: Some(path_box_id(scope, row.kind)),
        ..FieldOptions::default()
    };
    let response = ui
        .allocate_ui_with_layout(
            egui::vec2(PATH_FIELD_WIDTH, PATH_ROW_HEIGHT),
            egui::Layout::top_down(egui::Align::Min),
            |ui| outlined_field_opts(ui, &label, &mut draft, "", options),
        )
        .inner;
    // Escape also takes the focus away: that is backing out, so the draft goes back to what is
    // saved instead of being applied. Enter and clicking elsewhere apply it.
    let backed_out = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape));
    if backed_out {
        draft.clone_from(&saved);
    }
    let submitted =
        response.lost_focus() && !backed_out && draft.trim() != saved && !draft.trim().is_empty();
    ui.data_mut(|d| d.insert_temp(id, (saved, draft.clone())));
    submitted.then(|| {
        choose(
            scope,
            row.kind,
            ToolChoice::Executable(Path::new(draft.trim()).to_path_buf()),
        )
    })
}

fn path_box_id(scope: ProbeScope, kind: ToolKind) -> egui::Id {
    egui::Id::new(("tool_path_box", scope, kind.binary_name()))
}

fn suggestion(
    ui: &mut egui::Ui,
    path: &Path,
    scope: ProbeScope,
    kind: ToolKind,
    i18n: &I18n,
) -> Option<ToolsAction> {
    let shown = path.to_string_lossy();
    let label = i18n.t_with("tools.suggestion", &[("path", &shown)]);
    text_button(ui, label)
        .clicked()
        .then(|| choose(scope, kind, ToolChoice::Executable(path.to_path_buf())))
}
