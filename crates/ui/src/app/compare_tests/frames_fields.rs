//! Headless pseudo-locale frames of the field-level copy: the detail pane with its checkboxes and
//! queue buttons, the pending line of a field copy and the review dialog with its Fields column.
use super::fields::{queued, reviewing_queued, ticked};
use super::frames::{assert_translated, has, workspace};
use super::*;

#[test]
fn the_detail_pane_with_field_checkboxes_and_buttons_is_translated() {
    let texts = assert_translated(&workspace(ticked()), "field detail");
    assert!(has(&texts, "Copy selected fields to target"));
    assert!(has(&texts, "Copy selected fields to source"));
    assert!(has(&texts, "Selected fields: 2"));
    assert!(has(&texts, "Clear selected fields"));
    assert!(has(&texts, "Differences for key"));
}

#[test]
fn the_detail_pane_with_nothing_ticked_is_translated() {
    let state = act(ticked(), CompareAction::ClearFieldSelection).0;
    let texts = assert_translated(&workspace(state), "field detail, nothing ticked");
    assert!(has(&texts, "Selected fields: 0"));
}

#[test]
fn the_detail_pane_of_an_applied_pair_is_translated() {
    let mut state = workspace(ticked());
    state.compare.overview = state.compare.overview.clone().marked_applied(RUN_PAIR);
    let texts = assert_translated(&state, "field detail, applied");
    assert!(has(&texts, "Copy selected fields to target"));
}

#[test]
fn the_pending_line_of_a_field_copy_is_translated() {
    let texts = assert_translated(&workspace(queued()), "field pending line");
    assert!(has(&texts, "Pending changes"));
    assert!(has(&texts, "Fields: 2, documents: 1"));
    assert!(has(&texts, "Documents in total: 1"));
}

#[test]
fn the_review_dialog_with_the_fields_column_is_translated() {
    let texts = assert_translated(&workspace(reviewing_queued()), "field dialog");
    assert!(has(&texts, "Review before applying"));
    assert!(has(&texts, "Fields: 2, documents: 1"));
    assert!(texts.iter().any(|t| t == "«Fields»"));
    assert!(has(&texts, "Copying fields changes only the chosen fields"));
    assert!(!has(&texts, "To confirm the deletes"));
}
