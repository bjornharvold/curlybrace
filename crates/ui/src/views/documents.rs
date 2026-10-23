//! Document area: mode switcher, insert button, the three views and the delete confirmation.
use super::connection_badge::show_badge;
use super::write_warning::focus_cancel_unless_focused;
use super::{json::show_json, table::show_table, tree::show_tree};
use crate::I18n;
use crate::viewmodel::connection_tag::READ_ONLY_REASON_KEY;
use crate::viewmodel::{
    ConnectionBadge, DocAction, PageKey, TreeState, ViewMode, id_preview, page_key,
};
use curlybrace_core::store::Page;
use eframe::egui;

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
    target: DocTarget<'_>,
    i18n: &I18n,
) -> Option<DocAction> {
    let key = page_key(page);
    show_documents_keyed(ui, mode, page, &key, state, target, i18n)
}

/// The connection the shown documents belong to: named in the delete confirmation, which also
/// carries the production notice when the connection is flagged production.
#[derive(Clone, Copy, Debug)]
pub struct DocTarget<'a> {
    pub connection: &'a str,
    pub production: bool,
    /// Writes are off: Insert is disabled with a stated reason and row actions are listed disabled.
    pub read_only: bool,
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
    target: DocTarget<'_>,
    i18n: &I18n,
) -> Option<DocAction> {
    let key = *key;
    state.synced_to(&key);
    state.read_only = target.read_only;
    let action = show_insert(ui, target.read_only, i18n);
    let from_view = match mode {
        ViewMode::Tree => show_tree(ui, page, &key, state, i18n),
        ViewMode::Table => show_table(ui, page, &key, state, i18n),
        ViewMode::Json => show_json(ui, page, &key, state, i18n),
    };
    let from_dialog = show_delete_confirm(ui.ctx(), &key, state, target, i18n);
    action.or(from_view).or(from_dialog)
}

/// The Insert button. On a read-only connection it is disabled and the reason is both its hover
/// text and a visible note, so the state is never colour or hover alone.
fn show_insert(ui: &mut egui::Ui, read_only: bool, i18n: &I18n) -> Option<DocAction> {
    let insert = ui.add_enabled(!read_only, egui::Button::new(i18n.t("doc.insert")));
    if read_only {
        insert.on_disabled_hover_text(i18n.t(READ_ONLY_REASON_KEY));
        ui.label(i18n.t(READ_ONLY_REASON_KEY));
        return None;
    }
    insert.clicked().then_some(DocAction::Insert)
}

/// The one-line header naming the connection whose collection is shown.
pub fn show_connection_header(
    ui: &mut egui::Ui,
    connection: &str,
    badge: &ConnectionBadge,
    i18n: &I18n,
) {
    ui.horizontal(|ui| {
        show_badge(ui, badge, i18n);
        ui.label(i18n.t_with("doc.connection", &[("name", connection)]));
    });
}

