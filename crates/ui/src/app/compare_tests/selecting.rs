//! Ticking rows, the pending list and what clears them.
use super::confirm::{key, pending_for, rows_of, tab, tab_of, toggled, with_pending};
use super::*;
use crate::viewmodel::compare::{RowAction, Scope};

fn selected(state: &AppState, cat: Category, i: i32) -> bool {
    state.compare.selection.is_selected(RUN_PAIR, cat, &key(i))
}

const ALL_ACTIONS: [RowAction; 4] = [
    RowAction::CopyToTarget,
    RowAction::CopyToSource,
    RowAction::DeleteFromSource,
    RowAction::DeleteFromTarget,
];

#[test]
fn toggle_ticks_and_unticks_a_loaded_row() {
    let state = toggled(tab_of(Category::Different), &[1]);
    assert!(selected(&state, Category::Different, 1));
    assert!(!selected(&state, Category::Different, 0));
    let state = toggled(state, &[1]);
    assert!(!selected(&state, Category::Different, 1));
}

#[test]
fn a_key_that_is_not_on_the_loaded_page_cannot_be_ticked() {
    let (state, _) = act(
        tab_of(Category::Different),
        CompareAction::ToggleRow(key(99)),
    );
    assert!(
        state
            .compare
            .selection
            .is_empty_for(RUN_PAIR, Category::Different)
    );
}

#[test]
fn select_page_all_in_category_and_clear() {
    let state = tab(Category::Different, 3, true);
    let (state, cmds) = act(state, CompareAction::SelectPage);
    assert!(cmds.is_empty());
    assert!((0..3).all(|i| selected(&state, Category::Different, i)));
    let (state, _) = act(state, CompareAction::ClearSelection);
    assert!(
        state
            .compare
            .selection
            .is_empty_for(RUN_PAIR, Category::Different)
    );
    let (state, _) = act(state, CompareAction::SelectAllInCategory);
    assert!(
        selected(&state, Category::Different, 77),
        "all covers unloaded rows"
    );
    assert_eq!(
        state
            .compare
            .selection
            .count(RUN_PAIR, Category::Different, 250),
        250
    );
}

#[test]
fn unticking_under_all_keeps_the_other_loaded_rows() {
    let (state, _) = act(
        tab_of(Category::Different),
        CompareAction::SelectAllInCategory,
    );
    let state = toggled(state, &[1]);
    assert!(selected(&state, Category::Different, 0));
    assert!(!selected(&state, Category::Different, 1));
    assert!(selected(&state, Category::Different, 2));
    assert!(
        !selected(&state, Category::Different, 77),
        "unloaded rows drop out"
    );
}

#[test]
fn selection_survives_paging_and_a_category_switch() {
    let two = Summary {
        different: 150,
        only_in_source: 2,
        ..Summary::default()
    };
    let (state, _) = update(running_state(), pair_done(1, two));
    let (state, _) = update(state, rows_of(Category::Different, 100));
    let (state, _) = update(state, run_done());
    let state = toggled(state, &[5]);
    let (state, _) = act(state, CompareAction::Page(1));
    let (state, _) = act(state, CompareAction::SelectCategory(Category::OnlyInSource));
    let (state, _) = act(state, CompareAction::SelectCategory(Category::Different));
    assert!(selected(&state, Category::Different, 5));
}

#[test]
fn selection_is_dropped_with_the_results() {
    let ticked = || toggled(tab_of(Category::Different), &[0]);
    let (rerun, _) = act(ticked(), CompareAction::Rerun(ENTRY));
    let (discard, _) = act(ticked(), CompareAction::DiscardPair(ENTRY));
    let (remove, _) = act(ticked(), CompareAction::RemovePair(ENTRY));
    let (closed, _) = act(ticked(), CompareAction::CloseSession);
    let (started, _) = act(ticked(), CompareAction::Start);
    for state in [rerun, discard, remove, closed, started] {
        assert!(
            state
                .compare
                .selection
                .is_empty_for(RUN_PAIR, Category::Different)
        );
    }
}

#[test]
fn a_late_rows_page_for_another_pair_leaves_the_selection_alone() {
    let state = toggled(tab_of(Category::Different), &[1]);
    let late = ev(CompareEvent::Rows {
        session: 1,
        pair: 99,
        category: Category::Different,
        offset: 0,
        total: 1,
        rows: vec![],
    });
    let (after, _) = update(copy_of(&state), late);
    assert_eq!(after.compare.selection, state.compare.selection);
}

#[test]
fn only_the_offered_actions_can_be_queued_for_each_category() {
    for cat in [
        Category::OnlyInSource,
        Category::OnlyInTarget,
        Category::Different,
    ] {
        for action in ALL_ACTIONS {
            let state = toggled(tab_of(cat), &[0]);
            let (state, cmds) = act(state, CompareAction::AddToPending(action));
            assert!(cmds.is_empty());
            let offered = RowAction::offered_for(cat).contains(&action);
            assert_eq!(
                !state.compare.pending.is_empty(),
                offered,
                "{cat:?} {action:?}"
            );
            assert_eq!(state.compare.selection.is_empty_for(RUN_PAIR, cat), offered);
        }
    }
}

