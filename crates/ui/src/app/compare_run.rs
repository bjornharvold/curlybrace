//! Run and result handling of the Compare reducer: starting, cancelling and discarding a run,
//! paging through its results, and the runtime events that report progress, results and details.
use super::compare::{cmd, failure_banner, none, ready, with_compare};
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::compare::{CompareState, DetailView, ResultsView, RunOverview, SINGLE_ENTRY};
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::{
    CompareCommand, CompareEvent, PairOutcome, RowView, SessionId,
};
use curlybrace_core::compare::results::Category;
use curlybrace_core::runtime::Command;

pub(super) fn start(state: AppState) -> Out {
    let Some(session) = ready(&state) else {
        return none(state);
    };
    let c = &state.compare;
    if !c.can_start() {
        return none(state);
    }
    let run = c.next_run;
    let pair_id = c.next_pair;
    let Ok(pair) = c.pair_for(pair_id) else {
        return none(state);
    };
    let (overview, superseded) = c.overview.clone().started(run, &[(SINGLE_ENTRY, pair_id)]);
    let mut commands: Vec<Command> = superseded
        .iter()
        .map(|pair| {
            cmd(CompareCommand::Discard {
                session,
                pair: *pair,
            })
        })
        .collect();
    commands.push(cmd(CompareCommand::Start {
        session,
        run,
        pairs: vec![pair],
    }));
    let compare = CompareState {
        overview,
        next_run: run + 1,
        next_pair: pair_id + 1,
        results: c
            .results
            .iter()
            .filter(|(pair, _)| !superseded.contains(pair))
            .map(|(pair, view)| (*pair, view.clone()))
            .collect(),
        open_pair: None,
        detail: None,
        duplicates: c.duplicates.superseded(),
        ..c.clone()
    };
    (with_compare(state, compare), commands)
}

/// Sends Cancel once per run and marks it at once, so the view can say "cancelling" before
/// the runtime answers.
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
    let compare = state.compare.clone().results_updated(next.pair, |_| next);
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

pub(super) fn discard(state: AppState) -> Out {
    let (Some(session), Some(pair)) = (ready(&state), state.compare.single_pair()) else {
        return none(state);
    };
    if !state.compare.can_discard() {
        return none(state);
    }
    let c = &state.compare;
    let compare = CompareState {
        overview: c.overview.clone().discarded(pair),
        results: c
            .results
            .iter()
            .filter(|(id, _)| **id != pair)
            .map(|(id, view)| (*id, view.clone()))
            .collect(),
        open_pair: c.open_pair.filter(|open| *open != pair),
        detail: None,
        duplicates: c.duplicates.superseded(),
        ..c.clone()
    };
    (
        with_compare(state, compare),
        vec![cmd(CompareCommand::Discard { session, pair })],
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

fn pair_done(
    state: AppState,
    session: SessionId,
    run: u64,
    pair: u32,
    outcome: PairOutcome,
) -> Out {
    let c = &state.compare;
    let overview = c.overview.clone().pair_done(run, pair, outcome.clone());
    if overview == c.overview {
        return none(state);
    }
    let shown = match &outcome {
        PairOutcome::Completed(summary) => Some(ResultsView::new(pair, summary)),
        // The runtime keeps nothing for a cancelled or failed pair: no results, no Rows request.
        _ => None,
    };
    let commands = first_page(shown.as_ref(), session);
    let compare = CompareState {
        overview,
        results: c
            .results
            .iter()
            .map(|(id, view)| (*id, view.clone()))
            .chain(shown.map(|view| (pair, view)))
            .collect(),
        open_pair: match outcome {
            PairOutcome::Completed(_) => Some(pair),
            _ => c.open_pair,
        },
        detail: None,
        ..c.clone()
    };
    let next = with_compare(state, compare);
    match outcome {
        PairOutcome::Failed(error) => (failure_banner(next, "compare_start", error), commands),
        _ => (next, commands),
    }
}

fn run_done(state: AppState, run: u64) -> Out {
    let overview = state.compare.overview.clone().run_done(run);
    none(with_overview(state, overview))
}

fn rows_arrived(
    state: AppState,
    pair: u32,
    (category, offset, total): (Category, usize, usize),
    rows: Vec<RowView>,
) -> Out {
    let compare = state
        .compare
        .clone()
        .results_updated(pair, |r| r.accept_rows(category, offset, total, rows));
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
    if state.compare.open_pair != Some(pair) || !detail.loading || !detail.is_for(&key) {
        return none(state);
    }
    let compare = CompareState {
        detail: Some(detail.loaded(source, target, changes)),
        ..state.compare.clone()
    };
    none(with_compare(state, compare))
}
