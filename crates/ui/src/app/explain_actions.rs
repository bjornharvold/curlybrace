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

/// The verdict to draw beside the page on screen. A finished verdict shows only over the page of
/// its own run: while the run's find is still in flight (the explain answered first) it reads
/// as "checking", and when that find failed and the previous page stays it shows nothing, so a
/// badge never describes documents it was not computed for.
pub fn shown(state: &AppState) -> VerdictState {
    let Some(run) = state.verdict.run() else {
        return VerdictState::None;
    };
    match &state.verdict {
        VerdictState::Pending { .. } => state.verdict.clone(),
        _ if same_run(run, &state.page_query) => state.verdict.clone(),
        _ if state.loading_page && same_run(run, &state.issued_query) => {
            VerdictState::Pending { query: run.clone() }
        }
        _ => VerdictState::None,
    }
}

/// The name of an error's variant, e.g. `Database`. The message is never part of it: it can
/// quote the literal values of the query, so only the variant is safe to log.
pub(super) fn error_variant(error: &CoreError) -> String {
    format!("{error:?}")
        .chars()
        .take_while(|c| c.is_alphanumeric())
        .collect()
}

/// An explain command that failed as a command: the run still waiting turns unavailable, any
/// other state is left alone. The explain is advice, so this never raises a banner.
pub(super) fn command_failed(state: AppState, error: CoreError) -> AppState {
    let VerdictState::Pending { query } = &state.verdict else {
        return state;
    };
    log::warn!("explain failed: {}", error_variant(&error));
    let verdict = VerdictState::Unavailable {
        query: query.clone(),
        error,
    };
    AppState { verdict, ..state }
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
        Err(error) => {
            log::warn!("explain unavailable: {}", error_variant(&error));
            VerdictState::Unavailable { query, error }
        }
    };
    AppState { verdict, ..state }
}

/// `FindAction::Run`: fetches the page and, when `explain_enabled`, asks for the run's verdict
/// too. A run that issues the find without an explain drops the previous run's verdict: it no
/// longer describes what is shown.
pub(super) fn run(state: AppState, query: Query, explain_enabled: bool) -> super::reducer::Out {
    let (state, mut commands) = super::actions::issue_find(state, query.clone());
    let Some(ns) = state
        .selected_ns()
        .cloned()
        .filter(|_| !commands.is_empty())
    else {
        return (state, commands);
    };
    if !explain_enabled {
        return (
            AppState {
                verdict: VerdictState::None,
                ..state
            },
            commands,
        );
    }
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
