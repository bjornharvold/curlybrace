//! Document area: mode switcher, insert button, the three views and the delete confirmation.
use super::{json::show_json, table::show_table, tree::show_tree};
use crate::I18n;
use crate::viewmodel::{DocAction, PageKey, TreeState, ViewMode, id_preview, page_key};
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

/// Renders the page in `mode`.
///
/// Task 16 carry item: the reducer should still call `TreeState::results_replaced()` on
/// every Results event, including the refresh after a save. The content-sensitive page key
/// (`viewmodel::page_key`, computed once per frame here and synced into the state for every
/// view mode) is the safety net that keeps stale rows, JSON, expansion or a pending delete
/// from being shown or acted on when that call is missed.
pub fn show_documents(
    ui: &mut egui::Ui,
    mode: ViewMode,
    page: &Page,
    state: &mut TreeState,
    i18n: &I18n,
) -> Option<DocAction> {
    show_documents_keyed(ui, mode, page, &page_key(page), state, i18n)
}

/// Like [`show_documents`] with the page key supplied by the caller, which memoises it once
/// per result page instead of rehashing the whole page every frame. `key` MUST be
/// `page_key(page)`.
pub fn show_documents_keyed(
    ui: &mut egui::Ui,
    mode: ViewMode,
    page: &Page,
    key: &PageKey,
    state: &mut TreeState,
    i18n: &I18n,
) -> Option<DocAction> {
    let key = *key;
    state.synced_to(&key);
    let mut action = None;
    if ui.button(i18n.t("doc.insert")).clicked() {
        action = Some(DocAction::Insert);
    }
    let from_view = match mode {
        ViewMode::Tree => show_tree(ui, page, &key, state, i18n),
        ViewMode::Table => show_table(ui, page, &key, state, i18n),
        ViewMode::Json => show_json(ui, page, &key, state, i18n),
    };
    let from_dialog = show_delete_confirm(ui.ctx(), &key, state, i18n);
    action.or(from_view).or(from_dialog)
}

/// Modal asking to confirm a parked delete; confirming emits `Delete` exactly once.
fn show_delete_confirm(
    ctx: &egui::Context,
    key: &PageKey,
    state: &mut TreeState,
    i18n: &I18n,
) -> Option<DocAction> {
    let id = state.pending_delete_id(key)?;
    let message = i18n.t_with("doc.confirm_delete", &[("id", &id_preview(id))]);
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
        let (next, action) = std::mem::take(state).delete_confirmed(key);
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
        let key = page_key(&page);
        let (mut state, _) =
            TreeState::default().menu_chosen(MenuItem::Delete, &page.docs[0], &key);
        assert!(run_frames(ViewMode::Tree, &page, &mut state).is_empty());
        assert_eq!(state.pending_delete_id(&key), Some(&Bson::Int32(5)));
    }

    #[test]
    fn smaller_page_after_a_larger_one_renders_without_panic_or_stale_rows() {
        for mode in ViewMode::ALL {
            let big = big_page();
            let small = page_of(vec![mock_doc_with_id(1_i32)]);
            let mut state = TreeState::default().expanded_toggled(1, &["big".to_string()]);
            let ctx = egui::Context::default();
            let i18n = I18n::english();
            for page in [&big, &small, &big, &small] {
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        assert_eq!(show_documents(ui, mode, page, &mut state, &i18n), None);
                    });
                });
            }
            let rows = state.page_rows(&small, &page_key(&small));
            assert!(
                rows.iter().all(|r| r.doc_index() < small.docs.len()),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn stale_pending_delete_on_a_new_page_shows_no_dialog_and_cannot_confirm() {
        let old = page_of(vec![mock_doc_with_id(5_i32)]);
        let new = page_of(vec![mock_doc_with_id(6_i32)]);
        for mode in ViewMode::ALL {
            let (mut state, _) =
                TreeState::default().menu_chosen(MenuItem::Delete, &old.docs[0], &page_key(&old));
            assert!(run_frames(mode, &new, &mut state).is_empty(), "{mode:?}");
            let key = page_key(&new);
            assert_eq!(state.pending_delete_id(&key), None, "{mode:?}");
            assert_eq!(
                state.confirm,
                crate::viewmodel::DeleteConfirm::None,
                "{mode:?}"
            );
            assert_eq!(state.delete_confirmed(&key).1, None);
        }
    }

    #[test]
    fn value_only_change_clears_a_pending_delete_in_every_mode() {
        let before = page_of(vec![doc! { "_id": 5_i32, "name": "alice" }]);
        let after = page_of(vec![doc! { "_id": 5_i32, "name": "alicia" }]);
        for mode in ViewMode::ALL {
            let (mut state, _) = TreeState::default().menu_chosen(
                MenuItem::Delete,
                &before.docs[0],
                &page_key(&before),
            );
            assert!(run_frames(mode, &after, &mut state).is_empty());
            assert_eq!(
                state.confirm,
                crate::viewmodel::DeleteConfirm::None,
                "{mode:?}"
            );
        }
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

    /// One sample of every BSON variant, including ones `bson_type_name` maps to "other".
    fn every_bson_kind() -> Vec<Bson> {
        let mut kinds: Vec<Bson> = exotic_doc().into_iter().map(|(_, v)| v).collect();
        kinds.extend([
            Bson::String("s".into()),
            Bson::Boolean(true),
            Bson::Null,
            Bson::Undefined,
            Bson::Symbol("y".into()),
            Bson::JavaScriptCode("1".into()),
        ]);
        kinds
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
        for sample in every_bson_kind() {
            let name = mongostudio_core::ejson::bson_type_name(&sample);
            keys.push(crate::viewmodel::type_label_key(name));
        }
        keys.extend(["doc.header", "doc.confirm_delete", "tree.more_items"].map(String::from));
        for key in keys {
            assert!(!i18n.t(&key).starts_with('⟦'), "missing {key}");
        }
    }
}
