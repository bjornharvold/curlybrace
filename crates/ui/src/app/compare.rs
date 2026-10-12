//! Reducer for the Compare workspace: user actions in, runtime commands out; runtime compare
//! events in, view-models out. Compare sessions have their own connections, so nothing here
//! depends on (or is reset by) the main browsing connection.
use super::reducer::Out;
use super::state::{AppState, Banner, Workspace};
use crate::viewmodel::compare::{
    CompareState, DetailView, PAIR_ID, Pick, PreflightView, ResultsView, RulesForm, RunView,
    SessionPhase, SetupForm, SideForm,
};
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::{
    CompareCommand, CompareEvent, PairOutcome, RowView, SessionId, Side,
};
use curlybrace_core::compare::results::Category;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Command;
use uuid::Uuid;

/// Everything the Compare views can ask for.
#[derive(Clone, Debug, PartialEq)]
pub enum CompareAction {
    Show,
    Hide,
    Open { source: Uuid, target: Uuid },
    CloseSession,
    PickSide(Side, Pick),
    SetKey(String),
    SetFilter(Side, String),
    SetRules(RulesForm),
    RunCheck,
    Start,
    Cancel,
    SelectCategory(Category),
    Page(i64),
    Select(KeyTuple),
    Discard,
    Back,
}

fn cmd(c: CompareCommand) -> Command {
    Command::Compare(c)
}

fn none(state: AppState) -> Out {
    (state, Vec::new())
}

fn with_compare(state: AppState, compare: CompareState) -> AppState {
    AppState { compare, ..state }
}

/// The session id, once the session is open and usable.
fn ready(state: &AppState) -> Option<SessionId> {
    match state.compare.session {
        Some(SessionPhase::Ready(id)) => Some(id),
        _ => None,
    }
}

pub(super) fn on_action(state: AppState, action: CompareAction) -> Out {
    match action {
        CompareAction::Show => none(with_workspace(state, Workspace::Compare)),
        CompareAction::Hide => none(with_workspace(state, Workspace::Browse)),
        CompareAction::Open { source, target } => open(state, source, target),
        CompareAction::CloseSession => close(state),
        CompareAction::PickSide(side, pick) => pick_side(state, side, pick),
        CompareAction::SetKey(text) => edited(state, |f| f.with_key_text(&text)),
        CompareAction::SetFilter(side, text) => edited(state, |f| {
            let form = f.side(side).clone().with_filter(&text);
            f.with_side(side, form)
        }),
        CompareAction::SetRules(rules) => edited(state, |f| f.with_rules(rules)),
        CompareAction::RunCheck => check(state),
        CompareAction::Start => start(state),
        CompareAction::Cancel => cancel(state),
        CompareAction::SelectCategory(c) => moved(state, |r| r.in_category(c)),
        CompareAction::Page(delta) => moved(state, |r| r.paged(delta)),
        CompareAction::Select(key) => select(state, key),
        CompareAction::Discard => discard(state),
        CompareAction::Back => back(state),
    }
}

fn back(state: AppState) -> Out {
    let compare = CompareState {
        detail: None,
        ..state.compare.clone()
    };
    none(with_compare(state, compare))
}

fn with_workspace(state: AppState, workspace: Workspace) -> AppState {
    AppState { workspace, ..state }
}

fn open(state: AppState, source: Uuid, target: Uuid) -> Out {
    let (Some(src), Some(tgt)) = (
        state.connection(source).cloned(),
        state.connection(target).cloned(),
    ) else {
        return none(state);
    };
    let shown = with_workspace(state, Workspace::Compare);
    if shown.compare.session.is_some() && shown.compare.connections == Some((source, target)) {
        return none(shown);
    }
    let mut commands = Vec::new();
    if let Some(old) = shown.compare.session {
        commands.push(cmd(CompareCommand::Close { session: old.id() }));
    }
    let id = shown.compare.next_session;
    let fresh = CompareState {
        session: Some(SessionPhase::Opening(id)),
        connections: Some((source, target)),
        next_session: id + 1,
        ..shown.compare.clone().session_dropped()
    };
    commands.push(cmd(CompareCommand::Open {
        session: id,
        source: src,
        target: tgt,
    }));
    (with_compare(shown, fresh), commands)
}

