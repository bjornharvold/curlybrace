//! Results of the Compare reducer: opening a pair's results, paging and selecting within them,
//! and discarding or re-running a pair. `Rows` and `Detail` are taken only for the exact
//! request that is outstanding on a pair that still has results.
use super::compare::{cmd, none, ready, with_compare};
use super::compare_pairs::discards;
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::compare::{CompareState, DetailView, EntryId, PairStatus, ResultsView};
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::{PairId, RowView};
use curlybrace_core::compare::results::Category;

/// Shows one entry's results (its row must be completed and still hold them). Switching pair
/// clears the detail; a view with no page loaded asks for its current one.
pub(super) fn open_pair(state: AppState, entry: EntryId) -> Out {
    let c = &state.compare;
    let Some(row) = c.overview.row_for(entry) else {
        return none(state);
    };
    let (PairStatus::Completed(_), Some(view)) = (&row.status, c.results.get(&row.pair)) else {
        return none(state);
    };
    let pair = row.pair;
    let refetch = view.refetched();
    let commands = match (ready(&state), &refetch) {
        (Some(session), Some(next)) => next.request(session).into_iter().map(cmd).collect(),
        _ => Vec::new(),
    };
    let switched = c.open_pair != Some(pair);
    let opened = CompareState {
        open_pair: Some(pair),
        detail: if switched { None } else { c.detail.clone() },
        ..c.clone()
    };
    let compare = match refetch {
        Some(next) => opened.results_updated(pair, |_| next),
        None => opened,
    };
    (with_compare(state, compare), commands)
}

/// Back to the run overview: no pair open and no detail.
pub(super) fn back_to_overview(state: AppState) -> Out {
    let compare = CompareState {
        open_pair: None,
        detail: None,
        ..state.compare.clone()
    };
    none(with_compare(state, compare))
}

/// Category switch or paging: `change` yields the next results, whose page is then requested.
pub(super) fn moved(
    state: AppState,
    change: impl FnOnce(&ResultsView) -> Option<ResultsView>,
) -> Out {
    let (Some(session), Some(results)) = (ready(&state), state.compare.open_results()) else {
        return none(state);
    };
    let Some(next) = change(results) else {
        return none(state);
    };
    let commands = next.request(session).into_iter().map(cmd).collect();
    // The open detail belongs to a row of the tab and page just left. A page arrives later, so
    // it is dropped on any category or page change rather than checked against rows not yet
    // loaded (a late `Detail` for it is then ignored).
    let compare = CompareState {
        detail: None,
        ..state.compare.clone()
    }
    .results_updated(next.pair, |_| next);
    (with_compare(state, compare), commands)
}

pub(super) fn select(state: AppState, key: KeyTuple) -> Out {
    let (Some(session), Some(results)) = (ready(&state), state.compare.open_results()) else {
        return none(state);
    };
    let Some(command) = results.select(session, &key) else {
        return none(state);
    };
    let compare = CompareState {
        detail: Some(DetailView::loading(key)),
        ..state.compare.clone()
    };
    (with_compare(state, compare), vec![cmd(command)])
}

/// Throws away a finished pair's results: `Discard` for the row's current `PairId`, the view
/// gone, the row marked discarded. A pair still queued or running cannot be discarded.
pub(super) fn discard_pair(state: AppState, entry: EntryId) -> Out {
    let c = &state.compare;
    let Some(row) = c
        .overview
        .row_for(entry)
        .filter(|_| c.can_discard_entry(entry))
    else {
        return none(state);
    };
    let pair = row.pair;
    let commands = discards(ready(&state), &[pair]);
    let compare = CompareState {
        overview: c.overview.clone().discarded(pair),
        duplicates: c.duplicates.superseded_for(entry),
        ..c.clone()
    }
    .without_results(&[pair]);
    (with_compare(state, compare), commands)
}

/// A row that holds nothing to discard: still live, or already discarded.
fn is_idle(status: PairStatus) -> bool {
    matches!(
        status,
        PairStatus::Pending | PairStatus::Running { .. } | PairStatus::Discarded
    )
}

/// Re-run: only this entry stays included and opens in the editor, the verdicts go stale, its
/// old results are discarded and the overview is shown. Other entries' results stay.
pub(super) fn rerun(state: AppState, entry: EntryId) -> Out {
    let c = &state.compare;
    if !c.can_rerun(entry) {
        return none(state);
    }
    let old: Vec<PairId> = c
        .overview
        .row_for(entry)
        .filter(|r| !is_idle(r.status.clone()))
        .map(|r| r.pair)
        .into_iter()
        .collect();
    let overview = old
        .iter()
        .fold(c.overview.clone(), |o, pair| o.discarded(*pair));
    let compare = CompareState {
        overview,
        pairs: c.pairs.clone().only_included(entry).editing(Some(entry)),
        open_pair: None,
        detail: None,
        ..c.clone()
    }
    .without_results(&old)
    .invalidated();
    let commands = discards(ready(&state), &old);
    (with_compare(state, compare), commands)
}

pub(super) fn rows_arrived(
    state: AppState,
    pair: PairId,
    (category, offset, total): (Category, usize, usize),
    rows: Vec<RowView>,
) -> Out {
    let compare = state
        .compare
        .clone()
        .results_updated(pair, |r| r.accept_rows(category, offset, total, rows));
    none(with_compare(state, compare))
}

/// A detail is taken only for the open pair, while it is the loading request for that key.
pub(super) fn detail_arrived(
    state: AppState,
    pair: PairId,
    key: KeyTuple,
    (source, target): (Option<bson::Document>, Option<bson::Document>),
    changes: &[curlybrace_core::compare::diff::FieldChange],
) -> Out {
    let Some(detail) = state.compare.detail.clone() else {
        return none(state);
    };
    if state.compare.open_pair != Some(pair) || !detail.loading || !detail.is_for(&key) {
        return none(state);
    }
    let compare = CompareState {
        detail: Some(detail.loaded(source, target, changes)),
        ..state.compare.clone()
    };
    none(with_compare(state, compare))
}
