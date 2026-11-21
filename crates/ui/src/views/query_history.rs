//! The History button of the find bar and its popup (#72): a search, then the favourites of the
//! collection, then its recent queries. Draws what it is given and returns what the person did;
//! the dialogs the popup opens are in `query_history_dialogs`.
use super::aggregate::named_button;
use crate::I18n;
use crate::theme::{compact_text_button, gate, outlined_field, text_button};
use crate::viewmodel::query_history::{
    HistoryAction, HistoryUi, Listing, matches_phrase, one_line, ran_phrase,
};
use curlybrace_core::query_history::{Entry, Favourite};
use eframe::egui::{self, accesskit};

const POPUP_WIDTH: f32 = 380.0;
/// The share of the window's height the popup's lists may take before they scroll.
const LIST_HEIGHT_SHARE: f32 = 0.5;

/// What the popup shows: the collection's lists, or nothing when no collection is selected.
pub struct HistoryPanel<'a> {
    pub ui: &'a HistoryUi,
    /// Builds the lists of the collection; asked only while the popup is open (a closed popup
    /// costs nothing per frame). `None` when no collection is selected.
    pub listing: Option<Box<dyn Fn() -> Listing<'a> + 'a>>,
    /// Unix seconds, for "5 min ago".
    pub now: i64,
    /// Whether finds that are run are recorded (the setting).
    pub remembering: bool,
}

/// The History button (an icon, so it does not make the find bar's row wrap; "History" is its
/// accessible name and tooltip) and, while it is open, its popup. Without a collection the
/// button is off and says why as its tooltip and accessible description. Gives the response of the button (its
/// width is part of the row's measurement) and what was done.
pub fn show_history_button(
    ui: &mut egui::Ui,
    panel: &HistoryPanel<'_>,
    i18n: &I18n,
) -> (egui::Response, Vec<HistoryAction>) {
    let name = i18n.t("history.button");
    let glyph = i18n.t("history.button_icon");
    let Some(make_listing) = &panel.listing else {
        let reason = i18n.t("history.button_no_collection");
        let off = gate(ui, false, |ui| compact_text_button(ui, glyph)).on_hover_ui(|ui| {
            ui.strong(&name);
            ui.label(&reason);
        });
        off.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, false, name.clone())
        });
        ui.ctx()
            .accesskit_node_builder(off.id, |node| node.set_description(reason));
        return (off, Vec::new());
    };
    let button = compact_text_button(ui, glyph).on_hover_text(&name);
    button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name.clone()));
    let hint = i18n.t("history.button_hint");
    ui.ctx()
        .accesskit_node_builder(button.id, |node| node.set_description(hint));
    let actions = egui::Popup::menu(&button)
        // Typing in the search box and pressing the section headings must not close it: a pick
        // closes it explicitly.
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| popup(ui, panel, &make_listing(), i18n))
        .map(|shown| shown.inner);
    // Closed: the search of the last time is forgotten, so the next opening starts clean.
    let actions = match actions {
        Some(actions) => actions,
        None if !panel.ui.search.is_empty() => vec![HistoryAction::Search(String::new())],
        None => Vec::new(),
    };
    (button, actions)
}

fn popup(
    ui: &mut egui::Ui,
    panel: &HistoryPanel<'_>,
    listing: &Listing<'_>,
    i18n: &I18n,
) -> Vec<HistoryAction> {
    ui.set_width(POPUP_WIDTH);
    let mut actions = Vec::new();
    ui.label(crate::theme::typography::dialog_title(
        ui,
        i18n.t("history.title"),
    ));
    let mut search = panel.ui.search.clone();
    let field = outlined_field(
        ui,
        &i18n.t("history.search"),
        &mut search,
        &i18n.t("history.search_hint"),
    );
    if field.changed() {
        actions.push(HistoryAction::Search(search));
    }
    show_count(ui, panel, listing, i18n);
    let max_height = ui.ctx().screen_rect().height() * LIST_HEIGHT_SHARE;
    let picked = egui::ScrollArea::vertical()
        .max_height(max_height)
        .show(ui, |ui| lists(ui, panel, listing, i18n))
        .inner;
    if let Some(picked) = picked {
        actions.push(picked);
        ui.close();
    }
    ui.separator();
    if listing.recent_total > 0 && text_button(ui, i18n.t("history.clear")).clicked() {
        actions.push(HistoryAction::ClearAsked);
        ui.close();
    }
    ui.weak(i18n.t("history.privacy"));
    actions
}

