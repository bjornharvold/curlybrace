//! A document export streams the documents of one collection of an OPEN session to a file the
//! person chose, as NDJSON or as a JSON array (see [`crate::export`]). Like a copy run it opens no
//! connection of its own: it reads through the store of the session named in the request, so the
//! session's epoch rules apply. Export commands carry no session of their own and have no
//! per-command answer; a run answers with a stream of [`ExportEvent`]s.
//!
//! `dispatch` registers a run synchronously (path usable, session open and connected, the one run
//! slot free), so command ORDER decides outcomes: a second `Start` behind a running one is
//! answered with `Failed` for ITS run id and never disturbs the running one. The file is
//! created and written in a task.
//!
//! Event guarantees, per run:
//! - A refusal (bad path, session not open, slot taken) is a single `Failed`, sent before
//!   anything is created.
//! - Otherwise `Progress`* (after each batch read) and exactly one terminal event, always last:
//!   `Done` or `Failed`.
//! - `Cancel { run }` ends the run with `Done { cancelled: true, file_left }`, the file removed
//!   (`file_left` only when removing it failed). A cancel of an unknown run is ignored.
//! - An error (a read or write failure, a document without a usable `_id`, a panic) or a session
//!   that was closed or reconnected mid-run (`Superseded`) ends with `Failed` and the file
//!   removed; when it cannot be removed the error is `ExportFileNotRemoved`. The dispatcher
//!   cancels the run the moment a `Connect` or `Disconnect` of its session is requested
//!   (`ExportSlot::session_changing`), which abandons a read in flight.
//! - Only a run that wrote everything and synced the file keeps it.
//! - The slot is released BEFORE the terminal event is sent, so a client that reacts to it with
//!   a new run is never refused.
//!
//! Document values, filters and paths are data and are never logged or put in an error.

mod run;
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc::UnboundedSender;

use super::session::Session;
use super::{Event, Repaint, SessionId, Shared};
use crate::compare::engine::CancelFlag;
use crate::compare::protocol::RunId;
use crate::error::{CoreError, Result};
use crate::export::protocol::{DocExportRequest, ExportCommand, ExportEvent};
use crate::runtime::export_file::{FileGuard, validate_path};
use crate::store::DocumentStore;

type Events = UnboundedSender<Event>;

/// The one active export run of the runtime, if any.
#[derive(Default)]
pub(super) struct ExportSlot {
    active: Mutex<Option<Active>>,
}

/// What the dispatcher needs to know about the running export.
struct Active {
    run: RunId,
    cancel: CancelFlag,
    session: SessionId,
    /// The run's `Job::lost`: set when its session changed epoch.
    lost: Arc<AtomicBool>,
}

impl ExportSlot {
    fn take(&self, active: Active) -> Result<()> {
        let mut slot = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return Err(CoreError::Unsupported(
                "an export is already running".into(),
            ));
        }
        *slot = Some(active);
        Ok(())
    }

    fn cancel(&self, run: RunId) {
        let active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = active.as_ref().filter(|a| a.run == run) {
            active.cancel.cancel();
        }
    }

    /// Called synchronously by the dispatcher, before a `Connect` or `Disconnect` of `session`
    /// moves its epoch: a run over that session is marked lost and cancelled at once, so even a
    /// read in flight is abandoned. Other runs are not touched.
    pub(super) fn session_changing(&self, session: SessionId) {
        let active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = active.as_ref().filter(|a| a.session == session) {
            active.lost.store(true, Ordering::SeqCst);
            active.cancel.cancel();
        }
    }
}

/// Frees the slot however the run ends. It only ever clears the slot holding its own run.
struct SlotGuard {
    shared: Arc<Shared>,
    run: RunId,
}

impl SlotGuard {
    fn release(&self) {
        let mut active = self
            .shared
            .export
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if active.as_ref().is_some_and(|a| a.run == self.run) {
            *active = None;
        }
    }
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        self.release();
    }
}

