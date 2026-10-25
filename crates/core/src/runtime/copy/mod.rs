//! Copy runs over two OPEN sessions. Unlike a compare run, a copy opens no connection of its own:
//! it reads and writes the stores of the sessions named in the request (the connection ids), so
//! the read-only wrapper of a connection applies without any copy-specific code. Copy commands
//! carry no session of their own and have no per-command answer; a run answers with a stream of
//! [`CopyEvent`]s, like a compare run does.
//!
//! Like `compare::apply::dispatch`, `dispatch` registers a run synchronously (names valid, both
//! sessions open and connected, target writable for a `Start`, the one run slot free), so command
//! ORDER decides outcomes: a second `Start` or `Preview` behind a running one is answered with
//! `Failed` for ITS run id and never disturbs the running one. Listing a database, reading and
//! writing happen in a task.
//!
//! Event guarantees, per run:
//! - A refusal (invalid or excluded name, session not open, read-only target, slot taken) is a
//!   single `Failed`, sent before any read or write and with no `Planned` before it.
//! - Otherwise `Planned` is first, then per collection (in order, one at a time) `PreviewProgress`*
//!   and one `PreviewItemDone`, or `Progress`* and one `ItemDone` carrying the exact counts, then
//!   exactly one terminal event, always last: `PreviewDone`, `Done`, or `Failed`.
//! - `Cancel { run }` ends the run with `Done { cancelled: true }` after the collection in progress
//!   reported its exact counts in `ItemDone`; collections not yet started get no event. A cancel
//!   of an unknown (or finished) run is ignored.
//! - The first collection that ends in an error stops the run: its `ItemDone` (exact counts, with
//!   the error as its outcome) is followed by `Failed { error }`. A preview whose counts are
//!   unavailable because of the compare result cap is NOT an error (`counts: None`).
//! - A session that is closed or reconnected mid-run (its epoch moved) is noticed before each
//!   collection and after each written batch, and the dispatcher cancels the run the moment a
//!   `Connect` or `Disconnect` of either of its sessions is requested (`CopySlot::session_changing`),
//!   which abandons a read in flight. The collection in progress reports its exact counts and the
//!   run ends with `Failed { error: Superseded }`. No write is STARTED after the epoch change is
//!   observed; only a `bulk_write` already in flight may complete (its documents are counted).
//!   An epoch change of an unrelated session does not touch the run. A collection that had already written its last batch when the epoch
//!   moved still reports `Completed`; the run then fails before the next collection, or is `Done`
//!   when it was the last.
//! - Caveats: a cancelled preview emits no `PreviewItemDone` for the item in progress, and a panic
//!   in the middle of an item ends the run with `Failed` (internal error) without an `ItemDone`
//!   for that item.
//! - The slot is released BEFORE the terminal event is sent, so a client that reacts to it with
//!   a new run is never refused.
//!
//! Keys and documents are data and are never logged or put in an error.

mod preview;
mod run;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc::UnboundedSender;

use super::session::Session;
use super::{Event, Repaint, SessionId, Shared};
use crate::compare::apply::APPLY_BATCH;
use crate::compare::engine::CancelFlag;
use crate::compare::protocol::RunId;
use crate::copy::names::{
    is_excluded_collection, is_excluded_database, validate_database_name, validate_namespace,
};
use crate::copy::{
    ConflictPolicy, CopyCommand, CopyEvent, CopyItem, CopyJob, CopyRequest, CopyScope,
};
use crate::error::{CoreError, Result};
use crate::store::{CollectionKind, DocumentStore, Namespace};

type Events = UnboundedSender<Event>;

/// The one active run of the runtime, if any.
#[derive(Default)]
pub(super) struct CopySlot {
    active: Mutex<Option<Active>>,
}

/// What the dispatcher needs to know about the running copy.
struct Active {
    run: RunId,
    cancel: CancelFlag,
    /// The source and target sessions of the run.
    sessions: [SessionId; 2],
    /// The run's `Job::lost`: set when one of its sessions changed epoch.
    lost: Arc<AtomicBool>,
}

