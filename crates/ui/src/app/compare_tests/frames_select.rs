//! Headless pseudo-locale frames of the selection bar, the row checkboxes, the per-document
//! actions and the applied note: every string comes from the translation layer.
use super::confirm::{pending_for, tab_of, toggled};
use super::frames::{assert_translated, has, workspace};
use super::*;
use crate::viewmodel::compare::RowAction;

#[test]
fn the_selection_bar_and_its_actions_are_translated() {
    let state = workspace(toggled(tab_of(Category::OnlyInSource), &[0]));
    let texts = assert_translated(&state, "selection bar");
    assert!(has(&texts, "Selected: 1"));
    assert!(has(&texts, "Select all 3 in this tab"));
    assert!(has(&texts, "Clear selection"));
    assert!(has(&texts, "Copy to target"));
    assert!(has(&texts, "Delete from source"));
}

#[test]
fn an_empty_selection_frame_is_translated() {
    let texts = assert_translated(&workspace(tab_of(Category::Different)), "no selection");
    assert!(has(&texts, "Selected: 0"));
}

#[test]
fn the_pending_badge_is_translated() {
    let state = workspace(pending_for(Category::Different, RowAction::CopyToTarget));
    let texts = assert_translated(&state, "pending badge");
    assert!(has(&texts, "Pending changes: 2"));
}

#[test]
fn an_applied_pair_shows_the_translated_note() {
    let mut state = workspace(tab_of(Category::Different));
    state.compare.overview = state.compare.overview.clone().marked_applied(RUN_PAIR);
    let texts = assert_translated(&state, "applied");
    assert!(has(&texts, "Changes were applied to this pair"));
}

#[test]
fn a_detail_off_the_page_renders_with_its_actions_blocked_and_translated() {
    let mut state = workspace(tab_of(Category::Different));
    let off_page = KeyTuple(vec![Bson::Int32(99)]);
    state.compare.detail = Some(crate::viewmodel::compare::DetailView::loading(off_page));
    let buttons = state
        .compare
        .detail_buttons(&KeyTuple(vec![Bson::Int32(99)]));
    assert!(buttons.iter().all(|b| b.block.is_some()));
    let texts = assert_translated(&state, "detail off page");
    assert!(has(&texts, "Copy to source"));
}

#[test]
fn the_detail_actions_are_translated() {
    let key = KeyTuple(vec![Bson::Int32(1)]);
    let (state, _) = act(tab_of(Category::Different), CompareAction::Select(key));
    let texts = assert_translated(&workspace(state), "detail actions");
    assert!(has(&texts, "Copy to source"));
}
