use super::*;
use crate::viewmodel::compare::overview::{PairStatus, RunRow};
use crate::viewmodel::compare::{Counts, PendingList};
use bson::Bson;
use curlybrace_core::compare::engine::Summary;
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::RowView;

const PAIR: PairId = 2;

fn k(n: i32) -> KeyTuple {
    KeyTuple(vec![Bson::Int32(n)])
}

fn row(pair: PairId, status: PairStatus, applied: bool) -> RunRow {
    RunRow {
        entry: 0,
        pair,
        run: 1,
        status,
        last_progress: (0, 0, 0),
        applied,
        facts: None,
    }
}

fn state_with(status: PairStatus, applied: bool) -> CompareState {
    let mut state = CompareState::default();
    state.overview.rows = vec![row(PAIR, status, applied)];
    state
}

fn completed() -> PairStatus {
    PairStatus::Completed(Summary::default())
}

fn results(category: Category, total: usize, loaded: usize) -> ResultsView {
    let rows = (0..loaded as i32)
        .map(|n| RowView {
            key: k(n),
            has_source: true,
            has_target: true,
        })
        .collect();
    ResultsView {
        pair: PAIR,
        counts: Counts {
            only_in_source: total,
            only_in_target: total,
            different: total,
            identical: 0,
        },
        category,
        offset: 0,
        rows,
        requested: None,
        load_failed: false,
    }
}

#[test]
fn a_completed_pair_with_a_selection_enables_every_offered_action() {
    let state = state_with(completed(), false);
    let buttons = state.action_buttons(PAIR, Category::Different, 3);
    let actions: Vec<_> = buttons.iter().map(|b| b.action).collect();
    assert_eq!(
        actions,
        vec![RowAction::CopyToTarget, RowAction::CopyToSource]
    );
    assert!(buttons.iter().all(|b| b.block.is_none()));
}

#[test]
fn nothing_selected_is_the_reason_when_the_pair_is_otherwise_open() {
    let state = state_with(completed(), false);
    assert_eq!(
        state.queue_block(PAIR, 0),
        Some(ActionBlock::NothingSelected)
    );
    assert_eq!(state.tick_block(PAIR), None, "ticking itself stays open");
}

#[test]
fn an_applied_pair_is_closed_for_ticking_and_queueing() {
    let state = state_with(completed(), true);
    assert_eq!(state.tick_block(PAIR), Some(ActionBlock::Applied));
    assert_eq!(state.queue_block(PAIR, 5), Some(ActionBlock::Applied));
    assert!(state.pair_is_applied(PAIR));
}

#[test]
fn a_pair_without_finished_results_is_not_completed() {
    let state = state_with(PairStatus::Pending, false);
    assert_eq!(state.tick_block(PAIR), Some(ActionBlock::NotCompleted));
    assert_eq!(state.tick_block(99), Some(ActionBlock::NotCompleted));
}

#[test]
fn a_busy_apply_wins_over_every_other_reason() {
    let mut state = state_with(completed(), true);
    state.apply = crate::viewmodel::compare::ApplyUi::Running {
        run: 7,
        total: 0,
        rows: Vec::new(),
        cancel_requested: false,
    };
    assert_eq!(state.tick_block(PAIR), Some(ActionBlock::Busy));
    assert_eq!(state.queue_block(PAIR, 5), Some(ActionBlock::Busy));
}

#[test]
fn an_active_run_blocks_queueing_but_not_ticking() {
    let mut state = state_with(completed(), false);
    state.overview.active = Some(crate::viewmodel::compare::ActiveRun {
        run: 2,
        cancel_requested: false,
        failure_noted: false,
        pairs: 1,
    });
    assert!(state.is_running());
    assert_eq!(state.tick_block(PAIR), None);
    assert_eq!(state.queue_block(PAIR, 5), Some(ActionBlock::Running));
}

#[test]
fn every_reason_has_its_own_key() {
    let all = [
        ActionBlock::Busy,
        ActionBlock::Applied,
        ActionBlock::NotCompleted,
        ActionBlock::Running,
        ActionBlock::Checking,
        ActionBlock::NothingSelected,
        ActionBlock::NotOnPage,
    ];
    let keys: std::collections::BTreeSet<_> = all.iter().map(|b| b.i18n_key()).collect();
    assert_eq!(keys.len(), all.len());
}

#[test]
fn buttons_follow_the_category() {
    let state = state_with(completed(), false);
    let of = |c| {
        state
            .action_buttons(PAIR, c, 1)
            .iter()
            .map(|b| b.action)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        of(Category::OnlyInSource),
        vec![RowAction::CopyToTarget, RowAction::DeleteFromSource]
    );
    assert_eq!(
        of(Category::OnlyInTarget),
        vec![RowAction::CopyToSource, RowAction::DeleteFromTarget]
    );
}

