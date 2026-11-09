//! Reducer arms of the bulk update / delete flow: opening it on the displayed filter, stepping
//! through it, the write through the guard, and the answers of the runtime.
//!
//! Every count, preview and sample request carries an id from the state's counter, and an answer
//! is applied only when it carries the id still awaited, for the flow's own session and
//! collection. A Back, a Close or a new definition therefore leaves no answer that could describe
//! something other than what will run.
use super::actions::{gated, issue_find, mutation};
use super::bulk_state::BulkState;
use super::reducer::{Out, failure, mutated};
use super::state::{AppState, Banner};
use super::write_guard::{Guard, WriteKind};
use crate::viewmodel::{
    BulkEntry, BulkFlow, BulkKind, BulkStep, CancelNote, CountState, PreviewState,
};
use bson::Document;
use curlybrace_core::bulk::{
    BulkDeleteRequest, BulkOutcome, BulkRunId, BulkUpdateRequest, CancelEffect, MatchScope,
    UpdatePreview, parse_update,
};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Command;
use curlybrace_core::store::Namespace;
use uuid::Uuid;

/// What the person does in the bulk dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum BulkAction {
    Open(BulkKind),
    SetUpdateText(String),
    SetUpsert(bool),
    SetAllDocuments(bool),
    SetTyped(String),
    Next,
    Back,
    Run,
    Cancel,
    Close,
}

fn unchanged(state: AppState) -> Out {
    (state, Vec::new())
}

pub(super) fn on_action(state: AppState, action: BulkAction) -> Out {
    match action {
        BulkAction::Open(kind) => open(state, kind),
        other => in_flow(state, other),
    }
}

/// Runs `action` on the open flow, as its own session: the dialog may outlive a switch of the
/// focused session, and the commands must name the session the flow was opened on.
fn in_flow(state: AppState, action: BulkAction) -> Out {
    let Some(session) = state.bulk.as_ref().map(|b| b.session) else {
        return unchanged(state);
    };
    // The session was closed: nothing is left to report on, so even a running flow goes.
    if !state.is_open(session) {
        return unchanged(state.bulk_dropped_for(session, false));
    }
    state.with_session(session, |s| apply(s, action))
}

fn apply(state: AppState, action: BulkAction) -> Out {
    let Some(bulk) = state.bulk.clone() else {
        return unchanged(state);
    };
    let running = bulk.flow.step == BulkStep::Running;
    // A reconnect since the flow opened: it was aimed at an opening that is gone.
    if bulk.epoch != state.latest_requested_epoch && !running {
        return unchanged(state.bulk_dropped_for(bulk.session, false));
    }
    match action {
        BulkAction::Open(_) => unchanged(state),
        BulkAction::SetUpdateText(text) => edited(state, |f| f.with_update_text(text)),
        BulkAction::SetUpsert(on) => edited(state, |f| f.with_upsert(on)),
        BulkAction::SetAllDocuments(on) => edited(state, |f| f.with_all_documents(on)),
        BulkAction::SetTyped(text) => edited(state, |f| f.with_typed(text)),
        BulkAction::Next => next(state, bulk),
        BulkAction::Back => back(state, bulk),
        BulkAction::Run => run(state, bulk),
        BulkAction::Cancel if running => cancel_run(state, &bulk),
        BulkAction::Cancel | BulkAction::Close if !running => closed(state),
        BulkAction::Close | BulkAction::Cancel => unchanged(state),
    }
}

fn with_bulk(state: AppState, bulk: BulkState) -> AppState {
    AppState {
        bulk: Some(bulk),
        ..state
    }
}

fn edited(state: AppState, edit: impl FnOnce(BulkFlow) -> BulkFlow) -> Out {
    let next = state.bulk.clone().map(|b| {
        let flow = edit(b.flow.clone());
        b.with_flow(flow)
    });
    match next {
        Some(bulk) => (with_bulk(state, bulk), Vec::new()),
        None => unchanged(state),
    }
}

fn closed(state: AppState) -> Out {
    unchanged(AppState {
        bulk: None,
        ..state
    })
}

/// Opens the flow on the displayed page's filter ALONE (sort, paging and projection belong to the
/// view, not to the write) and counts what it matches.
fn open(state: AppState, kind: BulkKind) -> Out {
    gated(state, |s| {
        match s.bulk_entry() {
            BulkEntry::ReadOnly => {
                let banner = Some(Banner::ReadOnly);
                return unchanged(AppState { banner, ..s });
            }
            BulkEntry::Available => {}
            BulkEntry::Projected | BulkEntry::Busy => return unchanged(s),
        }
        let Some(ns) = s.selected_ns().cloned() else {
            return unchanged(s);
        };
        let filter = s.page_query.filter.clone();
        let flow = BulkFlow::open(kind, ns.clone(), filter.clone(), s.active_is_production());
        let (request, s) = numbered(s);
        let command = Command::BulkCount {
            session: s.session_id(),
            ns,
            filter,
            request,
        };
        let bulk = BulkState {
            flow,
            session: s.session_id(),
            epoch: s.latest_requested_epoch,
            count_request: Some(request),
            preview_request: None,
        };
        (with_bulk(s, bulk), vec![command])
    })
}

