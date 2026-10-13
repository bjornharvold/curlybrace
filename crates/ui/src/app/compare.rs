//! Reducer for the Compare workspace: user actions in, runtime commands out; runtime compare
//! events in, view-models out. Compare sessions have their own connections, so nothing here
//! depends on (or is reset by) the main browsing connection.
use super::compare_duplicates as duplicates;
use super::compare_run::{cancel, discard, moved, on_run_event, select, start};
use super::reducer::Out;
use super::state::{AppState, Banner, Workspace};
use crate::viewmodel::compare::{
    CompareState, PAIR_ID, Pick, PreflightView, ResultsView, RulesForm, RunView, SessionPhase,
    SetupForm, SideForm,
};
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::{
    CompareCommand, CompareEvent, Preflight, SessionId, Side,
};
use curlybrace_core::compare::results::Category;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Command;
use curlybrace_core::store::Namespace;
use uuid::Uuid;

/// Everything the Compare views can ask for.
#[derive(Clone, Debug, PartialEq)]
pub enum CompareAction {
    Show,
    Hide,
    Open {
        source: Uuid,
        target: Uuid,
    },
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
    /// Re-list the collections of the database picked on a side (a failed listing is retried).
    Reload(Side),
    /// Ask again for the results page whose loading failed.
    RetryRows,
    /// Ask both sides for their most frequent duplicate keys (after a duplicate-key failure).
    FindDuplicates,
}

pub(super) fn cmd(c: CompareCommand) -> Command {
    Command::Compare(c)
}

pub(super) fn none(state: AppState) -> Out {
    (state, Vec::new())
}

pub(super) fn with_compare(state: AppState, compare: CompareState) -> AppState {
    AppState { compare, ..state }
}

/// The session id, once the session is open and usable.
pub(super) fn ready(state: &AppState) -> Option<SessionId> {
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
        CompareAction::Reload(side) => reload(state, side),
        CompareAction::RetryRows => moved(state, ResultsView::retried),
        CompareAction::FindDuplicates => duplicates::find(state),
    }
}

/// Asks again for the collections of the side's picked database; the form is left as it is.
fn reload(state: AppState, side: Side) -> Out {
    let command = ready(&state)
        .zip(state.compare.setup.side(side).database.clone())
        .map(|(session, db)| cmd(CompareCommand::Collections { session, side, db }));
    (state, command.into_iter().collect())
}

/// "Compare to..." on a collection: the source side starts at that collection. The session is
/// opened for the connected server on both sides unless one is already open for this source.
pub(super) fn compare_to(state: AppState, ns: Namespace) -> Out {
    let Some(source) = state.connected.filter(|id| state.connection(*id).is_some()) else {
        return none(AppState {
            banner: Some(Banner::NotConnected),
            ..state
        });
    };
    let target = state
        .compare
        .connections
        .map(|(_, t)| t)
        .filter(|t| state.connection(*t).is_some())
        .unwrap_or(source);
    let (state, mut commands) = open(state, source, target);
    let (state, listing) = pick_side(state, Side::Source, Pick::Database(ns.db));
    let (state, collection) = pick_side(state, Side::Source, Pick::Collection(ns.collection));
    commands.extend(listing);
    commands.extend(collection);
    (state, commands)
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
        last_check: None,
        // A check still in flight now answers a different form.
        check_stale: state.compare.check_in_flight,
        duplicates: state.compare.duplicates.superseded(),
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
    let Ok(pair) = state.compare.pair() else {
        return none(state);
    };
    if state.compare.run.running().is_some() || state.compare.check_in_flight {
        return none(state);
    }
    let compare = CompareState {
        preflight: PreflightView::Checking,
        last_check: None,
        check_in_flight: true,
        check_stale: false,
        ..state.compare.clone()
    };
    (
        with_compare(state, compare),
        vec![cmd(CompareCommand::Check { session, pair })],
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
        | CompareEvent::DuplicateKeys { session, .. }
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
        } => checked(state, pair, preflight),
        CompareEvent::DuplicateKeys {
            pair,
            source,
            target,
            ..
        } => duplicates::answered(state, pair, source, target),
        CompareEvent::Failed { context, error, .. } => failed(state, context, error),
        // Run events are routed to `on_run_event`.
        _ => none(state),
    }
}

pub(super) fn failure_banner(state: AppState, context: &'static str, error: CoreError) -> AppState {
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
    let sides = [Side::Source, Side::Target];
    let mut commands: Vec<Command> = sides
        .into_iter()
        .map(|side| cmd(CompareCommand::Databases { session, side }))
        .collect();
    // A side filled in before the session was ready (the sidebar entry) still needs its listing.
    commands.extend(sides.into_iter().filter_map(|side| {
        let db = state.compare.setup.side(side).database.clone()?;
        Some(cmd(CompareCommand::Collections { session, side, db }))
    }));
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
fn check_settled(
    state: AppState,
    preflight: PreflightView,
    last_check: Option<Preflight>,
) -> AppState {
    let compare = CompareState {
        preflight,
        last_check,
        check_in_flight: false,
        check_stale: false,
        ..state.compare.clone()
    };
    with_compare(state, compare)
}

/// Checks carry no request identity, so at most one is in flight; an answer to a check the form
/// was edited under is dropped (the user checks again).
fn checked(state: AppState, pair: u32, answer: Preflight) -> Out {
    if pair != PAIR_ID || !state.compare.check_in_flight {
        return none(state);
    }
    if state.compare.check_stale {
        return none(check_settled(state, PreflightView::Unchecked, None));
    }
    let view = PreflightView::from(&answer);
    none(check_settled(state, view, Some(answer)))
}

fn check_failed(state: AppState, error: CoreError) -> Out {
    if !state.compare.check_in_flight {
        return none(failure_banner(state, "compare_check", error));
    }
    let obsolete = state.compare.check_stale;
    let next = check_settled(state, PreflightView::Unchecked, None);
    none(if obsolete {
        next
    } else {
        failure_banner(next, "compare_check", error)
    })
}

/// A whole command failed: show it, and undo whatever the command had optimistically marked.
fn failed(state: AppState, context: &'static str, error: CoreError) -> Out {
    if context == "compare_check" {
        return check_failed(state, error);
    }
    if context == "compare_duplicates" {
        return none(duplicates::failed(state, error));
    }
    let c = &state.compare;
    // `compare_rows` failures carry no offset, so they cannot be matched to a request: the
    // current request is kept (a newer one may be the live one), marked failed so the user can
    // retry it, and the banner reports it.
    let compare = match context {
        "compare_rows" => CompareState {
            results: c.results.clone().map(ResultsView::rows_failed),
            ..c.clone()
        },
        "compare_start" if c.run.running().is_some() => CompareState {
            run: RunView::Idle,
            cancel_requested: false,
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
