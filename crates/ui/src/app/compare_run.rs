//! Run and result handling of the Compare reducer: starting, cancelling and discarding a run,
//! paging through its results, and the runtime events that report progress, results and details.
use super::compare::{cmd, failure_banner, none, ready, with_compare};
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::compare::{CompareState, DetailView, PAIR_ID, ResultsView, RunView};
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::{
    CompareCommand, CompareEvent, PairOutcome, RowView, SessionId,
};
use curlybrace_core::compare::results::Category;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Command;

pub(super) fn start(state: AppState) -> Out {
    let Some(session) = ready(&state) else {
        return none(state);
    };
    let c = &state.compare;
    if c.run.running().is_some() || c.check_in_flight || !c.preflight.allows_start() {
        return none(state);
    }
    let Ok(pair) = c.pair() else {
        return none(state);
    };
    let run = c.next_run;
    let mut commands = Vec::new();
    if c.run != RunView::Idle {
        commands.push(cmd(CompareCommand::Discard {
            session,
            pair: PAIR_ID,
        }));
    }
    commands.push(cmd(CompareCommand::Start {
        session,
        run,
        pairs: vec![pair],
    }));
    let compare = CompareState {
        run: RunView::started(run),
        next_run: run + 1,
        cancel_requested: false,
        results: None,
        detail: None,
        duplicates: c.duplicates.superseded(),
        ..c.clone()
    };
    (with_compare(state, compare), commands)
}

/// Sends Cancel once per run and marks it at once, so the view can say "cancelling" before
/// the runtime answers.
pub(super) fn cancel(state: AppState) -> Out {
    let (Some(session), Some(run)) = (ready(&state), state.compare.run.running()) else {
        return none(state);
    };
    if state.compare.cancel_requested {
        return none(state);
    }
    let compare = CompareState {
        cancel_requested: true,
        ..state.compare.clone()
    };
    (
        with_compare(state, compare),
        vec![cmd(CompareCommand::Cancel { session, run })],
    )
}

/// Category switch or paging: `change` yields the next results, whose page is then requested.
pub(super) fn moved(
    state: AppState,
    change: impl FnOnce(&ResultsView) -> Option<ResultsView>,
) -> Out {
    let (Some(session), Some(results)) = (ready(&state), state.compare.results.as_ref()) else {
        return none(state);
    };
    let Some(next) = change(results) else {
        return none(state);
    };
    let commands = next.request(session).into_iter().map(cmd).collect();
    let compare = CompareState {
        results: Some(next),
        ..state.compare.clone()
    };
    (with_compare(state, compare), commands)
}