#[test]
fn queueing_clears_the_selection_and_nothing_selected_queues_nothing() {
    let state = with_pending();
    assert!(
        state
            .compare
            .selection
            .is_empty_for(RUN_PAIR, Category::Different)
    );
    let (state, _) = act(state, CompareAction::AddToPending(RowAction::CopyToSource));
    assert_eq!(state.compare.pending.items().len(), 1);
}

#[test]
fn the_same_rows_with_another_action_replace_the_earlier_one() {
    let state = toggled(with_pending(), &[1]);
    let (state, _) = act(state, CompareAction::AddToPending(RowAction::CopyToSource));
    let items = state.compare.pending.items();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].scope, Scope::Keys(vec![key(0)]));
    assert_eq!(items[1].action, RowAction::CopyToSource);
}

#[test]
fn all_in_category_becomes_a_category_item_that_replaces_key_items() {
    let state = with_pending();
    let (state, _) = act(state, CompareAction::SelectAllInCategory);
    let (state, _) = act(state, CompareAction::AddToPending(RowAction::CopyToSource));
    let items = state.compare.pending.items();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].scope, Scope::Category);
    assert!(
        state
            .compare
            .selection
            .is_empty_for(RUN_PAIR, Category::Different)
    );
}

#[test]
fn keys_under_a_category_item_are_ignored_and_stay_selected() {
    let (state, _) = act(
        tab_of(Category::Different),
        CompareAction::SelectAllInCategory,
    );
    let (state, _) = act(state, CompareAction::AddToPending(RowAction::CopyToTarget));
    let state = toggled(state, &[0]);
    let (after, _) = act(
        copy_of(&state),
        CompareAction::AddToPending(RowAction::CopyToSource),
    );
    assert_eq!(after.compare.pending, state.compare.pending);
    assert!(
        selected(&after, Category::Different, 0),
        "nothing happened, so nothing cleared"
    );
}

#[test]
fn an_applied_or_unfinished_pair_cannot_be_queued() {
    let mut applied = toggled(tab_of(Category::Different), &[0]);
    applied.compare.overview = applied.compare.overview.clone().marked_applied(RUN_PAIR);
    let (applied, _) = act(
        applied,
        CompareAction::AddToPending(RowAction::CopyToTarget),
    );
    assert!(applied.compare.pending.is_empty());
    let (ticking, _) = act(copy_of(&applied), CompareAction::ToggleRow(key(1)));
    assert!(
        !selected(&ticking, Category::Different, 1),
        "an applied pair is read-only"
    );
}

#[test]
fn add_key_queues_one_loaded_row_and_refuses_the_rest() {
    let state = tab_of(Category::Different);
    let add = |s, k, a| act(s, CompareAction::AddKeyToPending(k, a)).0;
    let queued = add(copy_of(&state), key(2), RowAction::CopyToSource);
    assert_eq!(
        queued.compare.pending.items()[0].scope,
        Scope::Keys(vec![key(2)])
    );
    assert!(
        add(copy_of(&state), key(99), RowAction::CopyToSource)
            .compare
            .pending
            .is_empty()
    );
    assert!(
        add(copy_of(&state), key(2), RowAction::DeleteFromSource)
            .compare
            .pending
            .is_empty()
    );
}

#[test]
fn add_key_is_refused_for_a_row_of_another_category_than_the_open_tab() {
    // Rows of the other tab are not loaded, so a detail left open across a tab switch cannot
    // queue its key under the wrong category.
    let both = Summary {
        different: 3,
        only_in_source: 3,
        ..Summary::default()
    };
    let (state, _) = update(running_state(), pair_done(1, both));
    let (state, _) = update(state, rows_of(Category::OnlyInSource, 3));
    let (state, _) = update(state, run_done());
    let (state, _) = act(state, CompareAction::SelectCategory(Category::Different));
    let (state, _) = act(
        state,
        CompareAction::AddKeyToPending(key(0), RowAction::CopyToTarget),
    );
    assert!(
        state.compare.pending.is_empty(),
        "no Different rows loaded yet"
    );
}

#[test]
fn remove_and_clear_pending() {
    let state = toggled(with_pending(), &[2]);
    let (state, _) = act(state, CompareAction::AddToPending(RowAction::CopyToSource));
    assert_eq!(state.compare.pending.items().len(), 2);
    let (state, _) = act(state, CompareAction::RemovePending(9));
    assert_eq!(state.compare.pending.items().len(), 2);
    let (state, _) = act(state, CompareAction::RemovePending(0));
    assert_eq!(state.compare.pending.items().len(), 1);
    let (state, _) = act(state, CompareAction::ClearPending);
    assert!(state.compare.pending.is_empty());
}

#[test]
fn pending_is_dropped_with_the_results() {
    for action in [
        CompareAction::Rerun(ENTRY),
        CompareAction::DiscardPair(ENTRY),
        CompareAction::RemovePair(ENTRY),
        CompareAction::CloseSession,
        CompareAction::Start,
    ] {
        let (state, _) = act(
            pending_for(Category::Different, RowAction::CopyToTarget),
            action.clone(),
        );
        assert!(state.compare.pending.is_empty(), "{action:?}");
    }
}