#[test]
fn the_bar_counts_the_whole_tab_for_an_all_selection() {
    let state = state_with(completed(), false);
    let state = CompareState {
        selection: Selection::default().all_selected(PAIR, Category::Different),
        ..state
    };
    let view = results(Category::Different, 1_000_000, 100);
    let bar = SelectionBar::of(&state, &view);
    assert_eq!((bar.selected, bar.total), (1_000_000, 1_000_000));
    assert!(bar.all_selected && !bar.can_select_all() && bar.can_clear());
    assert!(bar.actions.iter().all(|a| a.block.is_none()));
}

#[test]
fn an_empty_bar_offers_select_all_but_not_clear() {
    let state = state_with(completed(), false);
    let bar = SelectionBar::of(&state, &results(Category::Different, 250, 100));
    assert!(bar.can_select_all() && !bar.can_clear());
    assert!(
        bar.actions
            .iter()
            .all(|a| a.block == Some(ActionBlock::NothingSelected))
    );
}

#[test]
fn page_tick_needs_every_loaded_row() {
    assert!(!page_ticked(&[]));
    assert!(!page_ticked(&[true, false]));
    assert!(page_ticked(&[true, true]));
}

#[test]
fn pending_documents_count_keys_and_whole_tabs() {
    let (pending, _) = PendingList::default().with_action(
        PAIR,
        Category::OnlyInSource,
        RowAction::CopyToTarget,
        Scope::Keys(vec![k(1), k(2)]),
    );
    let (pending, _) = pending.with_action(
        PAIR,
        Category::Different,
        RowAction::CopyToTarget,
        Scope::Category,
    );
    let state = CompareState {
        pending,
        results: [(PAIR, results(Category::Different, 40, 0))].into(),
        ..state_with(completed(), false)
    };
    assert_eq!(state.pending_documents(PAIR), 42);
    assert_eq!(state.pending_documents(9), 0);
}

#[test]
fn a_million_key_all_selection_costs_nothing_per_visible_row() {
    let view = results(Category::Different, 1_000_000, 100);
    let all = Selection::default().all_selected(PAIR, Category::Different);
    // The `all` flag holds no keys, so there is nothing to scan by construction.
    let ticks = visible_ticks(&all, &view);
    assert!(ticks.iter().all(|t| *t) && ticks.len() == 100);
    assert!(page_ticked(&ticks));
}

#[test]
fn a_million_explicit_keys_are_only_looked_up_for_the_visible_rows() {
    let view = results(Category::Different, 1_000_000, 100);
    // Every even key of a million is ticked; the visible rows are keys 0..100.
    let keys: Vec<KeyTuple> = (0..2_000_000).step_by(2).map(k).collect();
    let big = Selection::default().page_selected(PAIR, Category::Different, &keys);
    assert_eq!(big.count(PAIR, Category::Different, 0), 1_000_000);
    // Correctness only: no wall-clock bound (it would be flaky). `visible_ticks` has no path
    // that iterates the selection.
    let ticks = visible_ticks(&big, &view);
    assert_eq!(ticks.iter().filter(|t| **t).count(), 50);
    assert!(ticks[0] && !ticks[1]);
}

fn open_with_rows(category: Category, loaded: usize) -> CompareState {
    CompareState {
        results: [(PAIR, results(category, 250, loaded))].into(),
        open_pair: Some(PAIR),
        ..state_with(completed(), false)
    }
}

#[test]
fn detail_buttons_are_open_for_a_key_on_the_loaded_page() {
    let state = open_with_rows(Category::Different, 5);
    let buttons = state.detail_buttons(&k(3));
    assert_eq!(buttons.len(), 2);
    assert!(buttons.iter().all(|b| b.block.is_none()));
}

#[test]
fn detail_buttons_are_blocked_for_a_key_off_the_page() {
    let state = open_with_rows(Category::Different, 5);
    let buttons = state.detail_buttons(&k(500));
    assert_eq!(buttons.len(), 2);
    assert!(
        buttons
            .iter()
            .all(|b| b.block == Some(ActionBlock::NotOnPage))
    );
    // A page not loaded yet has no rows: nothing is on it.
    let loading = open_with_rows(Category::Different, 0);
    assert!(
        loading
            .detail_buttons(&k(1))
            .iter()
            .all(|b| b.block == Some(ActionBlock::NotOnPage))
    );
    assert!(CompareState::default().detail_buttons(&k(1)).is_empty());
}
