//! Selecting rows of a results tab and queueing writes for them: the checkbox helpers, the
//! selection bar and the action buttons shared with the detail pane. Every decision (what is
//! enabled, why not) comes from `viewmodel::compare::select_bar`.
use crate::I18n;
use crate::app::compare::CompareAction;
use crate::viewmodel::compare::read_only::READ_ONLY_DESTINATION_KEY;
use crate::viewmodel::compare::{ActionBlock, ActionButton, RowAction, SelectionBar};
use eframe::egui;

/// A checkbox without visible text whose accessible name is `label`; true when it was clicked.
fn labelled_checkbox(ui: &mut egui::Ui, ticked: bool, enabled: bool, label: &str) -> bool {
    let mut value = ticked;
    let response = ui.add_enabled(enabled, egui::Checkbox::without_text(&mut value));
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, ticked, label)
    });
    let clicked = response.clicked();
    response.on_hover_text(label);
    clicked
}

/// The checkbox of one row, named by the row's key text.
pub fn row_checkbox(ui: &mut egui::Ui, ticked: bool, enabled: bool, key_text: &str) -> bool {
    labelled_checkbox(ui, ticked, enabled, key_text)
}

/// The header checkbox. Unticked it selects the loaded page; ticked (the whole page is
/// selected) a click clears the selection of the whole tab, and its name says so.
pub fn header_checkbox(ui: &mut egui::Ui, ticked: bool, enabled: bool, i18n: &I18n) -> bool {
    let key = if ticked {
        "compare.results.clear_selection_hint"
    } else {
        "compare.results.select_page"
    };
    let label = i18n.t(key);
    labelled_checkbox(ui, ticked, enabled, &label)
}

/// The texts of every reason (i18n keys), one per line.
pub fn reasons_text(keys: &[&str], i18n: &I18n) -> String {
    keys.iter()
        .map(|key| i18n.t(key))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A button that says why it is disabled on hover (every reason, when there are any); true when
/// clicked.
fn guarded_button(
    ui: &mut egui::Ui,
    text: String,
    enabled: bool,
    reasons: &[&str],
    i18n: &I18n,
) -> bool {
    let response = ui.add_enabled(enabled, egui::Button::new(text));
    if enabled {
        return response.clicked();
    }
    if !reasons.is_empty() {
        response.on_disabled_hover_text(reasons_text(reasons, i18n));
    }
    false
}

/// The keys of the reasons that close `button`: its block, then the read-only destination.
fn button_reasons(button: &ActionButton) -> Vec<&'static str> {
    let block = button.block.map(ActionBlock::i18n_key);
    let read_only = button.read_only.then_some(READ_ONLY_DESTINATION_KEY);
    block.into_iter().chain(read_only).collect()
}

/// One button per offered action; the clicked action, if any.
pub fn action_buttons(
    ui: &mut egui::Ui,
    buttons: &[ActionButton],
    i18n: &I18n,
) -> Option<RowAction> {
    let mut clicked = None;
    for button in buttons {
        let text = i18n.t(button.action.i18n_key());
        if guarded_button(ui, text, button.is_enabled(), &button_reasons(button), i18n) {
            clicked = Some(button.action);
        }
    }
    clicked
}

/// The bar above the table: how many are selected, Select all, Clear and the queue actions.
pub fn show_selection_bar(
    ui: &mut egui::Ui,
    bar: &SelectionBar,
    i18n: &I18n,
) -> Option<CompareAction> {
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        let selected = bar.selected.to_string();
        ui.label(i18n.t_with("compare.results.selected", &[("count", &selected)]));
        let total = bar.total.to_string();
        let select_all = i18n.t_with("compare.results.select_all", &[("count", &total)]);
        if guarded_button(
            ui,
            select_all,
            bar.can_select_all(),
            &bar.tick_block
                .map(ActionBlock::i18n_key)
                .into_iter()
                .collect::<Vec<_>>(),
            i18n,
        ) {
            action = Some(CompareAction::SelectAllInCategory);
        }
        let clear = i18n.t("compare.results.clear");
        let why_not = [bar
            .tick_block
            .unwrap_or(ActionBlock::NothingSelected)
            .i18n_key()];
        if guarded_button(ui, clear, bar.can_clear(), &why_not, i18n) {
            action = Some(CompareAction::ClearSelection);
        }
        ui.separator();
        if let Some(row_action) = action_buttons(ui, &bar.actions, i18n) {
            action = Some(CompareAction::AddToPending(row_action));
        }
    });
    action
}
