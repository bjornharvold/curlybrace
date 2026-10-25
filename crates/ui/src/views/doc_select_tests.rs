//! Picking documents in the three views and the selection bar, through the accesskit tree.
use super::documents::{DocTarget, show_documents};
use crate::I18n;
use crate::viewmodel::{DocAction, DocCopy, TreeState, ViewMode};
use bson::{Bson, doc};
use curlybrace_core::store::Page;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

struct Pane {
    state: TreeState,
    actions: Vec<DocAction>,
    read_only: bool,
}

fn page() -> Page {
    Page {
        docs: vec![
            doc! { "_id": 1_i32, "name": "a" },
            doc! { "_id": 2_i32, "name": "b" },
        ],
        skip: 0,
        has_more: false,
    }
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn pane(mode: ViewMode, state: TreeState, read_only: bool) -> Harness<'static, Pane> {
    let pane = Pane {
        state,
        actions: Vec::new(),
        read_only,
    };
    let mut harness = Harness::new_ui_state(
        move |ui, pane: &mut Pane| {
            let target = DocTarget {
                connection: "test",
                production: false,
                read_only: pane.read_only,
            };
            let got = show_documents(ui, mode, &page(), &mut pane.state, target, &I18n::english());
            pane.actions.extend(got);
        },
        pane,
    );
    harness.run_steps(3);
    harness
}

/// The label of the checkbox that picks document 2.
fn row_two_label(_mode: ViewMode) -> String {
    I18n::english().t_with("doc.select_row", &[("n", "2")])
}

#[test]
fn ticking_a_row_picks_its_id_in_every_view() {
    for mode in ViewMode::ALL {
        let mut harness = pane(mode, TreeState::default(), false);
        harness.get_by_label(&row_two_label(mode)).click();
        harness.run_steps(3);
        let pick = harness.state().state.pick.clone();
        assert_eq!(
            pick.copy(),
            Some(DocCopy::Ids(vec![Bson::Int32(2)])),
            "{mode:?}"
        );
        harness.get_by_label(&row_two_label(mode)).click();
        harness.run_steps(3);
        assert!(harness.state().state.pick.is_empty(), "{mode:?} unticks");
    }
}

#[test]
fn the_bar_copies_the_ticked_rows_and_nothing_before() {
    let copy = t("copy.menu.copy");
    let mut harness = pane(ViewMode::Tree, TreeState::default(), false);
    assert!(harness.get_by_label(&copy).accesskit_node().is_disabled());
    harness.get_by_label(&row_two_label(ViewMode::Tree)).click();
    harness.run_steps(3);
    let label = I18n::english().t("copy.menu.copy_document");
    harness.get_by_label(&label).click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().actions,
        vec![DocAction::Copy(DocCopy::Ids(vec![Bson::Int32(2)]))]
    );
}

#[test]
fn select_all_copies_the_whole_result_and_clear_resets() {
    let mut harness = pane(ViewMode::Table, TreeState::default(), false);
    harness.get_by_label(&t("doc.select_all_result")).click();
    harness.run_steps(3);
    harness
        .get_by_label(&t("copy.menu.copy_query_result"))
        .click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().actions,
        vec![DocAction::Copy(DocCopy::Result)]
    );
    harness.get_by_label(&t("doc.clear_selection")).click();
    harness.run_steps(3);
    assert!(harness.state().state.pick.is_empty());
}

#[test]
fn copy_is_disabled_for_a_system_collection_even_with_a_pick() {
    let mut state = TreeState::default();
    state.copy_blocked = true;
    let mut harness = pane(ViewMode::Tree, state, false);
    harness.get_by_label(&t("doc.select_all_result")).click();
    harness.run_steps(3);
    let label = t("copy.menu.copy_query_result");
    let copy = harness.get_by_label(&label);
    assert!(copy.accesskit_node().is_disabled());
}

#[test]
fn copy_stays_available_on_a_read_only_connection() {
    let mut harness = pane(ViewMode::Json, TreeState::default(), true);
    harness.get_by_label(&row_two_label(ViewMode::Json)).click();
    harness.run_steps(3);
    let label = t("copy.menu.copy_document");
    let copy = harness.get_by_label(&label);
    assert!(!copy.accesskit_node().is_disabled());
}

#[test]
fn the_row_menu_copy_is_enabled_on_read_only_while_writes_are_not() {
    let mut harness = pane(ViewMode::Json, TreeState::default(), true);
    harness.get_by_label_contains("\"b\"").click_secondary();
    harness.run_steps(4);
    let (copy, edit) = (t("copy.menu.copy_document"), t("doc.edit"));
    assert!(!harness.get_by_label(&copy).accesskit_node().is_disabled());
    assert!(harness.get_by_label(&edit).accesskit_node().is_disabled());
    harness.get_by_label(&copy).click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().actions,
        vec![DocAction::Copy(DocCopy::Ids(vec![Bson::Int32(2)]))]
    );
}
