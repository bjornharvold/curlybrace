//! The saved pipeline controls of the Aggregate header: Save, the Open menu (names with the
//! collection they were saved from, newest first, Rename and Delete per item), and the name of
//! the pipeline the draft came from with a "Modified" mark. Draws what it is given and returns
//! what the person did; the dialogs they open are in `aggregate_saved_dialogs`.
use super::aggregate::named_button;
use crate::I18n;
use crate::app::AggregateAction;
use crate::theme::{Tone, chip, gate, text_button, tonal_button};
use crate::viewmodel::aggregate::AggregateState;
use curlybrace_core::saved_pipeline::SavedPipeline;
use eframe::egui;

const MENU_WIDTH: f32 = 320.0;
/// The row room the opened name needs before it is worth keeping on the line.
const OPENED_MIN_ROOM: f32 = 160.0;
/// The share of the window's height the Open menu's list may take before it scrolls.
const MENU_HEIGHT_SHARE: f32 = 0.6;

/// Save, Open and the opened pipeline's name, inside the header's wrapping row. `entries` are
/// the active connection's saved pipelines, newest first.
pub fn show_saved_toolbar(
    ui: &mut egui::Ui,
    draft: &AggregateState,
    entries: &[SavedPipeline],
    i18n: &I18n,
) -> Vec<AggregateAction> {
    let mut actions = Vec::new();
    if tonal_button(ui, i18n.t("aggregate.saved.save")).clicked() {
        actions.push(AggregateAction::SaveClicked);
    }
    actions.extend(open_menu(ui, entries, i18n));
    show_opened(ui, draft, i18n);
    actions
}

/// The Open button. With nothing saved it is off, and says why as its tooltip and description.
fn open_menu(ui: &mut egui::Ui, entries: &[SavedPipeline], i18n: &I18n) -> Option<AggregateAction> {
    let label = i18n.t("aggregate.saved.open");
    if entries.is_empty() {
        let reason = i18n.t("aggregate.saved.open_empty");
        let off = gate(ui, false, |ui| tonal_button(ui, label)).on_hover_text(&reason);
        ui.ctx()
            .accesskit_node_builder(off.id, |node| node.set_description(reason));
        return None;
    }
    let button = tonal_button(ui, label);
    egui::Popup::menu(&button)
        .show(|ui| menu_items(ui, entries, i18n))
        .and_then(|shown| shown.inner)
}

fn menu_items(
    ui: &mut egui::Ui,
    entries: &[SavedPipeline],
    i18n: &I18n,
) -> Option<AggregateAction> {
    ui.set_width(MENU_WIDTH);
    let max_height = ui.ctx().screen_rect().height() * MENU_HEIGHT_SHARE;
    egui::ScrollArea::vertical()
        .max_height(max_height)
        .show(ui, |ui| {
            // Every item is drawn, top to bottom; at most one was picked.
            entries
                .iter()
                .fold(None, |picked, entry| item(ui, entry, i18n).or(picked))
        })
        .inner
}

/// One saved pipeline: its name (the Open button) and, below, the collection hint with Rename and
/// Delete. Picking any of them closes the menu.
fn item(ui: &mut egui::Ui, entry: &SavedPipeline, i18n: &I18n) -> Option<AggregateAction> {
    let mut picked = None;
    let open = text_button(ui, &entry.name);
    open.widget_info(|| {
        let name = i18n.t_with("aggregate.saved.open_item_n", &[("name", &entry.name)]);
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name)
    });
    if open.clicked() {
        picked = Some(AggregateAction::OpenMenuPicked(entry.id));
    }
    ui.horizontal(|ui| {
        if let Some(hint) = &entry.ns_hint {
            ui.add(egui::Label::new(egui::RichText::new(hint).weak()).truncate());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let delete = i18n.t_with("aggregate.saved.delete_n", &[("name", &entry.name)]);
            if named_button(ui, true, (&i18n.t("aggregate.saved.delete"), delete)) {
                picked = Some(AggregateAction::DeleteAsked(entry.id));
            }
            let rename = i18n.t_with("aggregate.saved.rename_n", &[("name", &entry.name)]);
            if named_button(ui, true, (&i18n.t("aggregate.saved.rename"), rename)) {
                picked = Some(AggregateAction::RenameStarted(entry.id));
            }
        });
    });
    ui.separator();
    if picked.is_some() {
        ui.close();
    }
    picked
}

/// The opened pipeline's name and, while the draft differs from it, the Modified mark. A name
/// that does not fit is elided with the full name as its tooltip.
fn show_opened(ui: &mut egui::Ui, draft: &AggregateState, i18n: &I18n) {
    let Some((_, name)) = &draft.saved.opened else {
        return;
    };
    if ui.available_width() < OPENED_MIN_ROOM {
        ui.end_row();
    }
    chip(
        ui,
        Tone::Neutral,
        &i18n.t_with("aggregate.saved.opened", &[("name", name)]),
    );
    if draft.is_modified() {
        chip(ui, Tone::Warning, &i18n.t("aggregate.saved.modified"));
    }
}
