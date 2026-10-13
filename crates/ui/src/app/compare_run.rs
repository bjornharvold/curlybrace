//! Runs of the Compare reducer: starting the included entries as one run, cancelling it, and the
//! runtime events that report progress and the end of each pair. Every started pair takes a
//! fresh `PairId`, so a late event for an earlier run's pair never matches the new one.
use super::compare::{cmd, failure_banner, none, ready, with_compare};
use super::compare_pairs::discards;
use super::compare_results::{detail_arrived, rows_arrived};
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::compare::{CompareState, EntryId, ResultsView, RunOverview};
use curlybrace_core::compare::protocol::{
    CompareCommand, CompareEvent, PairId, PairOutcome, PairSpec, SessionId,
};
use curlybrace_core::runtime::Command;

/// Run: the included entries (the gate has made sure every one passed its check).
pub(super) fn start(state: AppState) -> Out {
    begin(state, false)
}

/// Run passing: only the included entries whose check passed.
pub(super) fn start_passing(state: AppState) -> Out {
    begin(state, true)
}

/// The entries to run with a fresh pair id each, in list order.
fn plan(c: &CompareState, passing_only: bool) -> Vec<(EntryId, PairSpec)> {
    let mut next = c.next_pair;
    let mut planned = Vec::new();
    for entry in c.pairs.entries.iter().filter(|e| e.included) {
        if passing_only && !c.checks.allows_start(entry.id) {
            continue;
        }
        if let Some(Ok(spec)) = c.spec_for(entry.id, next) {
            planned.push((entry.id, spec));
            next += 1;
        }
    }
    planned
}

fn begin(state: AppState, passing_only: bool) -> Out {
    let Some(session) = ready(&state) else {
        return none(state);
    };
    let c = &state.compare;
    let allowed = if passing_only {
        c.can_start_passing()
    } else {
        c.can_start()
    };
    let planned = plan(c, passing_only);
    if !allowed || planned.is_empty() {
        return none(state);
    }
    let run = c.next_run;
    let ids: Vec<(EntryId, PairId)> = planned.iter().map(|(e, s)| (*e, s.id)).collect();
    let (overview, superseded) = c.overview.clone().started(run, &ids);
    let mut commands = discards(Some(session), &superseded);
    commands.push(cmd(CompareCommand::Start {
        session,
        run,
        pairs: planned.into_iter().map(|(_, spec)| spec).collect(),
    }));
    let compare = CompareState {
        overview,
        next_run: run + 1,
        next_pair: c.next_pair + ids.len() as PairId,
        duplicates: c.duplicates.superseded(),
        ..c.clone()
    }
    .without_results(&superseded);
    (with_compare(state, compare), commands)
}

/// Sends Cancel once per run and marks it at once, so the view can say "cancelling" before
/// the runtime answers. Rows still pending end by the runtime's own events.
pub(super) fn cancel(state: AppState) -> Out {
    let (Some(session), Some(active)) = (ready(&state), state.compare.overview.active) else {
        return none(state);
    };
    if !state.compare.can_cancel() {
        return none(state);
    }
    let compare = CompareState {
        overview: state.compare.overview.clone().cancelling(),
        ..state.compare.clone()
    };
    (
        with_compare(state, compare),
        vec![cmd(CompareCommand::Cancel {
            session,
            run: active.run,
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
    let overview = state
        .compare
        .overview
        .clone()
        .progressed(run, pair, counters);
    none(with_overview(state, overview))
}

fn with_overview(state: AppState, overview: RunOverview) -> AppState {
    let compare = CompareState {
        overview,
        ..state.compare.clone()
    };
    with_compare(state, compare)
}

fn first_page(results: Option<&ResultsView>, session: SessionId) -> Vec<Command> {
    results
        .and_then(|r| r.request(session))
        .into_iter()
        .map(cmd)
        .collect()
}

/// A pair ended. Only the active run's live row is settled; a completed pair gets its results
/// (and their first page), a cancelled or failed one none. A run of one pair opens its results
/// at once when nothing is open; the open detail stays unless the open pair changes. The
/// banner is raised for the first failed pair of a run only.
fn pair_done(
    state: AppState,
    session: SessionId,
    run: u64,
    pair: PairId,
    outcome: PairOutcome,
) -> Out {
    let c = &state.compare;
    let first_failure =
        matches!(outcome, PairOutcome::Failed(_)) && c.overview.failures_in(run) == 0;
    let overview = c.overview.clone().pair_done(run, pair, outcome.clone());
    if overview == c.overview {
        return none(state);
    }
    let shown = match &outcome {
        PairOutcome::Completed(summary) => Some(ResultsView::new(pair, summary)),
        // Only a completed pair holds results in the runtime: no results and no Rows request
        // for a cancelled or failed one (a later Discard for it is harmless).
        _ => None,
    };
    let commands = first_page(shown.as_ref(), session);
    let opens = shown.is_some() && c.open_pair.is_none() && overview.rows_in(run) == 1;
    let compare = CompareState {
        overview,
        results: c
            .results
            .iter()
            .map(|(id, view)| (*id, view.clone()))
            .chain(shown.map(|view| (pair, view)))
            .collect(),
        open_pair: if opens { Some(pair) } else { c.open_pair },
        detail: if opens { None } else { c.detail.clone() },
        ..c.clone()
    };
    let next = with_compare(state, compare);
    match outcome {
        PairOutcome::Failed(error) if first_failure => {
            (failure_banner(next, "compare_start", error), commands)
        }
        _ => (next, commands),
    }
}

fn run_done(state: AppState, run: u64) -> Out {
    let overview = state.compare.overview.clone().run_done(run);
    none(with_overview(state, overview))
}