/// Modal asking to confirm a parked delete; confirming emits `Delete` exactly once.
fn show_delete_confirm(
    ctx: &egui::Context,
    key: &PageKey,
    state: &mut TreeState,
    target: DocTarget<'_>,
    i18n: &I18n,
) -> Option<DocAction> {
    let id = state.pending_delete_id(key)?;
    let message = i18n.t_with(
        "doc.confirm_delete",
        &[("id", &id_preview(id)), ("connection", target.connection)],
    );
    let mut confirmed = false;
    let mut cancelled = false;
    let modal = egui::Modal::new(egui::Id::new("delete_confirm")).show(ctx, |ui| {
        ui.label(message);
        if target.production {
            // The notice rides in this one dialog: delete never opens a second warning.
            ui.label(
                egui::RichText::new(i18n.t("write_warning.production_line"))
                    .strong()
                    .color(ui.visuals().error_fg_color),
            );
        }
        ui.horizontal(|ui| {
            if target.production {
                // Cancel first and focused: a stray Enter can never delete on production.
                let cancel = ui.button(i18n.t("common.cancel"));
                let delete = ui.button(i18n.t("doc.delete"));
                focus_cancel_unless_focused(&cancel, &delete);
                cancelled = cancel.clicked();
                confirmed = delete.clicked();
            } else {
                confirmed = ui.button(i18n.t("doc.delete")).clicked();
                cancelled = ui.button(i18n.t("common.cancel")).clicked();
            }
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
    use curlybrace_core::test_support::mock_objects::{exotic_doc, mock_doc_with_id};

    fn test_target() -> DocTarget<'static> {
        DocTarget {
            connection: "test",
            production: false,
            read_only: false,
        }
    }

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
                    actions.extend(show_documents(ui, mode, page, state, test_target(), &i18n));
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
                        assert_eq!(
                            show_documents(ui, mode, page, &mut state, test_target(), &i18n),
                            None
                        );
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
    fn delete_dialog_shows_production_line_only_for_production() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let page = page_of(vec![mock_doc_with_id(5_i32)]);
        let line = I18n::english().t("write_warning.production_line");
        for production in [false, true] {
            let (state, _) =
                TreeState::default().menu_chosen(MenuItem::Delete, &page.docs[0], &page_key(&page));
            let mut harness = Harness::new_ui_state(
                |ui, state: &mut TreeState| {
                    let i18n = I18n::english();
                    let target = DocTarget {
                        connection: "prod",
                        production,
                        read_only: false,
                    };
                    show_documents(ui, ViewMode::Tree, &page, state, target, &i18n);
                },
                state,
            );
            harness.run_steps(4);
            assert_eq!(
                harness.query_by_label(&line).is_some(),
                production,
                "production={production}"
            );
            // One dialog either way: the delete button is there exactly once.
            let delete = I18n::english().t("doc.delete");
            assert_eq!(harness.get_all_by_label(&delete).count(), 1);
        }
    }

    /// The delete dialog of `production` or plain connection, a few frames in.
    fn delete_dialog(production: bool) -> egui_kittest::Harness<'static, TreeState> {
        let page = page_of(vec![mock_doc_with_id(5_i32)]);
        let (state, _) =
            TreeState::default().menu_chosen(MenuItem::Delete, &page.docs[0], &page_key(&page));
        let mut harness = egui_kittest::Harness::new_ui_state(
            move |ui, state: &mut TreeState| {
                let target = DocTarget {
                    connection: "prod",
                    production,
                    read_only: false,
                };
                show_documents(
                    ui,
                    ViewMode::Tree,
                    &page_of(vec![mock_doc_with_id(5_i32)]),
                    state,
                    target,
                    &I18n::english(),
                );
            },
            state,
        );
        harness.run_steps(4);
        harness
    }

    #[test]
    fn the_production_delete_dialog_is_cancel_first_and_cancel_focused() {
        use egui_kittest::kittest::{NodeT, Queryable};
        let i18n = I18n::english();
        let mut harness = delete_dialog(true);
        let (cancel_label, delete_label) = (i18n.t("common.cancel"), i18n.t("doc.delete"));
        {
            let cancel = harness.get_by_label(&cancel_label);
            let delete = harness.get_by_label(&delete_label);
            assert!(cancel.rect().left() < delete.rect().left(), "Cancel first");
            assert!(cancel.accesskit_node().is_focused());
            assert!(!delete.accesskit_node().is_focused());
        }
        harness.key_press(egui::Key::Enter);
        harness.run_steps(4);
        assert_eq!(
            harness.state().confirm,
            crate::viewmodel::DeleteConfirm::None,
            "Enter cancelled instead of deleting"
        );
    }

    #[test]
    fn the_plain_delete_dialog_keeps_delete_first_without_forced_focus() {
        use egui_kittest::kittest::{NodeT, Queryable};
        let i18n = I18n::english();
        let harness = delete_dialog(false);
        let (cancel_label, delete_label) = (i18n.t("common.cancel"), i18n.t("doc.delete"));
        let cancel = harness.get_by_label(&cancel_label);
        let delete = harness.get_by_label(&delete_label);
        assert!(delete.rect().left() < cancel.rect().left(), "Delete first");
        assert!(!cancel.accesskit_node().is_focused());
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
            let name = curlybrace_core::ejson::bson_type_name(&sample);
            keys.push(crate::viewmodel::type_label_key(name));
        }
        keys.extend(["doc.header", "doc.confirm_delete", "tree.more_items"].map(String::from));
        for key in keys {
            assert!(!i18n.t(&key).starts_with('⟦'), "missing {key}");
        }
    }
}
