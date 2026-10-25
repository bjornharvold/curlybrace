//! Picking documents to copy: a checkbox per document and the bar above the views with
//! "select all documents of the query", "clear" and Copy. Every control is a plain button or
//! checkbox, so the whole flow works from the keyboard.
use crate::I18n;
use crate::viewmodel::copy_menu::SYSTEM_SOURCE_BLOCKED_KEY;
use crate::viewmodel::{DocAction, MenuItem, PageKey, TreeState};
use bson::Bson;
use curlybrace_core::store::Page;
use eframe::egui;
use eframe::egui::{WidgetInfo, WidgetType};

/// Why Copy is disabled on the documents of a `system.*` collection.
/// Key of the plain Copy label, shown while nothing is picked.
const COPY_KEY: &str = "copy.menu.copy";

/// The width a checkbox column needs, so rows without one line up with rows that have one.
pub fn checkbox_width(ui: &egui::Ui) -> f32 {
    ui.spacing().icon_width + ui.spacing().item_spacing.x
}

/// A checkbox without visible text for the document numbered `number`; true when it was toggled.
/// Documents without an `_id` (a projection can drop it) cannot be picked by hand.
pub fn row_checkbox(
    ui: &mut egui::Ui,
    checked: bool,
    pickable: bool,
    number: u64,
    i18n: &I18n,
) -> bool {
    let mut value = checked;
    let response = ui.add_enabled(pickable, egui::Checkbox::without_text(&mut value));
    let label = i18n.t_with("doc.select_row", &[("n", &number.to_string())]);
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, pickable, checked, &label));
    response.changed()
}

/// The checkbox of the document at `index` of `page`, ticked as the pick says; true when it was
/// toggled.
pub fn doc_checkbox(
    ui: &mut egui::Ui,
    state: &TreeState,
    page: &Page,
    index: usize,
    i18n: &I18n,
) -> bool {
    let id = page.docs.get(index).and_then(pick_id);
    let checked = id.is_some_and(|id| state.pick.contains(id));
    let number = page.skip + index as u64 + 1;
    row_checkbox(ui, checked, id.is_some(), number, i18n)
}

/// Applies the toggle of the checkbox of the document at `toggled`, if one was clicked.
pub fn apply_toggle(state: &mut TreeState, page: &Page, toggled: Option<usize>) {
    if let Some(id) = toggled.and_then(|i| page.docs.get(i)).and_then(pick_id) {
        state.pick = std::mem::take(&mut state.pick).row_toggled(id, page);
    }
}

/// Hands the row-menu choice (the document index and the item) to the tree state; the action it
/// asks the page for, if any.
pub fn resolve_choice(
    state: &mut TreeState,
    page: &Page,
    key: &PageKey,
    chosen: Option<(usize, MenuItem)>,
) -> Option<DocAction> {
    let (index, item) = chosen?;
    let doc = page.docs.get(index)?;
    let (next, action) = std::mem::take(state).menu_chosen(item, doc, key);
    *state = next;
    action
}

/// The `_id` of the document, when it can be picked by hand.
pub fn pick_id(doc: &bson::Document) -> Option<&Bson> {
    curlybrace_core::crud::id_of(doc).ok()
}

/// The bar above the document views; returns the Copy action when Copy was clicked.
pub fn show_selection_bar(
    ui: &mut egui::Ui,
    state: &mut TreeState,
    i18n: &I18n,
) -> Option<DocAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        if ui.button(i18n.t("doc.select_all_result")).clicked() {
            state.pick = std::mem::take(&mut state.pick).all_selected();
        }
        let clear = ui.add_enabled(
            !state.pick.is_empty(),
            egui::Button::new(i18n.t("doc.clear_selection")),
        );
        if clear.clicked() {
            state.pick = Default::default();
        }
        action = copy_button(ui, state, i18n);
    });
    action
}

fn copy_button(ui: &mut egui::Ui, state: &TreeState, i18n: &I18n) -> Option<DocAction> {
    let label = match state.pick.is_empty() {
        true => i18n.t(COPY_KEY),
        false => {
            let (key, count) = state.pick.menu_label_key();
            i18n.t_with(key, &[("count", &count.to_string())])
        }
    };
    let action = state.copy_picked();
    let button = ui.add_enabled(action.is_some(), egui::Button::new(label));
    match (&action, state.copy_blocked) {
        (None, true) => {
            button.on_disabled_hover_text(i18n.t(SYSTEM_SOURCE_BLOCKED_KEY));
            None
        }
        (None, false) => {
            button.on_disabled_hover_text(i18n.t("doc.selection_hint"));
            None
        }
        (Some(_), _) => button.clicked().then(|| action.clone()).flatten(),
    }
}
