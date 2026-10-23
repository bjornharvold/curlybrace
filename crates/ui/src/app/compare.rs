//! Reducer for the Compare workspace: user actions in, runtime commands out; runtime compare
//! events in, view-models out. Compare sessions have their own connections, so nothing here
//! depends on (or is reset by) the main browsing connection.
use super::compare_apply as apply;
use super::compare_checks as checks;
use super::compare_duplicates as duplicates;
use super::compare_export as export;
use super::compare_fields as fields;
use super::compare_pairs as pairs;
use super::compare_results::{back_to_overview, discard_pair, moved, open_pair, rerun, select};
use super::compare_run::{cancel, on_run_event, start, start_passing};
use super::compare_select as selecting;
use super::reducer::Out;
use super::state::{AppState, Banner, Workspace};
use crate::viewmodel::compare::{
    CompareState, EntryId, ExportKind, PairOverrides, Pick, ResultsView, RowAction, RulesForm,
    SessionPhase, SideForm,
};
use curlybrace_core::compare::apply::IdPolicy;
use curlybrace_core::compare::diff::PathSegment;
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::{
    CompareCommand, CompareEvent, ExportFormat, SessionId, Side,
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
    /// Add the pair the pickers describe to the list.
    AddPair,
    /// Add every same-named collection pair of the two picked databases the list lacks.
    AddAllSameNamed,
    /// Pair a collection with no counterpart on `side` with a name typed for the other side.
    AddUnmatched {
        side: Side,
        name: String,
        as_name: String,
    },
    RemovePair(EntryId),
    IncludePair(EntryId, bool),
    EditPair(Option<EntryId>),
    SetPairOverrides(EntryId, PairOverrides),
    ToggleSystemCollections(bool),
    /// Check the included entries, one at a time.
    RunCheck,
    /// Run the included entries.
    Start,
    /// Run only the included entries that passed their check.
    StartPassing,
    Cancel,
    SelectCategory(Category),
    Page(i64),
    Select(KeyTuple),
    /// Show the results of an entry's finished pair.
    OpenPair(EntryId),
    BackToOverview,
    /// Throw away an entry's finished results.
    DiscardPair(EntryId),
    /// Select an entry to edit and check again, keeping its old results out of the way.
    Rerun(EntryId),
    /// Close the open detail (back to the table).
    Back,
    /// Re-list the collections of the database picked on a side (a failed listing is retried).
    Reload(Side),
    /// Ask again for the results page whose loading failed.
    RetryRows,
    /// Ask both sides for their most frequent duplicate keys of an entry that failed on a
    /// duplicate key.
    FindDuplicates(EntryId),
    /// Tick or untick one row of the loaded page of the open results tab.
    ToggleRow(KeyTuple),
    /// Tick every row of the loaded page.
    SelectPage,
    /// Tick every row of the tab, loaded or not.
    SelectAllInCategory,
    ClearSelection,
    /// Queue the action for the ticked rows of the open tab (nothing is written).
    AddToPending(RowAction),
    /// Queue the action for one loaded row of the open tab (from its detail).
    AddKeyToPending(KeyTuple, RowAction),
    RemovePending(usize),
    ClearPending,
    /// Tick or untick one changed field (by its path) of the open, loaded detail.
    ToggleField(Vec<PathSegment>),
    /// Untick every field of the open detail.
    ClearFieldSelection,
    /// Queue the ticked fields of the open detail (`CopyFieldsToTarget` or `CopyFieldsToSource`
    /// only; nothing is written).
    AddFieldsToPending(RowAction),
    /// Open the review dialog. `word` must come from `i18n.t(CONFIRM_WORD_KEY)`: the review
    /// stores it and a Word-type typed confirmation is later checked against it. A blank word
    /// can never confirm.
    ReviewApply {
        word: String,
    },
    SetConfirmText(String),
    SetIdPolicy(IdPolicy),
    /// The one action that can start a write: a no-op unless a review is open and confirmed.
    ConfirmApply,
    CancelReview,
    CancelApply,
    DismissApplyResult,
    /// Open the export modal (nothing is sent or written).
    OpenExport(ExportKind),
    SetExportFormat(ExportFormat),
    SetExportLabel(String),
    SetExportIdPolicy(IdPolicy),
    ToggleExportPair(EntryId),
    SetExportIncludePaths(bool),
    /// The file dialog opens (the app layer does that); the reducer sends nothing.
    ChooseExportFile,
    /// The only action that starts a file export: honoured in `Choosing` only.
    ExportPathChosen(crate::viewmodel::compare::RedactedPath),
    ExportPathCancelled,
    /// Copy the report to the clipboard (small reports; no file).
    CopyReport,
    CancelExport,
    CloseExport,
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
        CompareAction::SetKey(text) => pairs::defaults_edit(state, |f| f.with_key_text(&text)),
        CompareAction::SetFilter(side, text) => pairs::defaults_edit(state, |f| {
            let form = f.side(side).clone().with_filter(&text);
            f.with_side(side, form)
        }),
        CompareAction::SetRules(rules) => pairs::defaults_edit(state, |f| f.with_rules(rules)),
        other => on_pair_action(state, other),
    }
}