fn close(state: AppState) -> Out {
    let Some(phase) = state.compare.session else {
        return none(state);
    };
    let dropped = state.compare.clone().session_dropped();
    (
        with_compare(state, dropped),
        vec![cmd(CompareCommand::Close {
            session: phase.id(),
        })],
    )
}

/// Applies a form edit; any edit makes the earlier preflight answer stale.
fn edited(state: AppState, change: impl FnOnce(SetupForm) -> SetupForm) -> Out {
    let compare = CompareState {
        setup: change(state.compare.setup.clone()),
        preflight: PreflightView::Unchecked,
        // A check still in flight now answers a different form.
        check_stale: state.compare.check_in_flight,
        ..state.compare.clone()
    };
    none(with_compare(state, compare))
}

/// Picking what is already picked changes nothing: no command, no preflight reset.
fn pick_side(state: AppState, side: Side, pick: Pick) -> Out {
    let current = state.compare.setup.side(side).clone();
    let picked = match &pick {
        Pick::Database(db) => current.clone().with_database(db),
        Pick::Collection(name) => current.clone().with_collection(name),
    };
    if picked == current {
        return none(state);
    }
    let listing = match (&pick, ready(&state)) {
        (Pick::Database(db), Some(session)) => Some(cmd(CompareCommand::Collections {
            session,
            side,
            db: db.clone(),
        })),
        _ => None,
    };
    let (next, _) = edited(state, |f| f.with_side(side, picked));
    (next, listing.into_iter().collect())
}

fn check(state: AppState) -> Out {
    let Some(session) = ready(&state) else {
        return none(state);
    };
    let Ok(pair) = state.compare.setup.pair() else {
        return none(state);
    };
    if state.compare.run.running().is_some() || state.compare.check_in_flight {
        return none(state);
    }
    let compare = CompareState {
        preflight: PreflightView::Checking,
        check_in_flight: true,
        check_stale: false,
        ..state.compare.clone()
    };
    (
        with_compare(state, compare),
        vec![cmd(CompareCommand::Check { session, pair })],
    )
}

fn start(state: AppState) -> Out {
    let Some(session) = ready(&state) else {
        return none(state);
    };
    let c = &state.compare;
    if c.run.running().is_some() || c.check_in_flight || !c.preflight.allows_start() {
        return none(state);
    }
    let Ok(pair) = c.setup.pair() else {
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
        results: None,
        detail: None,
        ..c.clone()
    };
    (with_compare(state, compare), commands)
}

fn cancel(state: AppState) -> Out {
    match (ready(&state), state.compare.run.running()) {
        (Some(session), Some(run)) => (state, vec![cmd(CompareCommand::Cancel { session, run })]),
        _ => none(state),
    }
}

