//! The pair table: one row per pair with its include box, names, effective key, a marker when it
//! differs from the defaults and its check verdict, plus Add all, the system switch and the
//! collections that have no counterpart. Rows are drawn with `show_rows` (only the visible
//! ones); every row's content comes from `CompareState::pair_row`.
use super::table::{
    Col, MAX_ROWS_HEIGHT, ROW_HEIGHT, cell, entry_status_label, header, status_text, truncated,
    widths,
};
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::preflight::side_key;
use crate::viewmodel::compare::{
    CompareState, ListView, PairRow, StatusIcon, Unmatched, can_pair_with,
};
use curlybrace_core::compare::protocol::Side;
use eframe::egui;

/// Source, target, key, marker, status and the row's buttons; the names and the status share
/// what the window leaves after the columns that need a width of their own.
const COLUMNS: [Col; 6] = [
    Col::Flex(2.2),
    Col::Flex(2.0),
    Col::Fixed(80.0),
    Col::Fixed(100.0),
    Col::Flex(2.6),
    Col::Fixed(190.0),
];
const UNMATCHED_NAME_W: f32 = 200.0;

/// The table and the helpers under it. `list` is the memoised view of the pair list.
pub fn show_pairs(
    ui: &mut egui::Ui,
    state: &CompareState,
    list: &ListView,
    i18n: &I18n,
) -> Option<CompareAction> {
    ui.strong(i18n.t("compare.pairs.title"));
    let table = show_table(ui, state, list, i18n);
    let helpers = show_helpers(ui, state, list, i18n);
    table.or(helpers)
}

fn show_table(
    ui: &mut egui::Ui,
    state: &CompareState,
    list: &ListView,
    i18n: &I18n,
) -> Option<CompareAction> {
    let total = state.pairs.entries.len();
    if total == 0 {
        ui.label(i18n.t("compare.pairs.empty"));
        return None;
    }
    let widths = widths(ui, &COLUMNS);
    header(ui, &widths, &titles(i18n));
    egui::ScrollArea::vertical()
        .id_salt("compare_pair_rows")
        .max_height(MAX_ROWS_HEIGHT)
        .auto_shrink([false, true])
        .show_rows(ui, ROW_HEIGHT, total, |ui, range| {
            let mut action = None;
            for index in range {
                let Some(row) = state.pair_row(index, &list.digest) else {
                    ui.add_space(ROW_HEIGHT);
                    continue;
                };
                let shown = ui
                    .push_id(row.entry, |ui| show_row(ui, &widths, &row, i18n))
                    .inner;
                action = shown.or(action);
            }
            action
        })
        .inner
}

fn titles(i18n: &I18n) -> Vec<String> {
    vec![
        i18n.t("compare.pairs.column.source"),
        i18n.t("compare.pairs.column.target"),
        i18n.t("compare.pairs.column.key"),
        String::new(),
        i18n.t("compare.pairs.column.status"),
        String::new(),
    ]
}

fn show_row(ui: &mut egui::Ui, w: &[f32], row: &PairRow, i18n: &I18n) -> Option<CompareAction> {
    ui.horizontal(|ui| {
        let included = cell(ui, w[0], |ui| include_box(ui, row, i18n));
        cell(ui, w[1], |ui| truncated(ui, &row.target));
        cell(ui, w[2], |ui| truncated(ui, &row.key));
        cell(ui, w[3], |ui| {
            if row.customised {
                let text = i18n.t("compare.pairs.customised");
                ui.label(status_text(StatusIcon::Customised, &text));
            }
        });
        cell(ui, w[4], |ui| status_cell(ui, row, i18n));
        let buttons = cell(ui, w[5], |ui| row_buttons(ui, row, i18n));
        included.or(buttons)
    })
    .inner
}

/// The include box beside the source name. The box has no text of its own (so the row keeps
/// a fixed height); the name labels it for screen readers and carries the full text on hover.
fn include_box(ui: &mut egui::Ui, row: &PairRow, i18n: &I18n) -> Option<CompareAction> {
    let mut included = row.included;
    let hint = i18n.t("compare.pairs.column.included");
    let response = ui.checkbox(&mut included, "").on_hover_text(hint);
    let name = ui.add(egui::Label::new(row.source.as_str()).truncate());
    name.clone().on_hover_text(row.source.as_str());
    response.clone().labelled_by(name.id);
    response
        .changed()
        .then_some(CompareAction::IncludePair(row.entry, included))
}

