//! Find duplicate keys: one request at a time, asked after a run failed on a duplicate key.
//! Keys are data: they are shown by the panel, never logged.
use super::compare::{cmd, failure_banner, none, ready, with_compare};
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::compare::{CompareState, DuplicatesView, PAIR_ID};
use curlybrace_core::compare::protocol::{CompareCommand, DuplicateReport};
use curlybrace_core::error::CoreError;

fn with_view(state: AppState, duplicates: DuplicatesView) -> AppState {
    let compare = CompareState {
        duplicates,
        ..state.compare.clone()
    };
    with_compare(state, compare)
}

/// Sends the request for the current form's pair, exactly as a check would build it.
pub(super) fn find(state: AppState) -> Out {
    let Some(session) = ready(&state) else {
        return none(state);
    };
    if !state.compare.can_find_duplicates() {
        return none(state);
    }
    let Ok(pair) = state.compare.pair() else {
        return none(state);
    };
    let loading = DuplicatesView::Loading { stale: false };
    (
        with_view(state, loading),
        vec![cmd(CompareCommand::DuplicateKeys { session, pair })],
    )
}

/// The answer to the request in flight; one for another pair, with nothing requested, is
/// ignored, and one the form was edited under is dropped (the user asks again).
pub(super) fn answered(
    state: AppState,
    pair: u32,
    source: DuplicateReport,
    target: DuplicateReport,
) -> Out {
    let DuplicatesView::Loading { stale } = state.compare.duplicates else {
        return none(state);
    };
    if pair != PAIR_ID {
        return none(state);
    }
    let view = if stale {
        DuplicatesView::Idle
    } else {
        DuplicatesView::Loaded { source, target }
    };
    none(with_view(state, view))
}

/// The request failed: the banner reports it, unless it was stale (then the view just resets).
pub(super) fn failed(state: AppState, error: CoreError) -> AppState {
    let DuplicatesView::Loading { stale } = state.compare.duplicates else {
        return failure_banner(state, "compare_duplicates", error);
    };
    if stale {
        return with_view(state, DuplicatesView::Idle);
    }
    failure_banner(
        with_view(state, DuplicatesView::Failed),
        "compare_duplicates",
        error,
    )
}