/// Category switch or paging: `change` yields the next results, whose page is then requested.
fn moved(state: AppState, change: impl FnOnce(&ResultsView) -> Option<ResultsView>) -> Out {
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

fn select(state: AppState, key: KeyTuple) -> Out {
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

fn discard(state: AppState) -> Out {
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

// ---- events ----

fn event_session(ev: &CompareEvent) -> SessionId {
    match ev {
        CompareEvent::Opened { session }
        | CompareEvent::OpenFailed { session, .. }
        | CompareEvent::Databases { session, .. }
        | CompareEvent::Collections { session, .. }
        | CompareEvent::Checked { session, .. }
        | CompareEvent::Progress { session, .. }
        | CompareEvent::PairDone { session, .. }
        | CompareEvent::RunDone { session, .. }
        | CompareEvent::Rows { session, .. }
        | CompareEvent::Detail { session, .. }
        | CompareEvent::Failed { session, .. } => *session,
    }
}

/// Compare events act on the compare state only; events of a session that is not the current
/// one (closed, replaced, never opened) are dropped.
pub(super) fn on_event(state: AppState, ev: CompareEvent) -> Out {
    let Some(phase) = state.compare.session else {
        return none(state);
    };
    if event_session(&ev) != phase.id() {
        return none(state);
    }
    match ev {
        run_event @ (CompareEvent::Progress { .. }
        | CompareEvent::PairDone { .. }
        | CompareEvent::RunDone { .. }
        | CompareEvent::Rows { .. }
        | CompareEvent::Detail { .. }) => on_run_event(state, run_event),
        other => on_setup_event(state, other),
    }
}

/// Session lifecycle, listings, preflight and command failures.
fn on_setup_event(state: AppState, ev: CompareEvent) -> Out {
    match ev {
        CompareEvent::Opened { session } => opened(state, session),
        CompareEvent::OpenFailed { error, .. } => open_failed(state, error),
        CompareEvent::Databases { side, names, .. } => {
            listed(state, side, |s| s.with_databases(names))
        }
        CompareEvent::Collections {
            side, db, entries, ..
        } => listed(state, side, |s| s.with_collections(&db, entries)),
        CompareEvent::Checked {
            pair, preflight, ..
        } => checked(state, pair, PreflightView::from(&preflight)),
        CompareEvent::Failed { context, error, .. } => failed(state, context, error),
        // Run events are routed to `on_run_event`.
        _ => none(state),
    }
}

/// Progress, results and details of a run.
fn on_run_event(state: AppState, ev: CompareEvent) -> Out {
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

fn failure_banner(state: AppState, context: &'static str, error: CoreError) -> AppState {
    AppState {
        banner: Some(Banner::Failure { context, error }),
        ..state
    }
}

fn opened(state: AppState, session: SessionId) -> Out {
    if state.compare.session != Some(SessionPhase::Opening(session)) {
        return none(state);
    }
    let compare = CompareState {
        session: Some(SessionPhase::Ready(session)),
        ..state.compare.clone()
    };
    let commands = [Side::Source, Side::Target]
        .into_iter()
        .map(|side| cmd(CompareCommand::Databases { session, side }))
        .collect();
    (with_compare(state, compare), commands)
}

fn open_failed(state: AppState, error: CoreError) -> Out {
    let dropped = state.compare.clone().session_dropped();
    none(failure_banner(
        with_compare(state, dropped),
        "compare_open",
        error,
    ))
}

fn listed(state: AppState, side: Side, change: impl FnOnce(SideForm) -> SideForm) -> Out {
    let form = change(state.compare.setup.side(side).clone());
    let compare = CompareState {
        setup: state.compare.setup.clone().with_side(side, form),
        ..state.compare.clone()
    };
    none(with_compare(state, compare))
}

/// The single outstanding check has been answered (or failed): nothing is in flight any more.
fn check_settled(state: AppState, preflight: PreflightView) -> AppState {
    let compare = CompareState {
        preflight,
        check_in_flight: false,
        check_stale: false,
        ..state.compare.clone()
    };
    with_compare(state, compare)
}

/// Checks carry no request identity, so at most one is in flight; an answer to a check the form
/// was edited under is dropped (the user checks again).
fn checked(state: AppState, pair: u32, view: PreflightView) -> Out {
    if pair != PAIR_ID || !state.compare.check_in_flight {
        return none(state);
    }
    let preflight = if state.compare.check_stale {
        PreflightView::Unchecked
    } else {
        view
    };
    none(check_settled(state, preflight))
}

fn check_failed(state: AppState, error: CoreError) -> Out {
    if !state.compare.check_in_flight {
        return none(failure_banner(state, "compare_check", error));
    }
    let obsolete = state.compare.check_stale;
    let next = check_settled(state, PreflightView::Unchecked);
    none(if obsolete {
        next
    } else {
        failure_banner(next, "compare_check", error)
    })
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
        PairOutcome::Cancelled(s) => (Some(ResultsView::new(s)), true, failure),
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
        },
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

/// A whole command failed: show it, and undo whatever the command had optimistically marked.
fn failed(state: AppState, context: &'static str, error: CoreError) -> Out {
    if context == "compare_check" {
        return check_failed(state, error);
    }
    let c = &state.compare;
    // `compare_rows` failures carry no offset, so they cannot be matched to a request: the
    // current request is kept (a newer one may be the live one) and the banner reports it.
    let compare = match context {
        "compare_start" if c.run.running().is_some() => CompareState {
            run: RunView::Idle,
            ..c.clone()
        },
        "compare_detail" => CompareState {
            detail: None,
            ..c.clone()
        },
        _ => c.clone(),
    };
    none(failure_banner(with_compare(state, compare), context, error))
}

#[cfg(test)]
#[path = "compare_tests.rs"]
mod tests;