fn status_cell(ui: &mut egui::Ui, row: &PairRow, i18n: &I18n) {
    let text = entry_status_label(&row.status, i18n);
    let label = ui.add(egui::Label::new(text.as_str()).truncate());
    match row.status.hover_key() {
        Some(key) => label.on_hover_text(i18n.t(key)),
        None => label.on_hover_text(text),
    };
}

fn row_buttons(ui: &mut egui::Ui, row: &PairRow, i18n: &I18n) -> Option<CompareAction> {
    let mut action = None;
    if ui.button(i18n.t("compare.pairs.edit_settings")).clicked() {
        action = Some(CompareAction::EditPair(Some(row.entry)));
    }
    if ui.button(i18n.t("compare.pairs.remove")).clicked() {
        action = Some(CompareAction::RemovePair(row.entry));
    }
    action
}

/// Add all, the system switch and the collections with no counterpart.
fn show_helpers(
    ui: &mut egui::Ui,
    state: &CompareState,
    list: &ListView,
    i18n: &I18n,
) -> Option<CompareAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let add_all = egui::Button::new(i18n.t("compare.pairs.add_all_same_named"));
        if ui.add_enabled(list.can_add_all(), add_all).clicked() {
            action = Some(CompareAction::AddAllSameNamed);
        }
        let mut system = state.pairing.include_system;
        let switch = ui.checkbox(&mut system, i18n.t("compare.pairs.show_system"));
        if switch.changed() {
            action = Some(CompareAction::ToggleSystemCollections(system));
        }
    });
    let unmatched = show_unmatched(ui, &list.unmatched, i18n);
    action.or(unmatched)
}

fn show_unmatched(ui: &mut egui::Ui, names: &[Unmatched], i18n: &I18n) -> Option<CompareAction> {
    if names.is_empty() {
        return None;
    }
    ui.strong(i18n.t("compare.pairs.no_counterpart"));
    ui.weak(i18n.t("compare.pairs.no_counterpart_hint"));
    egui::ScrollArea::vertical()
        .id_salt("compare_unmatched_rows")
        .max_height(MAX_ROWS_HEIGHT)
        .auto_shrink([false, true])
        .show_rows(ui, ROW_HEIGHT, names.len(), |ui, range| {
            let mut action = None;
            for item in &names[range] {
                let id = (item.side == Side::Source, item.name.as_str());
                let shown = ui
                    .push_id(id, |ui| show_unmatched_row(ui, item, i18n))
                    .inner;
                action = shown.or(action);
            }
            action
        })
        .inner
}

/// The typed name lives in egui memory (a widget buffer); it is cleared once used.
fn show_unmatched_row(ui: &mut egui::Ui, item: &Unmatched, i18n: &I18n) -> Option<CompareAction> {
    let side = i18n.t(side_key(item.side));
    let text = i18n.t_with(
        "compare.pairs.no_counterpart_row",
        &[("name", &item.name), ("side", &side)],
    );
    let id = ui.make_persistent_id("typed_name");
    let mut typed: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let mut action = None;
    ui.horizontal(|ui| {
        let name_w = (ui.available_width() - UNMATCHED_NAME_W - 140.0).max(120.0);
        cell(ui, name_w, |ui| truncated(ui, &text));
        let hint = i18n.t("compare.pairs.counterpart_name_hint");
        let edit = egui::TextEdit::singleline(&mut typed).hint_text(hint);
        ui.add(edit.desired_width(UNMATCHED_NAME_W));
        let pair = egui::Button::new(i18n.t("compare.pairs.pair_with_name"));
        if ui.add_enabled(can_pair_with(&typed), pair).clicked() {
            action = Some(CompareAction::AddUnmatched {
                side: item.side,
                name: item.name.clone(),
                as_name: typed.trim().to_string(),
            });
            typed.clear();
        }
    });
    ui.data_mut(|d| d.insert_temp(id, typed));
    action
}
