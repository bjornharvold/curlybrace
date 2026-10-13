//! Selection and pending changes of the Compare reducer. Ticking rows and queueing writes only
//! changes view-model state: nothing here sends a runtime command. All of it acts on the open
//! results tab, only while the pair is completed and not yet applied, and never while an apply
//! is being reviewed or runs (the lists are read-only then).
use super::compare::{none, with_compare};
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::compare::{
    CompareState, PendingList, ResultsView, RowAction, Scope, Selection,
};
use curlybrace_core::compare::key::{KeyTuple, compare_tuples};
use curlybrace_core::compare::protocol::PairId;
use curlybrace_core::compare::results::Category;
use std::cmp::Ordering;

/// The results tab the person can tick rows in.
struct Tab<'a> {
    pair: PairId,
    cat: Category,
    view: &'a ResultsView,
}

impl Tab<'_> {
    fn loaded_keys(&self) -> Vec<KeyTuple> {
        self.view.rows.iter().map(|r| r.key.clone()).collect()
    }

    fn has_row(&self, key: &KeyTuple) -> bool {
        // By value: a key holding NaN is not `==` to itself.
        self.view
            .rows
            .iter()
            .any(|r| compare_tuples(&r.key, key) == Ordering::Equal)
    }
}

/// The open tab, if ticking and queueing are allowed on it right now.
fn open_tab(c: &CompareState) -> Option<Tab<'_>> {
    if c.apply.is_busy() {
        return None;
    }
    let view = c.open_results()?;
    c.pair_is_applicable(view.pair).then_some(Tab {
        pair: view.pair,
        cat: view.category,
        view,
    })
}

fn with_selection(state: AppState, selection: Selection) -> Out {
    let compare = CompareState {
        selection,
        ..state.compare.clone()
    };
    none(with_compare(state, compare))
}

fn with_pending(state: AppState, pending: PendingList) -> Out {
    let compare = CompareState {
        pending,
        ..state.compare.clone()
    };
    none(with_compare(state, compare))
}

/// Ticks or unticks a row of the loaded page; a key that is not loaded is ignored.
pub(super) fn toggle_row(state: AppState, key: KeyTuple) -> Out {
    let c = &state.compare;
    let Some(tab) = open_tab(c).filter(|t| t.has_row(&key)) else {
        return none(state);
    };
    let selection = c
        .selection
        .toggled(tab.pair, tab.cat, &key, &tab.loaded_keys());
    with_selection(state, selection)
}

pub(super) fn select_page(state: AppState) -> Out {
    let c = &state.compare;
    let Some(tab) = open_tab(c).filter(|t| !t.view.rows.is_empty()) else {
        return none(state);
    };
    let selection = c
        .selection
        .page_selected(tab.pair, tab.cat, &tab.loaded_keys());
    with_selection(state, selection)
}

/// Every row of the tab, loaded or not; a tab with no rows has nothing to tick.
pub(super) fn select_all(state: AppState) -> Out {
    let c = &state.compare;
    let Some(tab) = open_tab(c).filter(|t| t.view.counts.of(t.cat) > 0) else {
        return none(state);
    };
    let selection = c.selection.all_selected(tab.pair, tab.cat);
    with_selection(state, selection)
}

pub(super) fn clear_selection(state: AppState) -> Out {
    let c = &state.compare;
    let Some(tab) = open_tab(c) else {
        return none(state);
    };
    let selection = c.selection.cleared(tab.pair, tab.cat);
    with_selection(state, selection)
}

/// What the tab's ticks stand for: the whole category, or the ticked keys.
fn ticked_scope(c: &CompareState, tab: &Tab) -> Option<Scope> {
    let ticked = c.selection.get(tab.pair, tab.cat)?;
    match (ticked.all, ticked.keys.is_empty()) {
        (true, _) => Some(Scope::Category),
        (false, true) => None,
        (false, false) => Some(Scope::Keys(
            ticked.keys.iter().map(|k| k.0.clone()).collect(),
        )),
    }
}

/// Queues `action` for the ticked rows and clears the ticks. Refused when nothing is ticked,
/// when the tab does not offer the action, or when a category item already covers the keys
/// (then nothing changes, so the ticks stay and show that nothing happened).
pub(super) fn add_selection(state: AppState, action: RowAction) -> Out {
    let c = &state.compare;
    let Some(tab) = open_tab(c).filter(|t| RowAction::offered_for(t.cat).contains(&action)) else {
        return none(state);
    };
    let Some(scope) =
        ticked_scope(c, &tab).filter(|s| *s != Scope::Category || tab.view.counts.of(tab.cat) > 0)
    else {
        return none(state);
    };
    let (pending, ignored) = c.pending.with_action(tab.pair, tab.cat, action, scope);
    if ignored > 0 {
        return none(state);
    }
    let selection = c.selection.cleared(tab.pair, tab.cat);
    let compare = CompareState {
        pending,
        selection,
        ..c.clone()
    };
    none(with_compare(state, compare))
}

/// Queues `action` for one row of the loaded page (the per-document button of the detail). The
/// row must be on the loaded page of the open tab, which also ties its category to the tab.
pub(super) fn add_key(state: AppState, key: KeyTuple, action: RowAction) -> Out {
    let c = &state.compare;
    let Some(tab) =
        open_tab(c).filter(|t| t.has_row(&key) && RowAction::offered_for(t.cat).contains(&action))
    else {
        return none(state);
    };
    let (pending, ignored) =
        c.pending
            .with_action(tab.pair, tab.cat, action, Scope::Keys(vec![key]));
    if ignored > 0 {
        return none(state);
    }
    with_pending(state, pending)
}

pub(super) fn remove_pending(state: AppState, index: usize) -> Out {
    let c = &state.compare;
    if c.apply.is_busy() || index >= c.pending.items().len() {
        return none(state);
    }
    let pending = c.pending.without(index);
    with_pending(state, pending)
}

pub(super) fn clear_pending(state: AppState) -> Out {
    let c = &state.compare;
    if c.apply.is_busy() {
        return none(state);
    }
    let pending = c.pending.cleared();
    with_pending(state, pending)
}
