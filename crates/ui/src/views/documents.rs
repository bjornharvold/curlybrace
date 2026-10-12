//! Document area: mode switcher, insert button, the three views and the delete confirmation.
use super::{json::show_json, table::show_table, tree::show_tree};
use crate::I18n;
use crate::viewmodel::{DeleteConfirm, DocAction, TreeState, ViewMode, cell_text};
use eframe::egui;
use mongostudio_core::store::Page;

/// Radio-style mode switcher; returns the (possibly changed) mode.
pub fn show_mode_switcher(ui: &mut egui::Ui, mode: ViewMode, i18n: &I18n) -> ViewMode {
    let mut next = mode;
    ui.horizontal(|ui| {
        for candidate in ViewMode::ALL {
            let label = i18n.t(candidate.label_key());
            if ui.selectable_label(mode == candidate, label).clicked() {
                next = candidate;
            }
        }
    });
    next
}

/// Renders the page in `mode`. Call `state.results_replaced()` whenever a new page arrives.
pub fn show_documents(
    ui: &mut egui::Ui,
    mode: ViewMode,
    page: &Page,
    state: &mut TreeState,
    i18n: &I18n,
) -> Option<DocAction> {
    let mut action = None;
    if ui.button(i18n.t("doc.insert")).clicked() {
        action = Some(DocAction::Insert);
    }
    let from_view = match mode {
        ViewMode::Tree => show_tree(ui, page, state, i18n),
        ViewMode::Table => show_table(ui, page, state, i18n),
        ViewMode::Json => show_json(ui, page, state, i18n),
    };
    let from_dialog = show_delete_confirm(ui.ctx(), state, i18n);
    action.or(from_view).or(from_dialog)
}

/// Modal asking to confirm a parked delete; confirming emits `Delete` exactly once.
fn show_delete_confirm(
    ctx: &egui::Context,
    state: &mut TreeState,
    i18n: &I18n,
) -> Option<DocAction> {
    let DeleteConfirm::Pending(id) = &state.confirm else {
        return None;
    };
    let message = i18n.t_with("doc.confirm_delete", &[("id", &cell_text(id, i18n))]);
    let mut confirmed = false;
    let mut cancelled = false;
    let modal = egui::Modal::new(egui::Id::new("delete_confirm")).show(ctx, |ui| {
        ui.label(message);
        ui.horizontal(|ui| {
            confirmed = ui.button(i18n.t("doc.delete")).clicked();
            cancelled = ui.button(i18n.t("common.cancel")).clicked();
        });
    });
    if confirmed {
        let (next, action) = std::mem::take(state).delete_confirmed();
        *state = next;
        return action;
    }
    if cancelled || modal.should_close() {
        *state = std::mem::take(state).delete_cancelled();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewmodel::MenuItem;
    use bson::{Bson, Document, doc};
    use mongostudio_core::test_support::mock_objects::{exotic_doc, mock_doc_with_id};

    fn page_of(docs: Vec<Document>) -> Page {
        Page {
            docs,
            skip: 0,
            has_more: false,
        }
    }

    fn big_page() -> Page {
        let big: Vec<Bson> = (0..10_000).map(Bson::Int32).collect();
        page_of(vec![
            exotic_doc(),
            doc! { "_id": 1_i32, "big": big, "ключ ✓": "x", "a.b c": { "d": 1_i32 } },
        ])
    }

    /// Runs a few frames and returns every action emitted.
    fn run_frames(mode: ViewMode, page: &Page, state: &mut TreeState) -> Vec<DocAction> {
        let ctx = egui::Context::default();
        let i18n = I18n::english();
        let mut actions = Vec::new();
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    actions.extend(show_documents(ui, mode, page, state, &i18n));
                });
            });
        }
        actions
    }

    #[test]
    fn every_mode_renders_without_panic_or_action() {
        for mode in ViewMode::ALL {
            let mut state = TreeState::default().expanded_toggled(1, &["big".to_string()]);
            assert!(
                run_frames(mode, &big_page(), &mut state).is_empty(),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn empty_page_renders_in_every_mode() {
        for mode in ViewMode::ALL {
            let mut state = TreeState::default();
            assert!(run_frames(mode, &page_of(vec![]), &mut state).is_empty());
        }
    }

    #[test]
    fn pending_delete_dialog_renders_and_emits_nothing_untouched() {
        let page = page_of(vec![mock_doc_with_id(5_i32)]);
        let (mut state, _) = TreeState::default().menu_chosen(MenuItem::Delete, &page.docs[0]);
        assert!(run_frames(ViewMode::Tree, &page, &mut state).is_empty());
        assert_eq!(state.confirm, DeleteConfirm::Pending(Bson::Int32(5)));
    }

    #[test]
    fn switcher_without_interaction_keeps_mode() {
        let ctx = egui::Context::default();
        let i18n = I18n::english();
        let mut result = ViewMode::Table;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                result = show_mode_switcher(ui, ViewMode::Table, &i18n);
            });
        });
        assert_eq!(result, ViewMode::Table);
    }

    #[test]
    fn every_ui_key_exists_in_locale() {
        let i18n = I18n::english();
        let mut keys: Vec<String> = ViewMode::ALL.iter().map(|m| m.label_key().into()).collect();
        keys.extend(
            [
                MenuItem::Edit,
                MenuItem::Clone,
                MenuItem::Delete,
                MenuItem::Insert,
            ]
            .iter()
            .map(|m| m.label_key().to_string()),
        );
        for name in [
            "objectId", "int32", "other", "null", "maxKey", "minKey", "regex",
        ] {
            keys.push(crate::viewmodel::type_label_key(name));
        }
        keys.extend(["doc.header", "doc.confirm_delete", "tree.more_items"].map(String::from));
        for key in keys {
            assert!(!i18n.t(&key).starts_with('⟦'), "missing {key}");
        }
    }
}