impl AppState {
    /// Whether a bulk flow may be opened now, and if not, why. `Open` and the documents toolbar
    /// both go through here: read-only first, then a projected page, then a flow or a write
    /// already in the way.
    pub fn bulk_entry(&self) -> BulkEntry {
        if self.active_is_read_only() {
            BulkEntry::ReadOnly
        } else if self.page_is_projected() {
            BulkEntry::Projected
        } else if self.bulk.is_some() || self.pending.is_some() || self.write_warning.is_some() {
            BulkEntry::Busy
        } else {
            BulkEntry::Available
        }
    }
}

/// The next request id, and the state that counted it.
fn numbered(state: AppState) -> (u64, AppState) {
    let request = state.bulk_requests + 1;
    (
        request,
        AppState {
            bulk_requests: request,
            ..state
        },
    )
}

/// The scope a PREVIEW is sent with. The core's preview checks the scope like the write does, and
/// the all-documents tick happens only at Confirm, after the preview; so an empty filter is
/// previewed with `AllDocuments`. That is safe: the preview's transaction is always aborted (and
/// it only reads when transactions are unavailable), so it writes nothing. `Run` never uses this:
/// it always sends `flow.scope()`, which is `AllDocuments` only for an empty filter that was
/// ticked.
fn preview_scope(filter: &Document) -> MatchScope {
    match filter.is_empty() {
        true => MatchScope::AllDocuments,
        false => MatchScope::Filtered,
    }
}

fn next(state: AppState, bulk: BulkState) -> Out {
    let advanced = bulk.flow.clone().advanced();
    if advanced.step == bulk.flow.step {
        return unchanged(state);
    }
    if bulk.flow.step != BulkStep::Define {
        return unchanged(with_bulk(state, bulk.with_flow(advanced)));
    }
    // The connection was locked since the flow opened: the preview executes the user's update
    // code, so the core would refuse it. Say so now instead of waiting for the refusal.
    if state.active_is_read_only() {
        return unchanged(preview_refused(state));
    }
    let (request, state) = numbered(state);
    let Some(command) = preview_command(&state, &bulk.flow, request) else {
        return unchanged(state);
    };
    let bulk = BulkState {
        preview_request: Some(request),
        ..bulk.with_flow(advanced)
    };
    (with_bulk(state, bulk), vec![command])
}

/// The request that previews what the flow defines: the update's result, or the documents a
/// delete would remove.
fn preview_command(state: &AppState, flow: &BulkFlow, request: u64) -> Option<Command> {
    let (session, ns) = (state.session_id(), flow.ns.clone());
    match flow.kind {
        BulkKind::Delete => Some(Command::BulkSample {
            session,
            ns,
            filter: flow.filter.clone(),
            request,
        }),
        BulkKind::Update => Some(Command::BulkPreview {
            session,
            ns,
            request: BulkUpdateRequest {
                filter: flow.filter.clone(),
                update: parse_update(&flow.update_text).ok()?,
                upsert: flow.upsert,
                scope: preview_scope(&flow.filter),
            },
            request_id: request,
        }),
    }
}

fn back(state: AppState, bulk: BulkState) -> Out {
    let leaves_preview = bulk.flow.step == BulkStep::Preview;
    let flow = bulk.flow.clone().back();
    let preview_request = match leaves_preview {
        true => None,
        false => bulk.preview_request,
    };
    let bulk = BulkState {
        preview_request,
        ..bulk.with_flow(flow)
    };
    unchanged(with_bulk(state, bulk))
}

/// Sends the write through the guard. The dialog's typed confirmation is the acknowledgement, so
/// a production connection is not asked a second time.
fn run(state: AppState, bulk: BulkState) -> Out {
    let flow = &bulk.flow;
    if !flow.can_run() {
        return unchanged(state);
    }
    // The connection was locked since the flow opened: the write is refused and the flow over.
    if state.active_is_read_only() {
        return unchanged(write_refused(state, Banner::ReadOnly));
    }
    // The person moved to another collection: what was previewed is not what would run.
    if state.selected_ns() != Some(&flow.ns) {
        return closed(state);
    }
    let run = BulkRunId(Uuid::new_v4());
    let Some((kind, build)) = write_of(flow, run) else {
        return unchanged(state);
    };
    let (next, commands) = mutation(state, None, kind, Guard::Confirmed, build);
    if commands.is_empty() {
        return (next, commands);
    }
    let running = bulk.flow.clone().running(run);
    (with_bulk(next, bulk.with_flow(running)), commands)
}

