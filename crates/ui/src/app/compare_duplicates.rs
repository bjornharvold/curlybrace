//! Find duplicate keys: one request at a time, asked for a pair that failed on a duplicate key.
//! Keys are data: they are shown by the panel, never logged.
use super::compare::{cmd, failure_banner, none, ready, with_compare};
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::compare::{CompareState, DuplicatesView, EntryId};
use curlybrace_core::compare::protocol::{CompareCommand, DuplicateReport, PairId};
use curlybrace_core::error::CoreError;

fn with_view(state: AppState, duplicates: DuplicatesView) -> AppState {
    let compare = CompareState {
        duplicates,
        ..state.compare.clone()
    };
    with_compare(state, compare)
}

/// Sends the request for the entry's spec under a fresh pair id, which also names its answer.
/// The failed run's id is never reused: a late page or answer of that run stays recognisable.
pub(super) fn find(state: AppState, entry: EntryId) -> Out {
    let Some(session) = ready(&state) else {
        return none(state);
    };
    let c = &state.compare;
    if !c.can_find_duplicates_for(entry) {
        return none(state);
    }
    let id = c.next_pair;
    let Some(Ok(pair)) = c.spec_for(entry, id) else {
        return none(state);
    };
    let loading = DuplicatesView::Loading {
        entry,
        pair: id,
        stale: false,
    };
    let compare = CompareState {
        duplicates: loading,
        next_pair: id + 1,
        ..c.clone()
    };
    (
        with_compare(state, compare),
        vec![cmd(CompareCommand::DuplicateKeys { session, pair })],
    )
}

/// The answer to the request in flight. One for another pair is ignored (the lookup of another
/// entry, or one that never was); the answer of a lookup the setup was edited under is dropped
/// and the user asks again.
pub(super) fn answered(
    state: AppState,
    pair: PairId,
    source: DuplicateReport,
    target: DuplicateReport,
) -> Out {
    let DuplicatesView::Loading {
        entry,
        pair: asked,
        stale,
    } = state.compare.duplicates
    else {
        return none(state);
    };
    if asked != pair {
        return none(state);
    }
    let view = if stale {
        DuplicatesView::Idle
    } else {
        DuplicatesView::Loaded {
            entry,
            source,
            target,
        }
    };
    none(with_view(state, view))
}

/// The request failed: the banner reports it, unless it was stale (then the view just resets).
pub(super) fn failed(state: AppState, error: CoreError) -> AppState {
    let context = failure_context(&error);
    let DuplicatesView::Loading { entry, stale, .. } = state.compare.duplicates else {
        return failure_banner(state, context, error);
    };
    if stale {
        return with_view(state, DuplicatesView::Idle);
    }
    failure_banner(
        with_view(state, DuplicatesView::Failed { entry }),
        context,
        error,
    )
}

/// The runtime's bound on the lookup is reported as `Database("timeout")`; that gets advice of
/// its own (narrow the collections) instead of the generic "try again".
fn failure_context(error: &CoreError) -> &'static str {
    match error {
        CoreError::Database(detail) if detail == "timeout" => "compare_duplicates_timeout",
        _ => "compare_duplicates",
    }
}