/// How many rows the search left, announced politely to a screen reader as it changes.
fn show_count(ui: &mut egui::Ui, panel: &HistoryPanel<'_>, listing: &Listing<'_>, i18n: &I18n) {
    if panel.ui.search.trim().is_empty() {
        return;
    }
    let response = ui.weak(matches_phrase(listing.matches(), i18n));
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_live(accesskit::Live::Polite);
    });
}

fn lists(
    ui: &mut egui::Ui,
    panel: &HistoryPanel<'_>,
    listing: &Listing<'_>,
    i18n: &I18n,
) -> Option<HistoryAction> {
    let mut picked = None;
    ui.strong(i18n.t("history.section.favourites"));
    if listing.favourites_total == 0 {
        ui.weak(i18n.t("history.empty.favourites"));
    }
    for favourite in &listing.favourites {
        picked = favourite_row(ui, favourite, i18n).or(picked);
    }
    ui.add_space(ui.spacing().item_spacing.y);
    ui.strong(i18n.t("history.section.recent"));
    if listing.recent_total == 0 {
        let key = match panel.remembering {
            true => "history.empty.recent",
            false => "history.empty.recent_off",
        };
        ui.weak(i18n.t(key));
    }
    for (index, entry) in &listing.recent {
        picked = recent_row(ui, (*index, entry), panel.now, i18n).or(picked);
    }
    picked
}

/// A row's load button: the one-line text (elided), named for a screen reader as a whole.
fn load_button(ui: &mut egui::Ui, shown: &str, name: String) -> egui::Response {
    let button = text_button(ui, shown);
    button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name.clone()));
    button
}

fn recent_row(
    ui: &mut egui::Ui,
    (index, entry): (usize, &Entry),
    now: i64,
    i18n: &I18n,
) -> Option<HistoryAction> {
    let mut picked = None;
    let line = one_line(&entry.text, &i18n.t("history.row.no_filter"));
    let name = i18n.t_with("history.row.load", &[("text", &line)]);
    if load_button(ui, &line, name).clicked() {
        picked = Some(HistoryAction::LoadRecent(index));
    }
    ui.horizontal(|ui| {
        ui.weak(ran_phrase(entry.ran_unix, now, i18n));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let star = i18n.t_with("history.star_n", &[("text", &line)]);
            if named_button(ui, true, (&i18n.t("history.star"), star)) {
                picked = Some(HistoryAction::StarAsked(index));
            }
        });
    });
    ui.separator();
    picked
}

fn favourite_row(ui: &mut egui::Ui, favourite: &Favourite, i18n: &I18n) -> Option<HistoryAction> {
    let mut picked = None;
    let line = one_line(&favourite.text, &i18n.t("history.row.no_filter"));
    let name = i18n.t_with(
        "history.row.load_favourite",
        &[("name", &favourite.name), ("text", &line)],
    );
    if load_button(ui, &favourite.name, name).clicked() {
        picked = Some(HistoryAction::LoadFavourite(favourite.id));
    }
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(egui::RichText::new(&line).weak()).truncate());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let delete = i18n.t_with("history.delete_n", &[("name", &favourite.name)]);
            if named_button(ui, true, (&i18n.t("history.delete"), delete)) {
                picked = Some(HistoryAction::DeleteAsked(favourite.id));
            }
            let rename = i18n.t_with("history.rename_n", &[("name", &favourite.name)]);
            if named_button(ui, true, (&i18n.t("history.rename"), rename)) {
                picked = Some(HistoryAction::RenameAsked(favourite.id));
            }
        });
    });
    ui.separator();
    picked
}
