//! The query bar's index-miss guard: before a Run sends its find, the server is asked to plan the
//! query, and a plan that would scan the collection (or sort in memory) is shown as a notice
//! instead of running. The guard is advice that fails open: whenever the check cannot answer, the
//! run goes on as if there were no guard.
//!
//! Guard checks are numbered from `AppState::guard_request`, which the Shell tab shares: an answer
//! is offered to the query bar first ([`awaits`]) and, when it is not the query bar's, to the
//! Shell tab (`shell_guard`).

use super::explain_actions::error_variant;
use super::reducer::Out;
use super::state::AppState;
use curlybrace_core::error::CoreError;
use curlybrace_core::guard::{GUARD_MIN_DOCS, GuardReport, GuardTarget};
use curlybrace_core::runtime::Command;
use curlybrace_core::store::Query;

/// Where the query bar's guard stands for the run the person asked for.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum GuardState {
    #[default]
    None,
    /// The plan of `query` was asked for (as guard request `request`); its find is not sent yet.
    Checking { request: u64, query: Query },
    /// The plan of `query` would miss an index: the find waits for Run anyway or Cancel.
    Warn { query: Query, report: GuardReport },
}

impl GuardState {
    /// True while the plan is being asked for (the find is held back).
    pub fn is_checking(&self) -> bool {
        matches!(self, Self::Checking { .. })
    }

    /// The query held back and what its check found, while the notice is shown.
    pub fn warning(&self) -> Option<(&Query, &GuardReport)> {
        match self {
            Self::Warn { query, report } => Some((query, report)),
            Self::None | Self::Checking { .. } => None,
        }
    }
}

/// Whether a query is worth checking: with neither a filter nor a sort the server reads the
/// collection in natural order whatever indexes exist, so there is nothing to warn about.
pub(super) fn worth_checking(query: &Query) -> bool {
    !(query.filter.is_empty() && query.sort.is_none())
}

/// `FindAction::Run`: with the guard on, a collection open and a query worth checking, asks for
/// the plan and holds the find back; otherwise runs as without a guard. Either way the previous
/// run's guard is gone.
pub(super) fn run(state: AppState, query: Query) -> Out {
    let state = dropped(state);
    let ns = state.selected_ns().cloned();
    let Some(ns) = ns.filter(|_| state.guard_enabled() && worth_checking(&query)) else {
        let explain = state.explain_enabled();
        return super::explain_actions::run(state, query, explain);
    };
    let request = state.guard_request + 1;
    let session = state.session_id();
    // A production connection warns whatever the collection's size.
    let min_docs = (!state.session_production(session)).then_some(GUARD_MIN_DOCS);
    let command = Command::GuardCheck {
        session,
        request,
        targets: vec![GuardTarget {
            ns,
            query: query.clone(),
        }],
        min_docs,
    };
    let next = AppState {
        guard: Box::new(GuardState::Checking { request, query }),
        guard_request: request,
        ..state
    };
    (next, vec![command])
}

/// Whether guard request `request` is the query bar's check still waiting for its answer.
pub(super) fn awaits(state: &AppState, request: u64) -> bool {
    matches!(&*state.guard, GuardState::Checking { request: r, .. } if *r == request)
}

/// `Event::GuardChecked`, offered to the query bar's check first. An answer the query bar does
/// not await goes to the Shell tab, which drops it in turn when no shell waits for it (a check
/// that is gone).
pub(super) fn answered(
    state: AppState,
    request: u64,
    outcome: Result<GuardReport, CoreError>,
) -> Out {
    if awaits(&state, request) {
        return checked(state, outcome);
    }
    super::shell_guard::answered(state, request, outcome)
}

/// `Event::RequestFailed` of a guard check that could not start, routed like [`answered`]. The
/// query bar's or the shell's run goes on, without a banner, exactly as after a failed check.
pub(super) fn not_started(state: AppState, request: u64, error: CoreError) -> Out {
    if awaits(&state, request) {
        return checked(state, Err(error));
    }
    super::shell_guard::answered(state, request, Err(error))
}

/// The answer to the query bar's check (the caller made sure it [`awaits`] it). Findings hold the
/// run back as a notice while the guard is on; no findings, a check that failed, or a guard
/// turned off since (a background session's view misses the switch) let it go on.
fn checked(state: AppState, outcome: Result<GuardReport, CoreError>) -> Out {
    let GuardState::Checking { query, .. } = (*state.guard).clone() else {
        return (state, Vec::new());
    };
    match outcome {
        Ok(report) if !report.findings.is_empty() && state.guard_enabled() => {
            let guard = Box::new(GuardState::Warn { query, report });
            (AppState { guard, ..state }, Vec::new())
        }
        Ok(_) => go_on(state, query),
        Err(error) => {
            log::warn!("index guard check unavailable: {}", error_variant(&error));
            go_on(state, query)
        }
    }
}

/// Sends the run's find and, as a Run would, the badge's explain.
fn go_on(state: AppState, query: Query) -> Out {
    let state = dropped(state);
    let explain = state.explain_enabled();
    super::explain_actions::run(state, query, explain)
}

/// The guard switch was turned off: the run shown behaves as without a guard. A check in
/// flight lets its run go on (as a failed check does; its late answer is then ignored), and a
/// notice just closes (the person can Run again).
pub(super) fn switched_off(state: AppState) -> Out {
    match (*state.guard).clone() {
        GuardState::Checking { query, .. } => go_on(state, query),
        GuardState::Warn { .. } => (dropped(state), Vec::new()),
        GuardState::None => (state, Vec::new()),
    }
}

/// `FindAction::GuardRunAnyway`: sends the held-back find once. It is neither checked again nor
/// explained: the badge's explain executes the query, which is what the notice warned about.
pub(super) fn run_anyway(state: AppState) -> Out {
    let GuardState::Warn { query, .. } = (*state.guard).clone() else {
        return (state, Vec::new());
    };
    super::explain_actions::run(dropped(state), query, false)
}

/// Forgets the guard of the run shown: its notice, or a check still in flight (whose answer is
/// then dropped). Called when the run it belongs to is gone, and for `FindAction::GuardCancel`.
pub(super) fn dropped(state: AppState) -> AppState {
    AppState {
        guard: Box::default(),
        ..state
    }
}