type Build = Box<dyn FnOnce(curlybrace_core::runtime::SessionId, Namespace) -> Command>;

fn write_of(flow: &BulkFlow, run: BulkRunId) -> Option<(WriteKind, Build)> {
    let filter = flow.filter.clone();
    let scope = flow.scope();
    match flow.kind {
        BulkKind::Delete => {
            let request = BulkDeleteRequest { filter, scope };
            let build: Build = Box::new(move |session, ns| Command::BulkDelete {
                session,
                ns,
                request,
                run,
            });
            Some((WriteKind::BulkDelete, build))
        }
        BulkKind::Update => {
            let request = BulkUpdateRequest {
                filter,
                update: parse_update(&flow.update_text).ok()?,
                upsert: flow.upsert,
                scope,
            };
            let build: Build = Box::new(move |session, ns| Command::BulkUpdate {
                session,
                ns,
                request,
                run,
            });
            Some((WriteKind::BulkUpdate, build))
        }
    }
}

fn cancel_run(state: AppState, bulk: &BulkState) -> Out {
    let Some(run) = bulk.flow.run else {
        return unchanged(state);
    };
    let session = state.session_id();
    (state, vec![Command::BulkCancel { session, run }])
}

// ------------------------------------------------------------------ answers

/// The flow when it belongs to the session being handled and to `ns`.
fn flow_of<'a>(state: &'a AppState, ns: Option<&Namespace>) -> Option<&'a BulkState> {
    state
        .bulk
        .as_ref()
        .filter(|b| b.session == state.session_id() && ns.is_none_or(|n| b.flow.ns == *n))
}

/// Replaces the flow with what `take` makes of it; `None` leaves the state as it is.
fn updated(
    state: AppState,
    ns: Option<&Namespace>,
    take: impl FnOnce(&BulkState) -> Option<BulkState>,
) -> AppState {
    match flow_of(&state, ns).and_then(take) {
        Some(bulk) => with_bulk(state, bulk),
        None => state,
    }
}

fn count_settled(
    state: AppState,
    ns: Option<&Namespace>,
    request: u64,
    count: CountState,
) -> AppState {
    updated(state, ns, |b| {
        (b.count_request == Some(request)).then(|| BulkState {
            count_request: None,
            ..b.clone().with_flow(b.flow.clone().with_count(count))
        })
    })
}

fn preview_settled(
    state: AppState,
    ns: Option<&Namespace>,
    request: u64,
    preview: PreviewState,
) -> AppState {
    updated(state, ns, |b| {
        (b.preview_request == Some(request)).then(|| BulkState {
            preview_request: None,
            ..b.clone().with_flow(b.flow.clone().with_preview(preview))
        })
    })
}

pub(super) fn count_arrived(
    state: AppState,
    ns: Namespace,
    request: u64,
    count: Option<u64>,
) -> AppState {
    let count = count.map_or(CountState::Unavailable, CountState::Known);
    count_settled(state, Some(&ns), request, count)
}

/// A numbered bulk read failed, settled by its own id. A count that failed (or outran the
/// client-side bound) is unavailable, which asks for the typed confirmation; a sample that failed
/// leaves the preview failed. A failure of an older request, or of a context that is no numbered
/// read of the flow, changes nothing.
pub(super) fn read_failed(
    state: AppState,
    context: &str,
    request: u64,
    error: CoreError,
) -> AppState {
    match context {
        "bulk_count" => count_settled(state, None, request, CountState::Unavailable),
        "bulk_sample" => preview_settled(state, None, request, PreviewState::Failed(error)),
        _ => state,
    }
}

pub(super) fn sample_arrived(
    state: AppState,
    ns: Namespace,
    request: u64,
    docs: Vec<Document>,
) -> AppState {
    preview_settled(state, Some(&ns), request, PreviewState::Sample(docs))
}

pub(super) fn preview_arrived(
    state: AppState,
    ns: Namespace,
    request: u64,
    outcome: Result<UpdatePreview, CoreError>,
) -> AppState {
    let preview = outcome.map_or_else(PreviewState::Failed, PreviewState::Update);
    preview_settled(state, Some(&ns), request, preview)
}

/// The run finished. The flow of that run reports it and the page is read again, like after any
/// write; a finish that belongs to no open flow (it was closed with its session, or it is an
/// older run) changes no flow and clears no write in flight, but the page is still read again.
pub(super) fn finished(
    state: AppState,
    ns: Namespace,
    run: BulkRunId,
    outcome: Result<BulkOutcome, CoreError>,
) -> Out {
    let ours = flow_of(&state, Some(&ns))
        .filter(|b| b.flow.run == Some(run))
        .cloned();
    let Some(bulk) = ours else {
        return match state.selected_ns() == Some(&ns) {
            true => {
                let query = state.page_query.clone();
                issue_find(state, query)
            }
            false => unchanged(state),
        };
    };
    let reported = bulk.flow.clone().reported(outcome);
    mutated(with_bulk(state, bulk.with_flow(reported)), ns)
}