pub(super) fn select(state: AppState, key: KeyTuple) -> Out {
    let (Some(session), Some(results)) = (ready(&state), state.compare.results.as_ref()) else {
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

pub(super) fn discard(state: AppState) -> Out {
    let Some(session) = ready(&state) else {
        return none(state);
    };
    if state.compare.run.running().is_some() {
        return none(state);
    }
    let compare = CompareState {
        run: RunView::Idle,
        results: None,
        detail: None,
        duplicates: state.compare.duplicates.superseded(),
        ..state.compare.clone()
    };
    (
        with_compare(state, compare),
        vec![cmd(CompareCommand::Discard {
            session,
            pair: PAIR_ID,
        })],
    )
}

/// Progress, results and details of a run.
pub(super) fn on_run_event(state: AppState, ev: CompareEvent) -> Out {
    match ev {
        CompareEvent::Progress {
            run,
            pair,
            scanned_source,
            scanned_target,
            differences,
            ..
        } => progress(
            state,
            run,
            pair,
            (scanned_source, scanned_target, differences),
        ),
        CompareEvent::PairDone {
            session,
            run,
            pair,
            outcome,
        } => pair_done(state, session, run, pair, outcome),
        CompareEvent::RunDone { run, .. } => run_done(state, run),
        CompareEvent::Rows {
            pair,
            category,
            offset,
            total,
            rows,
            ..
        } => rows_arrived(state, pair, (category, offset, total), rows),
        CompareEvent::Detail {
            pair,
            key,
            source,
            target,
            changes,
            ..
        } => detail_arrived(state, pair, key, (source, target), &changes),
        _ => none(state),
    }
}

fn progress(state: AppState, run: u64, pair: u32, counters: (u64, u64, u64)) -> Out {
    let RunView::Running {
        run: current,
        finished_pairs,
        cancelled,
        failure,
        ..
    } = state.compare.run.clone()
    else {
        return none(state);
    };
    if run != current || pair != PAIR_ID {
        return none(state);
    }
    let compare = CompareState {
        run: RunView::Running {
            run,
            finished_pairs,
            scanned_source: counters.0,
            scanned_target: counters.1,
            differences: counters.2,
            cancelled,
            failure,
        },
        ..state.compare.clone()
    };
    none(with_compare(state, compare))
}

/// What a pair outcome adds to the run: its results and the updated cancelled/failure markers.
fn settle(
    outcome: &PairOutcome,
    cancelled: bool,
    failure: Option<CoreError>,
) -> (Option<ResultsView>, bool, Option<CoreError>) {
    match outcome {
        PairOutcome::Completed(s) => (Some(ResultsView::new(s)), cancelled, failure),
        // The runtime keeps nothing for a cancelled pair: no results view, no Rows request.
        PairOutcome::Cancelled(_) => (None, true, failure),
        PairOutcome::Failed(e) => (None, cancelled, Some(e.clone())),
    }
}

fn first_page(results: Option<&ResultsView>, session: SessionId) -> Vec<Command> {
    results
        .and_then(|r| r.request(session))
        .into_iter()
        .map(cmd)
        .collect()
}

fn pair_done(
    state: AppState,
    session: SessionId,
    run: u64,
    pair: u32,
    outcome: PairOutcome,
) -> Out {
    let RunView::Running {
        run: current,
        finished_pairs,
        scanned_source,
        scanned_target,
        differences,
        cancelled,
        failure,
    } = state.compare.run.clone()
    else {
        return none(state);
    };
    if run != current || pair != PAIR_ID {
        return none(state);
    }
    let (results, cancelled, failure) = settle(&outcome, cancelled, failure);
    let commands = first_page(results.as_ref(), session);
    // The pair's own summary has the final counts; a failed pair keeps the last progress.
    let (scanned_source, scanned_target) = match &outcome {
        PairOutcome::Completed(s) | PairOutcome::Cancelled(s) => {
            (s.scanned_source, s.scanned_target)
        }
        PairOutcome::Failed(_) => (scanned_source, scanned_target),
    };
    let compare = CompareState {
        run: RunView::Running {
            run,
            finished_pairs: finished_pairs + 1,
            scanned_source,
            scanned_target,
            differences,
            cancelled,
            failure,
        },
        cancel_requested: false,
        results,
        detail: None,
        ..state.compare.clone()
    };
    let next = with_compare(state, compare);
    match outcome {
        PairOutcome::Failed(error) => (failure_banner(next, "compare_start", error), commands),
        _ => (next, commands),
    }
}

fn run_done(state: AppState, run: u64) -> Out {
    let RunView::Running {
        run: current,
        cancelled,
        failure,
        scanned_source,
        scanned_target,
        ..
    } = state.compare.run.clone()
    else {
        return none(state);
    };
    if run != current {
        return none(state);
    }
    let compare = CompareState {
        run: RunView::Finished {
            run,
            cancelled,
            failure,
            scanned_source,
            scanned_target,
        },
        cancel_requested: false,
        ..state.compare.clone()
    };
    none(with_compare(state, compare))
}

fn rows_arrived(
    state: AppState,
    pair: u32,
    (category, offset, total): (Category, usize, usize),
    rows: Vec<RowView>,
) -> Out {
    let Some(results) = state.compare.results.clone() else {
        return none(state);
    };
    if pair != PAIR_ID {
        return none(state);
    }
    let compare = CompareState {
        results: Some(results.accept_rows(category, offset, total, rows)),
        ..state.compare.clone()
    };
    none(with_compare(state, compare))
}

fn detail_arrived(
    state: AppState,
    pair: u32,
    key: KeyTuple,
    (source, target): (Option<bson::Document>, Option<bson::Document>),
    changes: &[curlybrace_core::compare::diff::FieldChange],
) -> Out {
    let Some(detail) = state.compare.detail.clone() else {
        return none(state);
    };
    if pair != PAIR_ID || !detail.loading || !detail.is_for(&key) {
        return none(state);
    }
    let compare = CompareState {
        detail: Some(detail.loaded(source, target, changes)),
        ..state.compare.clone()
    };
    none(with_compare(state, compare))
}
