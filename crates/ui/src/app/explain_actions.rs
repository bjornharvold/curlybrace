//! The verdict of the query bar's current run (release A): which run it belongs to and when an
//! `Explain` event may replace it.

use super::state::AppState;
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::Verdict;
use curlybrace_core::runtime::Command;
use curlybrace_core::store::{Namespace, Query};

/// What the query bar knows about how the current run is served.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum VerdictState {
    #[default]
    None,
    /// An explain was issued for this run and has not answered yet.
    Pending {
        query: Query,
    },
    Ready {
        query: Query,
        verdict: Verdict,
    },
    /// The explain failed or timed out: no verdict, and nothing to alarm the user about.
    Unavailable {
        query: Query,
        error: CoreError,
    },
}

impl VerdictState {
    /// The run this state belongs to.
    fn run(&self) -> Option<&Query> {
        match self {
            Self::None => None,
            Self::Pending { query }
            | Self::Ready { query, .. }
            | Self::Unavailable { query, .. } => Some(query),
        }
    }
}

/// True when `a` and `b` are the same run: paging (`skip`) does not make a new one.
pub(super) fn same_run(a: &Query, b: &Query) -> bool {
    a.filter == b.filter && a.sort == b.sort && a.projection == b.projection && a.limit == b.limit
}

/// Whether the Run action should also ask for a verdict. A constant until the setting exists.
pub(super) fn enabled() -> bool {
    true
}

/// Stores the outcome of an explain when it belongs to the run still shown; any other (another
/// collection, a superseded run) is dropped and the state returned unchanged.
pub(super) fn arrived(
    state: AppState,
    ns: Namespace,
    query: Query,
    outcome: Result<Verdict, CoreError>,
) -> AppState {
    let current = state.selected_ns() == Some(&ns)
        && state.verdict.run().is_some_and(|run| same_run(run, &query));
    if !current {
        return state;
    }
    let verdict = match outcome {
        Ok(verdict) => VerdictState::Ready { query, verdict },
        Err(error) => VerdictState::Unavailable { query, error },
    };
    AppState { verdict, ..state }
}

/// `FindAction::Run`: fetches the page and, when enabled, asks for the run's verdict too.
pub(super) fn run(state: AppState, query: Query) -> super::reducer::Out {
    let (state, mut commands) = super::actions::issue_find(state, query.clone());
    let Some(ns) = state
        .selected_ns()
        .cloned()
        .filter(|_| state.explain_enabled())
    else {
        return (state, commands);
    };
    commands.push(Command::ExplainFind {
        session: state.session_id(),
        ns,
        query: query.clone(),
    });
    (
        AppState {
            verdict: VerdictState::Pending { query },
            ..state
        },
        commands,
    )
}