/// Sends the events of one run.
#[derive(Clone)]
struct RunCtx {
    events: Events,
    repaint: Repaint,
    run: RunId,
}

impl RunCtx {
    fn emit(&self, event: ExportEvent) {
        if self.events.send(Event::Export(event)).is_ok() {
            (self.repaint)();
        }
    }

    fn failed(&self, error: CoreError) -> ExportEvent {
        ExportEvent::Failed {
            run: self.run,
            error,
        }
    }
}

/// Everything a run task needs.
struct Job {
    ctx: RunCtx,
    request: DocExportRequest,
    session: Arc<Session>,
    /// The epoch the run was registered in.
    generation: u64,
    store: Arc<dyn DocumentStore>,
    /// Bound for the count that gives the progress denominator.
    limit: Duration,
    cancel: CancelFlag,
    /// Set when the session was found stale, or the dispatcher saw its epoch change; the run then
    /// ends with `Superseded`.
    lost: Arc<AtomicBool>,
}

impl Job {
    /// True once the run must stop: cancelled, or the session left the epoch the run started in
    /// (recorded, and the flag cancelled, so a read in flight is abandoned too).
    fn stopped(&self) -> bool {
        if !self.session.is_current(self.generation) {
            self.lost.store(true, Ordering::SeqCst);
            self.cancel.cancel();
        }
        self.cancel.is_cancelled()
    }

    fn is_lost(&self) -> bool {
        self.lost.load(Ordering::SeqCst)
    }
}

pub(super) fn dispatch(
    shared: Arc<Shared>,
    events: Events,
    repaint: Repaint,
    command: ExportCommand,
) {
    let (run, request) = match command {
        ExportCommand::Cancel { run } => return shared.export.cancel(run),
        ExportCommand::Start { run, request } => (run, request),
    };
    let ctx = RunCtx {
        events,
        repaint,
        run,
    };
    match register(&shared, &ctx, request) {
        Ok((job, guard)) => spawn_run(job, guard),
        Err(error) => ctx.emit(ctx.failed(error)),
    }
}

/// The synchronous part: validates, resolves the session, takes the slot.
fn register(
    shared: &Arc<Shared>,
    ctx: &RunCtx,
    request: DocExportRequest,
) -> Result<(Job, SlotGuard)> {
    validate_path(&request.path)?;
    let not_connected = || CoreError::Unsupported("the connection is not open".into());
    let session = shared.session(request.session).ok_or_else(not_connected)?;
    // The generation is read BEFORE the store: a reconnect in between leaves a stale
    // generation (noticed at once), never a new store under an old generation.
    let generation = session.generation();
    let store = session.current_store().ok_or_else(not_connected)?;
    let cancel = CancelFlag::default();
    let lost = Arc::new(AtomicBool::new(false));
    shared.export.take(Active {
        run: ctx.run,
        cancel: cancel.clone(),
        session: request.session,
        lost: Arc::clone(&lost),
    })?;
    let guard = SlotGuard {
        shared: Arc::clone(shared),
        run: ctx.run,
    };
    let job = Job {
        ctx: ctx.clone(),
        request,
        session,
        generation,
        store,
        limit: shared.config.command_timeout,
        cancel,
        lost,
    };
    Ok((job, guard))
}

fn spawn_run(job: Job, guard: SlotGuard) {
    tokio::spawn(async move {
        let ctx = job.ctx.clone();
        // An inner task, so a panic surfaces as a `JoinError` and still ends the run with one
        // terminal event (the file guard removes the file while unwinding).
        let terminal = match tokio::spawn(run::execute(job)).await {
            Ok(event) => event,
            Err(_) => ctx.failed(CoreError::Database("internal error".into())),
        };
        guard.release();
        ctx.emit(terminal);
    });
}

/// Removes the file after a failure; the failure itself, or `ExportFileNotRemoved` when the file
/// could not be removed.
fn after_removal(guard: FileGuard, error: CoreError) -> CoreError {
    if guard.remove() {
        error
    } else {
        CoreError::ExportFileNotRemoved
    }
}