/// What a cancel request came to is told on the flow; the run carries on whatever it was and its
/// finish comes. Nothing found to stop leaves Stop usable, so the person can try again.
pub(super) fn cancel_answered(
    state: AppState,
    run: BulkRunId,
    outcome: Result<CancelEffect, CoreError>,
) -> AppState {
    let note = match outcome {
        Ok(CancelEffect::Stopped) => CancelNote::Requested,
        Ok(CancelEffect::NothingToStop) => CancelNote::NothingToStop,
        Err(_) => CancelNote::Unavailable,
    };
    updated(state, None, |b| {
        (b.flow.run == Some(run))
            .then(|| b.clone().with_flow(b.flow.clone().with_cancel_note(note)))
    })
}

/// A bulk command failed as a command (a refusal, not a lost server). A read-only refusal closes
/// the flow with the read-only banner; the reads degrade inline; a refused write never ran, so
/// its flow closes and the failure is told in the failure banner.
pub(super) fn command_failed(state: AppState, context: &'static str, error: CoreError) -> AppState {
    if error == CoreError::ReadOnly {
        return match context {
            // A preview has no write in flight: only the flow waiting for it is over.
            "bulk_preview" => preview_read_only(state),
            _ => write_failed(state, Banner::ReadOnly),
        };
    }
    let banner = Banner::Failure {
        context,
        error: error.clone(),
    };
    match context {
        // The runtime sends the command-level failure of a preview (it has no request id).
        // Counts and samples fail as `RequestFailed`, see `read_failed`.
        "bulk_preview" => preview_failed(state, error),
        "bulk_cancel" => with_banner(state, banner),
        _ => write_failed(state, banner),
    }
}

fn with_banner(state: AppState, banner: Banner) -> AppState {
    match banner {
        Banner::Failure { context, error } => failure(state, context, error),
        other => AppState {
            banner: Some(other),
            ..state
        },
    }
}

/// A write command failed. Only the open flow's own write in flight can be refused: a late
/// `Failed` of an older write (the flow is closed, reopened or not running) changes nothing.
fn write_failed(state: AppState, banner: Banner) -> AppState {
    let in_flight = state.pending.is_some()
        && flow_of(&state, None).is_some_and(|b| b.flow.step == BulkStep::Running);
    match in_flight {
        true => write_refused(state, banner),
        false => state,
    }
}

/// The write never ran: nothing is in flight and the flow is over.
fn write_refused(state: AppState, banner: Banner) -> AppState {
    let state = AppState {
        bulk: None,
        pending: None,
        ..state
    };
    with_banner(state, banner)
}

/// The preview was refused for a read-only connection: the flow ends with the read-only banner.
/// Only the flow that awaits a preview ends; a late refusal of an older preview changes nothing.
fn preview_read_only(state: AppState) -> AppState {
    let waiting = flow_of(&state, None).is_some_and(|b| b.preview_request.is_some());
    match waiting {
        true => preview_refused(state),
        false => state,
    }
}

/// Closes the flow whose preview could not run, with the read-only banner. A write of another
/// kind that is in flight (`pending`) is not this flow's and stays.
fn preview_refused(state: AppState) -> AppState {
    let state = AppState {
        bulk: None,
        ..state
    };
    with_banner(state, Banner::ReadOnly)
}

fn preview_failed(state: AppState, error: CoreError) -> AppState {
    let request = flow_of(&state, None).and_then(|b| b.preview_request);
    match request {
        Some(request) => preview_settled(state, None, request, PreviewState::Failed(error)),
        None => state,
    }
}

impl AppState {
    /// Closes the bulk flow of `session`. A running flow stays when `keep_running`: the runtime
    /// delivers its finish whatever happened to the connection, and the report of a write whose
    /// result is not known must not be lost. Callers: a lost connection (`failed`) and a reconnect
    /// (`connect`) keep it, because the session lives on; closing the session
    /// (`session_dropped`) and a flow whose session is no longer open do not, because the report
    /// would have no session view to land on and the flow would block every later bulk flow.
    pub fn bulk_dropped_for(self, session: Uuid, keep_running: bool) -> AppState {
        let drops = self.bulk.as_ref().is_some_and(|b| {
            b.session == session && !(keep_running && b.flow.step == BulkStep::Running)
        });
        match drops {
            true => AppState { bulk: None, ..self },
            false => self,
        }
    }
}
