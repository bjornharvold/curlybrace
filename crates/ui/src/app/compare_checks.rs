//! The preflight queue of the Compare reducer: `RunCheck` queues the included entries and the
//! reducer sends one `Check` at a time, each under a fresh probe id, so an answer names its
//! entry and a stale answer is recognisable.
use super::compare::{cmd, failure_banner, none, ready, with_compare};
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::compare::{CheckQueue, CompareState};
use curlybrace_core::compare::protocol::{CompareCommand, PairId, Preflight, SessionId};
use curlybrace_core::error::CoreError;

/// Queues every included entry whose spec builds (the others show their issue) and sends the
/// first check.
pub(super) fn run_check(state: AppState) -> Out {
    let Some(session) = ready(&state) else {
        return none(state);
    };
    let c = &state.compare;
    if c.is_running() || c.check_in_flight() || c.apply.is_busy() {
        return none(state);
    }
    let ids = c.checkable_entries();
    if ids.is_empty() {
        return none(state);
    }
    let compare = CompareState {
        checks: CheckQueue::queued(&ids),
        ..c.clone()
    };
    send_next(with_compare(state, compare), session)
}

/// Sends the check of the next queued entry under a fresh probe id; nothing while a check is
/// out. An entry whose spec no longer builds ends as failed and the queue moves on.
fn send_next(state: AppState, session: SessionId) -> Out {
    let c = &state.compare;
    let Some(entry) = c.checks.next_to_send() else {
        return none(state);
    };
    let probe = c.next_pair;
    let Some(Ok(pair)) = c.spec_for(entry, probe) else {
        let compare = CompareState {
            checks: c.checks.clone().skipped(entry),
            ..c.clone()
        };
        return send_next(with_compare(state, compare), session);
    };
    let compare = CompareState {
        checks: c.checks.clone().sending(entry, probe),
        next_pair: probe + 1,
        ..c.clone()
    };
    (
        with_compare(state, compare),
        vec![cmd(CompareCommand::Check { session, pair })],
    )
}

/// An answer is taken only under the probe that is in flight; an orphan's (an edit made it
/// stale) is dropped and does not resume the queue, the user checks again.
pub(super) fn checked(state: AppState, probe: PairId, answer: Preflight) -> Out {
    if state.compare.orphaned_check == Some(probe) {
        return none(settled_orphan(state));
    }
    let (checks, fresh) = state.compare.checks.clone().answered(probe, answer);
    if !fresh {
        return none(state);
    }
    let compare = CompareState {
        checks,
        ..state.compare.clone()
    };
    resume(with_compare(state, compare))
}

fn resume(state: AppState) -> Out {
    match ready(&state) {
        Some(session) => send_next(state, session),
        None => none(state),
    }
}

fn settled_orphan(state: AppState) -> AppState {
    let compare = CompareState {
        orphaned_check: None,
        ..state.compare.clone()
    };
    with_compare(state, compare)
}

/// A failed check names no probe: it is the orphan's (quiet) or the one in flight, which ends
/// as failed, raises the banner and lets the queue go on.
pub(super) fn check_failed(state: AppState, error: CoreError) -> Out {
    let c = &state.compare;
    let Some((_, probe)) = c.checks.in_flight() else {
        // The orphan's failure is obsolete and quiet; with no orphan it is reported.
        return match c.orphaned_check {
            Some(_) => none(settled_orphan(state)),
            None => none(failure_banner(state, "compare_check", error)),
        };
    };
    let compare = CompareState {
        checks: c.checks.clone().failed(probe),
        ..c.clone()
    };
    let shown = failure_banner(with_compare(state, compare), "compare_check", error);
    resume(shown)
}