/// The pair list; everything else goes on to `on_run_action`.
fn on_pair_action(state: AppState, action: CompareAction) -> Out {
    match action {
        CompareAction::AddPair => pairs::add_pair(state),
        CompareAction::AddAllSameNamed => pairs::add_all_same_named(state),
        CompareAction::AddUnmatched {
            side,
            name,
            as_name,
        } => pairs::add_unmatched(state, side, &name, &as_name),
        CompareAction::RemovePair(entry) => pairs::remove_pair(state, entry),
        CompareAction::IncludePair(entry, on) => pairs::include_pair(state, entry, on),
        CompareAction::EditPair(entry) => pairs::edit_pair(state, entry),
        CompareAction::SetPairOverrides(entry, o) => pairs::set_overrides(state, entry, o),
        CompareAction::ToggleSystemCollections(on) => pairs::toggle_system(state, on),
        other => on_run_action(state, other),
    }
}

/// Checks, runs, results and the routing of the apply actions.
fn on_run_action(state: AppState, action: CompareAction) -> Out {
    match action {
        CompareAction::RunCheck => checks::run_check(state),
        CompareAction::Start => start(state),
        CompareAction::StartPassing => start_passing(state),
        CompareAction::Cancel => cancel(state),
        CompareAction::SelectCategory(c) => moved(state, |r| r.in_category(c)),
        CompareAction::Page(delta) => moved(state, |r| r.paged(delta)),
        CompareAction::Select(key) => select(state, key),
        CompareAction::OpenPair(entry) => open_pair(state, entry),
        CompareAction::BackToOverview => back_to_overview(state),
        CompareAction::DiscardPair(entry) => discard_pair(state, entry),
        CompareAction::Rerun(entry) => rerun(state, entry),
        CompareAction::Back => back(state),
        CompareAction::Reload(side) => reload(state, side),
        CompareAction::RetryRows => moved(state, ResultsView::retried),
        CompareAction::FindDuplicates(entry) => duplicates::find(state, entry),
        CompareAction::ToggleRow(_)
        | CompareAction::SelectPage
        | CompareAction::SelectAllInCategory
        | CompareAction::ClearSelection
        | CompareAction::AddToPending(_)
        | CompareAction::AddKeyToPending(..)
        | CompareAction::RemovePending(_)
        | CompareAction::ClearPending
        | CompareAction::ToggleField(_)
        | CompareAction::ClearFieldSelection
        | CompareAction::AddFieldsToPending(_)
        | CompareAction::ReviewApply { .. }
        | CompareAction::SetConfirmText(_)
        | CompareAction::SetIdPolicy(_)
        | CompareAction::ConfirmApply
        | CompareAction::CancelReview
        | CompareAction::CancelApply
        | CompareAction::DismissApplyResult => on_apply_action(state, action),
        CompareAction::OpenExport(_)
        | CompareAction::SetExportFormat(_)
        | CompareAction::SetExportLabel(_)
        | CompareAction::SetExportIdPolicy(_)
        | CompareAction::ToggleExportPair(_)
        | CompareAction::SetExportIncludePaths(_)
        | CompareAction::ChooseExportFile
        | CompareAction::ExportPathChosen(_)
        | CompareAction::ExportPathCancelled
        | CompareAction::CopyReport
        | CompareAction::CancelExport
        | CompareAction::CloseExport => export::on_action(state, action),
        // Handled by `on_action` or `on_pair_action`; listed so a new variant must be routed on purpose.
        CompareAction::AddPair
        | CompareAction::AddAllSameNamed
        | CompareAction::AddUnmatched { .. }
        | CompareAction::RemovePair(_)
        | CompareAction::IncludePair(..)
        | CompareAction::EditPair(_)
        | CompareAction::SetPairOverrides(..)
        | CompareAction::ToggleSystemCollections(_)
        | CompareAction::Show
        | CompareAction::Hide
        | CompareAction::Open { .. }
        | CompareAction::CloseSession
        | CompareAction::PickSide(..)
        | CompareAction::SetKey(_)
        | CompareAction::SetFilter(..)
        | CompareAction::SetRules(_) => none(state),
    }
}

