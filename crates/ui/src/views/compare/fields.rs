//! Copying single fields from the detail pane: the changes grid with a checkbox per field, and
//! the bar above it with the two queue buttons, the count of ticked fields and Clear. Every
//! decision (which checkbox or button is open, why not) comes from `viewmodel::compare::field_bar`.
use super::select::reasons_text;
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::read_only::READ_ONLY_DESTINATION_KEY;
use crate::viewmodel::compare::{ChangeLine, FieldBar, FieldBlock, FieldRow, RowAction};
use eframe::egui;

/// Width the reason text wraps at.
const REASON_WIDTH: f32 = 360.0;

const COLUMNS: [&str; 4] = [
    "compare.detail.change_column",
    "compare.detail.path_column",
    "compare.detail.source_value",
    "compare.detail.target_value",
];

/// The changes of one document: the field bar, then the grid. The action the person took, if any.
pub fn show_changes(
    ui: &mut egui::Ui,
    changes: &[ChangeLine],
    bar: &FieldBar,
    i18n: &I18n,
) -> Option<CompareAction> {
    if changes.is_empty() {
        ui.label(i18n.t("compare.detail.no_changes"));
        return None;
    }
    let queued = show_bar(ui, bar, i18n);
    let toggled = show_grid(ui, changes, bar, i18n);
    toggled.or(queued)
}

fn show_grid(
    ui: &mut egui::Ui,
    changes: &[ChangeLine],
    bar: &FieldBar,
    i18n: &I18n,
) -> Option<CompareAction> {
    let mut action = None;
    egui::Grid::new("compare_changes")
        .striped(true)
        .show(ui, |ui| {
            // The checkbox column has no title: each checkbox is named by its field path.
            // A spacer, not an empty label: an empty label would be an unnamed accessibility node.
            ui.allocate_space(egui::Vec2::ZERO);
            for key in COLUMNS {
                ui.strong(i18n.t(key));
            }
            // The reason column has no title either.
            ui.allocate_space(egui::Vec2::ZERO);
            ui.end_row();
            for (line, row) in changes.iter().zip(&bar.rows) {
                action = show_change(ui, line, row, i18n).or(action.take());
                ui.end_row();
            }
        });
    action
}

fn show_change(
    ui: &mut egui::Ui,
    line: &ChangeLine,
    row: &FieldRow,
    i18n: &I18n,
) -> Option<CompareAction> {
    let toggled = field_checkbox(ui, row, &line.path, i18n);
    ui.label(i18n.t(line.kind));
    ui.monospace(&line.path);
    for value in [&line.source_text, &line.target_text] {
        match value {
            Some(text) => ui.monospace(text),
            None => ui.weak(i18n.t("compare.detail.absent")),
        };
    }
    show_reason(ui, row, i18n);
    toggled.then(|| CompareAction::ToggleField(line.segments.clone()))
}

/// Why a field cannot be copied on its own, in plain sight (not only on hover), wrapped to a
/// fixed width so a long sentence does not stretch the grid. Other blocks (busy, no run) apply to
/// every row and are explained by the buttons.
fn show_reason(ui: &mut egui::Ui, row: &FieldRow, i18n: &I18n) {
    match row.block {
        Some(block @ FieldBlock::Unaddressable(_)) => {
            ui.scope(|ui| {
                ui.set_max_width(REASON_WIDTH);
                ui.add(
                    egui::Label::new(egui::RichText::new(i18n.t(block.i18n_key())).weak()).wrap(),
                );
            });
        }
        _ => {
            ui.allocate_space(egui::Vec2::ZERO);
        }
    }
}

/// A checkbox without visible text whose accessible name is the field path. Disabled, it says
/// why on hover; true when it was clicked.
fn field_checkbox(ui: &mut egui::Ui, row: &FieldRow, path: &str, i18n: &I18n) -> bool {
    let enabled = row.block.is_none();
    let mut value = row.ticked;
    let response = ui.add_enabled(enabled, egui::Checkbox::without_text(&mut value));
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, row.ticked, path)
    });
    let clicked = response.clicked();
    match row.block {
        Some(block) => {
            response.on_disabled_hover_text(i18n.t(block.i18n_key()));
        }
        None => {
            response.on_hover_text(path);
        }
    }
    clicked
}

/// The two queue buttons, how many fields are ticked, and Clear.
fn show_bar(ui: &mut egui::Ui, bar: &FieldBar, i18n: &I18n) -> Option<CompareAction> {
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        for (row_action, key) in [
            (
                RowAction::CopyFieldsToTarget,
                "compare.detail.copy_fields_to_target",
            ),
            (
                RowAction::CopyFieldsToSource,
                "compare.detail.copy_fields_to_source",
            ),
        ] {
            let reasons = queue_reasons(bar, row_action);
            if guarded_button(ui, i18n.t(key), &reasons, i18n) {
                action = Some(CompareAction::AddFieldsToPending(row_action));
            }
        }
        let count = bar.selected.to_string();
        ui.label(i18n.t_with("compare.detail.selected_fields", &[("count", &count)]));
        let clear = bar.clear.map(FieldBlock::i18n_key);
        let clear = clear.as_slice();
        if guarded_button(ui, i18n.t("compare.detail.clear_fields"), clear, i18n) {
            action = Some(CompareAction::ClearFieldSelection);
        }
    });
    action
}

/// The keys of the reasons a queue button is closed: its block, then the read-only destination.
fn queue_reasons(bar: &FieldBar, action: RowAction) -> Vec<&'static str> {
    let block = bar.queue_block(action).map(FieldBlock::i18n_key);
    let read_only = bar
        .queue_read_only(action)
        .then_some(READ_ONLY_DESTINATION_KEY);
    block.into_iter().chain(read_only).collect()
}

/// A button that says why it is disabled on hover (every reason); true when clicked.
fn guarded_button(ui: &mut egui::Ui, text: String, reasons: &[&str], i18n: &I18n) -> bool {
    let response = ui.add_enabled(reasons.is_empty(), egui::Button::new(text));
    if reasons.is_empty() {
        return response.clicked();
    }
    response.on_disabled_hover_text(reasons_text(reasons, i18n));
    false
}