impl CopySlot {
    fn take(&self, active: Active) -> Result<()> {
        let mut slot = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return Err(CoreError::Unsupported("a copy is already running".into()));
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
    /// moves its epoch. A run over that session is marked lost and cancelled at once, so even a
    /// read still in flight is abandoned and no further batch is started. Other runs are not
    /// touched.
    pub(super) fn session_changing(&self, session: SessionId) {
        let active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = active.as_ref().filter(|a| a.sessions.contains(&session)) {
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
            .copy
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Preview,
    Copy,
}

/// Sends the events of one run.
#[derive(Clone)]
struct RunCtx {
    events: Events,
    repaint: Repaint,
    run: RunId,
}

impl RunCtx {
    fn emit(&self, event: CopyEvent) {
        if self.events.send(Event::Copy(event)).is_ok() {
            (self.repaint)();
        }
    }

    fn failed(&self, error: CoreError) -> CopyEvent {
        CopyEvent::Failed {
            run: self.run,
            error,
        }
    }
}

/// One side of a run: the session, the epoch it was registered in and its store.
struct Side {
    session: Arc<Session>,
    generation: u64,
    store: Arc<dyn DocumentStore>,
}

impl Side {
    fn open(shared: &Shared, id: SessionId) -> Result<Self> {
        let not_connected = || CoreError::Unsupported("not connected".into());
        let session = shared.session(id).ok_or_else(not_connected)?;
        // The generation is read BEFORE the store: a reconnect in between leaves a stale
        // generation (noticed at once), never a new store under an old generation.
        let generation = session.generation();
        let store = session.current_store().ok_or_else(not_connected)?;
        Ok(Self {
            session,
            generation,
            store,
        })
    }

    fn is_current(&self) -> bool {
        self.session.is_current(self.generation)
    }
}

/// Everything a run task needs.
struct Job {
    ctx: RunCtx,
    mode: Mode,
    source: Side,
    target: Side,
    scope: CopyScope,
    policy: ConflictPolicy,
    cancel: CancelFlag,
    /// Set when a session was found stale, or the dispatcher saw its epoch change (see
    /// `CopySlot::session_changing`); the run then ends with `Superseded`.
    lost: Arc<AtomicBool>,
}

impl Job {
    /// True while both sessions are still in the epoch the run started in. When one is not, records
    /// it and cancels the engine, which stops before its next batch (so before any further write).
    fn sessions_current(&self) -> bool {
        let current = self.source.is_current() && self.target.is_current();
        if !current {
            self.lost.store(true, Ordering::SeqCst);
            self.cancel.cancel();
        }
        current
    }

    fn is_lost(&self) -> bool {
        self.lost.load(Ordering::SeqCst)
    }

    fn copy_job(&self, item: &CopyItem) -> CopyJob {
        CopyJob {
            source: Arc::clone(&self.source.store),
            source_ns: item.source.clone(),
            filter: item.filter.clone(),
            target: Arc::clone(&self.target.store),
            target_ns: item.target.clone(),
            policy: self.policy,
            batch: APPLY_BATCH,
        }
    }
}

pub(super) fn dispatch(
    shared: Arc<Shared>,
    events: Events,
    repaint: Repaint,
    command: CopyCommand,
) {
    let (run, request, mode) = match command {
        CopyCommand::Cancel { run } => return shared.copy.cancel(run),
        CopyCommand::Preview { run, request } => (run, request, Mode::Preview),
        CopyCommand::Start { run, request } => (run, request, Mode::Copy),
    };
    let ctx = RunCtx {
        events,
        repaint,
        run,
    };
    match register(&shared, &ctx, request, mode) {
        Ok((job, guard)) => spawn_run(job, guard),
        Err(error) => ctx.emit(ctx.failed(error)),
    }
}

/// The synchronous part: validates, resolves the sessions, takes the slot.
fn register(
    shared: &Arc<Shared>,
    ctx: &RunCtx,
    request: CopyRequest,
    mode: Mode,
) -> Result<(Job, SlotGuard)> {
    refuse_bad_names(&request.scope)?;
    let source = Side::open(shared, request.source)?;
    let target = Side::open(shared, request.target)?;
    if mode == Mode::Copy && target.store.is_read_only() {
        return Err(CoreError::ReadOnly);
    }
    let cancel = CancelFlag::default();
    let lost = Arc::new(AtomicBool::new(false));
    shared.copy.take(Active {
        run: ctx.run,
        cancel: cancel.clone(),
        sessions: [request.source, request.target],
        lost: Arc::clone(&lost),
    })?;
    let guard = SlotGuard {
        shared: Arc::clone(shared),
        run: ctx.run,
    };
    let job = Job {
        ctx: ctx.clone(),
        mode,
        source,
        target,
        scope: request.scope,
        policy: request.policy,
        cancel,
        lost,
    };
    Ok((job, guard))
}

fn unsupported(text: &str) -> CoreError {
    CoreError::Unsupported(text.into())
}

/// Server-owned databases and collections are never copied, from or into, and every target name
/// must be one MongoDB accepts. Judged before anything is read or written. The refusals carry
/// fixed text: a name is data and is never echoed.
fn refuse_bad_names(scope: &CopyScope) -> Result<()> {
    match scope {
        CopyScope::Database {
            source_db,
            target_db,
        } => {
            if is_excluded_database(source_db) || is_excluded_database(target_db) {
                return Err(unsupported("server-owned databases cannot be copied"));
            }
            validate_database_name(target_db).map_err(|_| unsupported("invalid target name"))
        }
        CopyScope::Collections(items) => {
            if items.is_empty() {
                return Err(unsupported("nothing to copy"));
            }
            items.iter().try_for_each(refuse_bad_item)
        }
    }
}

fn refuse_bad_item(item: &CopyItem) -> Result<()> {
    let excluded = is_excluded_database(&item.source.db)
        || is_excluded_database(&item.target.db)
        || is_excluded_collection(&item.source.collection);
    if excluded {
        return Err(unsupported("server-owned databases cannot be copied"));
    }
    validate_namespace(&item.target).map_err(|_| unsupported("invalid target name"))
}

fn spawn_run(job: Job, guard: SlotGuard) {
    tokio::spawn(async move {
        let ctx = job.ctx.clone();
        // An inner task, so a panic surfaces as a `JoinError` and still ends the run with one
        // terminal event.
        let terminal = match tokio::spawn(execute(job)).await {
            Ok(event) => event,
            Err(_) => ctx.failed(CoreError::Database("internal error".into())),
        };
        guard.release();
        ctx.emit(terminal);
    });
}

/// Plans and runs; returns the terminal event, which the caller sends once the slot is free.
async fn execute(job: Job) -> CopyEvent {
    let items = match plan(&job).await {
        Ok(items) => items,
        Err(error) => return job.ctx.failed(error),
    };
    job.ctx.emit(CopyEvent::Planned {
        run: job.ctx.run,
        items: items.clone(),
    });
    match job.mode {
        Mode::Preview => preview::run(&job, &items).await,
        Mode::Copy => run::run(&job, &items).await,
    }
}

/// The collections of the run. A database scope lists the source database now, so the plan is
/// what is there when the run starts.
async fn plan(job: &Job) -> Result<Vec<CopyItem>> {
    let (source_db, target_db) = match &job.scope {
        CopyScope::Collections(items) => return Ok(items.clone()),
        CopyScope::Database {
            source_db,
            target_db,
        } => (source_db, target_db),
    };
    let kinds = job.source.store.list_collection_kinds(source_db).await?;
    Ok(kinds
        .into_iter()
        .filter(|(name, kind)| *kind == CollectionKind::Collection && !is_excluded_collection(name))
        .map(|(name, _)| CopyItem {
            source: Namespace::new(source_db.as_str(), name.as_str()),
            target: Namespace::new(target_db.as_str(), name.as_str()),
            filter: bson::Document::new(),
        })
        .collect())
}