/// Selection, field ticks and pending changes; the rest goes on to `on_review_action`.
fn on_apply_action(state: AppState, action: CompareAction) -> Out {
    match action {
        CompareAction::ToggleRow(key) => selecting::toggle_row(state, key),
        CompareAction::SelectPage => selecting::select_page(state),
        CompareAction::SelectAllInCategory => selecting::select_all(state),
        CompareAction::ClearSelection => selecting::clear_selection(state),
        CompareAction::AddToPending(a) => selecting::add_selection(state, a),
        CompareAction::AddKeyToPending(key, a) => selecting::add_key(state, key, a),
        CompareAction::RemovePending(index) => selecting::remove_pending(state, index),
        CompareAction::ClearPending => selecting::clear_pending(state),
        CompareAction::ToggleField(path) => fields::toggle_field(state, path),
        CompareAction::ClearFieldSelection => fields::clear_fields(state),
        CompareAction::AddFieldsToPending(a) => fields::add_fields(state, a),
        other => on_review_action(state, other),
    }
}

/// Review, confirmation and the apply run.
fn on_review_action(state: AppState, action: CompareAction) -> Out {
    match action {
        CompareAction::ReviewApply { word } => apply::review(state, &word),
        CompareAction::SetConfirmText(text) => apply::set_confirm_text(state, &text),
        CompareAction::SetIdPolicy(policy) => apply::set_id_policy(state, policy),
        CompareAction::ConfirmApply => apply::confirm_apply(state),
        CompareAction::CancelReview => apply::cancel_review(state),
        CompareAction::CancelApply => apply::cancel_apply(state),
        CompareAction::DismissApplyResult => apply::dismiss_result(state),
        // Only `on_apply_action` calls this, with the actions it does not handle itself; the
        // earlier groups list every other variant, and the exhaustive no-write test covers all.
        _ => none(state),
    }
}

/// Asks again for the collections of the side's picked database; the form is left as it is.
fn reload(state: AppState, side: Side) -> Out {
    let command = ready(&state)
        .zip(state.compare.setup.side(side).database.clone())
        .map(|(session, db)| cmd(CompareCommand::Collections { session, side, db }));
    (state, command.into_iter().collect())
}

/// Opens (or reuses) the session a sidebar entry starts from: the connected server on the source
/// side, and the session's current target (else the same server) on the target side. Without a
/// connection the banner is shown instead. The flag tells whether the sides use different servers.
fn from_sidebar(state: AppState, then: impl FnOnce(AppState, Vec<Command>, bool) -> Out) -> Out {
    let Some(source) = state.active.filter(|id| state.connection(*id).is_some()) else {
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
    // Opening for other connections would replace the session an apply is reviewed or runs on:
    // show the workspace and leave the session and the pickers alone.
    if state.compare.workflow_busy() && state.compare.connections != Some((source, target)) {
        return none(with_workspace(state, Workspace::Compare));
    }
    let (state, commands) = open(state, source, target);
    then(state, commands, source != target)
}

/// "Compare to..." on a collection: the source side starts at that collection. The session is
/// opened for the connected server on both sides unless one is already open for this source.
pub(super) fn compare_to(state: AppState, ns: Namespace) -> Out {
    from_sidebar(state, |state, mut commands, _| {
        let (state, listing) = pick_side(state, Side::Source, Pick::Database(ns.db));
        let (state, collection) = pick_side(state, Side::Source, Pick::Collection(ns.collection));
        commands.extend(listing);
        commands.extend(collection);
        (state, commands)
    })
}

/// "Compare database to..." on a database header: the source side starts at that database, and
/// the target side at the same-named database when it is on another connection.
pub(super) fn compare_database_to(state: AppState, db: String) -> Out {
    from_sidebar(state, |state, mut commands, differs| {
        let (state, listing) = pick_side(state, Side::Source, Pick::Database(db.clone()));
        commands.extend(listing);
        if !differs {
            return (state, commands);
        }
        let (state, listing) = pick_side(state, Side::Target, Pick::Database(db));
        commands.extend(listing);
        (state, commands)
    })
}

fn back(state: AppState) -> Out {
    let compare = state.compare.clone().with_detail(None);
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
    // Replacing the session would drop an apply that is being reviewed or runs.
    if shown.compare.workflow_busy() {
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
        connections_production: (src.production, tgt.production),
        connections_read_only: (src.read_only, tgt.read_only),
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
    let Some(phase) = state.compare.session.filter(|_| state.compare.can_close()) else {
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
    // The pickers only feed Add pair: no entry depends on them, so no verdict goes stale.
    let setup = state.compare.setup.clone().with_side(side, picked);
    let compare = CompareState {
        setup,
        ..state.compare.clone()
    };
    (with_compare(state, compare), listing.into_iter().collect())
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
        | CompareEvent::ApplyProgress { session, .. }
        | CompareEvent::ApplyPairDone { session, .. }
        | CompareEvent::ApplyDone { session, .. }
        | CompareEvent::ExportProgress { session, .. }
        | CompareEvent::ExportDone { session, .. }
        | CompareEvent::ExportText { session, .. }
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
        apply_event @ (CompareEvent::ApplyProgress { .. }
        | CompareEvent::ApplyPairDone { .. }
        | CompareEvent::ApplyDone { .. }) => apply::on_event(state, apply_event),
        export_event @ (CompareEvent::ExportProgress { .. }
        | CompareEvent::ExportDone { .. }
        | CompareEvent::ExportText { .. }) => export::on_event(state, export_event),
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
        } => checks::checked(state, pair, preflight),
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
    let setup = state.compare.setup.clone().with_side(side, form);
    let compare = CompareState {
        setup,
        ..state.compare.clone()
    };
    none(with_compare(state, compare))
}

/// `compare_rows` failures carry no pair or offset, so they cannot be matched to a request:
/// every view waiting for a page is marked failed (its request is kept so the user can retry
/// it, also after opening a pair that was not on show) and the banner reports it. With no view
/// waiting, the request was dropped by us (a Discard raced it) and nothing is reported.
fn rows_failed(state: AppState, error: CoreError) -> Out {
    let (compare, any) = state.compare.clone().rows_failed_all();
    let next = with_compare(state, compare);
    match any {
        true => none(failure_banner(next, "compare_rows", error)),
        false => none(next),
    }
}

/// A whole command failed: show it, and undo whatever the command had optimistically marked.
fn failed(state: AppState, context: &'static str, error: CoreError) -> Out {
    if context == "compare_check" {
        return checks::check_failed(state, error);
    }
    if context == "compare_duplicates" {
        return none(duplicates::failed(state, error));
    }
    if context == "compare_apply" {
        return apply::failed(state, error);
    }
    if context == "compare_export" {
        return export::failed(state, error);
    }
    if context == "compare_rows" {
        return rows_failed(state, error);
    }
    let c = &state.compare;
    let compare = match context {
        // The runtime refused the whole start: only this run's pending rows fail.
        "compare_start" if c.is_running() => CompareState {
            overview: c.overview.clone().start_refused(&error),
            ..c.clone()
        },
        "compare_detail" => c.clone().with_detail(None),
        _ => c.clone(),
    };
    none(failure_banner(with_compare(state, compare), context, error))
}

#[cfg(test)]
#[path = "compare_tests.rs"]
pub(in crate::app) mod tests;
