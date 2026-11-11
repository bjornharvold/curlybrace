//! Async bridge between the (synchronous) UI and the document store.
//!
//! The UI sends [`Command`]s and drains [`Event`]s. Every command yields
//! EXACTLY ONE event, followed by one `repaint()` call, on every code path:
//! success, error, timeout and even a panic inside a handler. There are three exceptions.
//! `Command::Copy`: a run answers with a stream of events (see the `copy` module). And
//! `Command::Compare`: compare sessions are long-lived and independent of the connections,
//! so `Cancel`, `Discard` and `Close` have no answer of their own and `Start` answers with a
//! stream of events (one `PairDone` per pair, then one `RunDone`). The answer rule per compare
//! command is documented in the `compare` module. And `Event::Sampler`: the index usage sampler
//! of a connected session reports its status (running, or paused and why) on its own, whenever
//! it changes, with no command behind it. The session-less `SetSamplingExcluded` and
//! `PurgeHistory` have no answer, like `LockReadOnly`, and neither has `CopyShellHistory`.
//!
//! Every connection is its own session (`Connect(cfg)` opens session `cfg.id`); data commands and
//! session events carry the session they belong to, and all epochs come from one counter.
//!
//! The spawn functions must be called from within a Tokio runtime (they use
//! [`tokio::runtime::Handle::current`]); the binary creates and owns that
//! runtime. Commands run concurrently, so a UI that needs ordering (e.g.
//! `Connect` before `Find`) must wait for the corresponding event first.

#[cfg(test)]
mod admin_tests;
mod bulk;
#[cfg(test)]
mod bulk_cancel_tests;
#[cfg(test)]
mod bulk_read_tests;
#[cfg(test)]
mod bulk_support;
mod bulk_undo;
#[cfg(test)]
mod bulk_undo_tests;
#[cfg(test)]
mod bulk_write_tests;
mod command;
mod compare;
mod config;
mod copy;
#[cfg(test)]
mod copy_failure_tests;
#[cfg(test)]
mod copy_support;
#[cfg(test)]
mod copy_tests;
mod dispatch;
#[cfg(test)]
mod epoch_tests;
#[cfg(test)]
mod explain_tests;
mod handlers;
#[cfg(test)]
mod index_build_timeout_tests;
#[cfg(test)]
mod index_facts_tests;
#[cfg(test)]
mod index_history_tests;
#[cfg(test)]
mod index_request_failed_tests;
#[cfg(test)]
mod index_tests;
mod indexes;
mod job_flag;
#[cfg(test)]
mod lost_server_tests;
mod profiler;
mod profiler_fold;
mod profiler_handlers;
#[cfg(test)]
mod profiler_poll_tests;
mod profiler_start;
mod profiler_status;
#[cfg(test)]
mod profiler_tests;
#[cfg(test)]
mod prompt_gate_tests;
mod read_only_registry;
#[cfg(test)]
mod read_only_tests;
mod sampler;
#[cfg(test)]
mod sampler_tests;
mod session;
#[cfg(test)]
mod session_tests;
mod shell_handlers;
#[cfg(test)]
mod shell_history_tests;
mod shell_slot;
#[cfg(test)]
mod shell_slot_tests;
#[cfg(test)]
mod shell_tests;
mod snapshots;
#[cfg(test)]
mod test_connection_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tunnel_tests;
mod tunneling;
mod typed_record;
#[cfg(test)]
mod typed_record_tests;

use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use bson::{Bson, Document};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::watch;

use crate::connection::ConnectionConfig;
use crate::error::{CoreError, Result};
use crate::explain::Verdict;
use crate::history::{ChartRange, HistoryDb, ShapeSummary};
use crate::indexes::IndexSpec;
use crate::secrets::SecretStore;
use crate::store::{
    CollectionIndex, DocumentStore, Namespace, Page, ProfilingState, Query, Topology,
};
use session::Session;

pub use config::{
    BULK_COUNT_MAX_TIME, BULK_WRITE_TIMEOUT, COMMAND_TIMEOUT, CONNECT_TIMEOUT, DUPLICATE_TIMEOUT,
    INDEX_BUILD_TIMEOUT, PING_TIMEOUT, RuntimeConfig, default_history_path, default_snapshot_dir,
};
pub use handlers::COMMAND_TIMEOUT_MESSAGE;
pub use indexes::IndexSeries;
pub use profiler_status::ProfilerStatus;
pub use sampler::{MIN_INTERVAL, SamplerConfig, SamplerPause, SamplerStatus};
pub use session::{EMBEDDED_SESSION, SessionId};
/// What `RuntimeHandle::rx.try_recv()` reports, so callers can tell "no event yet" from "the
/// runtime is gone" without depending on tokio themselves.
pub use tokio::sync::mpsc::error::TryRecvError;
pub use tunneling::{
    SshTunnelOpener, TunnelHandle, TunnelOpener, TunnelRequest, default_known_hosts_path,
};

/// Opens a store for a connection config. Injectable so tests need no server.
#[async_trait::async_trait]
pub trait Connector: Send + Sync {
    async fn connect(
        &self,
        cfg: &ConnectionConfig,
        password: Option<&str>,
        tunnel: Option<SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>>;
}

/// Default connector: the real driver.
pub struct MongoConnector;

#[async_trait::async_trait]
impl Connector for MongoConnector {
    async fn connect(
        &self,
        cfg: &ConnectionConfig,
        password: Option<&str>,
        tunnel: Option<SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        let client = crate::client::connect(cfg, password, tunnel).await?;
        Ok(Arc::new(crate::mongo_store::MongoStore::new(client)))
    }
}

/// Which password or SSH secret a connection test uses. Debug redacts through
/// [`SecretString`](crate::secrets::SecretString).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum PasswordChoice {
    /// Nothing typed: use the password stored for the connection, if any.
    #[default]
    Stored,
    /// The user chose to clear the saved password: connect without one, ignoring the stored one.
    Without,
    /// A password typed into the form for this test only.
    Typed(crate::secrets::SecretString),
}

/// Secrets typed into the connection form for one test. `Stored` on either choice means "use the
/// stored one".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TestSecrets {
    pub password: PasswordChoice,
    pub ssh_secret: PasswordChoice,
}

/// What a successful connection test learned.
#[derive(Clone, Debug, PartialEq)]
pub struct TestReport {
    pub server_version: Option<String>,
    pub tls: bool,
    pub authenticated: bool,
}

// Commands are short-lived, moved once into a channel; boxing would ripple through every call site.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum Command {
    /// Opens the session `cfg.id`, replacing that session's earlier connection.
    Connect(ConnectionConfig),
    Disconnect {
        session: SessionId,
    },
    ListDatabases {
        session: SessionId,
    },
    ListCollections {
        session: SessionId,
        db: String,
    },
    Find {
        session: SessionId,
        ns: Namespace,
        query: Query,
    },
    /// Explains how the server runs `query` and judges it. A read, allowed on a read-only
    /// connection. Answered by [`Event::Explain`], whose `outcome` also carries any failure.
    ExplainFind {
        session: SessionId,
        ns: Namespace,
        query: Query,
    },
    Insert {
        session: SessionId,
        ns: Namespace,
        json: String,
    },
    Replace {
        session: SessionId,
        ns: Namespace,
        id: Bson,
        json: String,
    },
    Delete {
        session: SessionId,
        ns: Namespace,
        id: Bson,
    },
    Clone {
        session: SessionId,
        ns: Namespace,
        source: Document,
    },
    /// A data-compare session command; see [`compare`] for how it is answered.
    Compare(crate::compare::protocol::CompareCommand),
    /// A copy between two open sessions; see [`copy`] for how it is answered. Like Compare it
    /// streams its own events and is not a per-session write for the UI's write guard.
    Copy(crate::copy::CopyCommand),
    /// Opens a throw-away connection and reports on it with [`Event::TestConnection`]. It never
    /// begins an epoch and never touches the installed store or tunnel.
    TestConnection {
        request_id: u64,
        cfg: ConnectionConfig,
        secrets: TestSecrets,
    },
    /// Makes every open store of the connection refuse writes from now on. Independent of the
    /// sessions like `TestConnection`: no epoch, no answer event, and an unknown id does nothing.
    LockReadOnly {
        connection: uuid::Uuid,
    },
    /// Lists the indexes of a collection; answered with [`Event::Indexes`], which echoes
    /// `request`. The caller numbers its requests so an older answer that arrives after a newer
    /// one can be told apart; the runtime never reads it.
    ListIndexes {
        session: SessionId,
        ns: Namespace,
        request: u64,
    },
    /// Reads the recorded usage of the collection's indexes; answered with
    /// [`Event::IndexHistory`] (no series when history is off).
    IndexHistory {
        session: SessionId,
        ns: Namespace,
        range: ChartRange,
        /// Echoed by the answer, like [`Command::ListIndexes`]'s.
        request: u64,
    },
    /// Reads the profiler level of `db`; answered with [`Event::ProfilingState`], whose `outcome`
    /// also carries any failure. A read: it starts no background task.
    ProfilingState {
        session: SessionId,
        db: String,
    },
    /// Sets the profiler level of `db` (the only runtime path that does), a write refused by a
    /// read-only store. Answered with [`Event::ProfilingState`] like [`Command::ProfilingState`],
    /// a refusal included. Starts no background task.
    SetProfilingLevel {
        session: SessionId,
        db: String,
        level: i32,
    },
    /// Reads the recorded query shapes of the collection; answered with [`Event::QueryShapes`]
    /// (none when history is off), a failure with [`Event::RequestFailed`].
    QueryShapes {
        session: SessionId,
        ns: Namespace,
        range: ChartRange,
        /// Echoed by the answer, like [`Command::ListIndexes`]'s.
        request: u64,
    },
    /// Asks for the collection's estimated document count; answered with
    /// [`Event::EstimatedCount`].
    EstimatedCount {
        session: SessionId,
        ns: Namespace,
    },
    /// The four index writes below are answered with [`Event::IndexChanged`].
    CreateIndex {
        session: SessionId,
        ns: Namespace,
        spec: IndexSpec,
    },
    DropIndex {
        session: SessionId,
        ns: Namespace,
        name: String,
    },
    SetIndexHidden {
        session: SessionId,
        ns: Namespace,
        name: String,
        hidden: bool,
    },
    SetIndexTtl {
        session: SessionId,
        ns: Namespace,
        name: String,
        seconds: i64,
    },
    /// The five administration writes below are answered with [`Event::AdminChanged`]; the UI
    /// re-lists the databases and collections. A drop of something already gone succeeds.
    DropCollection {
        session: SessionId,
        ns: Namespace,
    },
    DropDatabase {
        session: SessionId,
        db: String,
    },
    RenameCollection {
        session: SessionId,
        ns: Namespace,
        new_name: String,
        /// Replace an existing target; without it an existing target fails the rename.
        drop_target: bool,
    },
    /// Creates a read-only view; `pipeline_json` is a JSON array of stages (extended JSON).
    CreateView {
        session: SessionId,
        ns: Namespace,
        source: String,
        pipeline_json: String,
    },
    DuplicateCollection {
        session: SessionId,
        ns: Namespace,
        target: String,
    },
    /// Counts and sizes of a collection; answered with [`Event::CollectionStats`], which echoes
    /// `request` like [`Command::ListIndexes`]'s.
    CollectionStats {
        session: SessionId,
        ns: Namespace,
        request: u64,
    },
    /// Counts and sizes of a database; answered with [`Event::DatabaseStats`].
    DatabaseStats {
        session: SessionId,
        db: String,
        request: u64,
    },
    /// Sets the databases every running (and later started) sampler skips, on top of the system
    /// databases. No session, no epoch, no answer event; samplers see it at their next pass.
    SetSamplingExcluded {
        databases: BTreeSet<String>,
    },
    /// Deletes the stored usage history of a connection (for example one that was deleted). No
    /// session, no epoch, no answer event; a failure is logged.
    PurgeHistory {
        connection: uuid::Uuid,
    },
    /// Counts the documents matching `filter` (bounded by `BULK_COUNT_MAX_TIME`); a read answered
    /// with [`Event::BulkCount`], which echoes `request`. A count that outruns the client-side
    /// bound arrives as `Event::RequestFailed { context: "bulk_count", request }`; the UI must
    /// treat that as "count unavailable".
    BulkCount {
        session: SessionId,
        ns: Namespace,
        filter: Document,
        request: u64,
    },
    /// Reads the first `BULK_PREVIEW_DOCS` documents matching `filter` ALONE (no sort, skip or
    /// projection) and records no query history. A read, allowed on a read-only connection;
    /// answered with [`Event::BulkSample`], which echoes `request`.
    BulkSample {
        session: SessionId,
        ns: Namespace,
        filter: Document,
        request: u64,
    },
    /// Previews an update's exact result; classified as a write because the preview may run in a
    /// transaction. Answered with [`Event::BulkPreview`], failures included.
    BulkPreview {
        session: SessionId,
        ns: Namespace,
        request: crate::bulk::BulkUpdateRequest,
        request_id: u64,
    },
    /// Applies an update to every matching document; answered with [`Event::BulkFinished`].
    BulkUpdate {
        session: SessionId,
        ns: Namespace,
        request: crate::bulk::BulkUpdateRequest,
        run: crate::bulk::BulkRunId,
        undo: crate::bulk::UndoPolicy,
    },
    /// Deletes every matching document; answered with [`Event::BulkFinished`].
    BulkDelete {
        session: SessionId,
        ns: Namespace,
        request: crate::bulk::BulkDeleteRequest,
        run: crate::bulk::BulkRunId,
        undo: crate::bulk::UndoPolicy,
    },
    /// What undoing the run `run` would touch: its size and first documents beside their current
    /// state. Classified as a write (undo is one); answered with [`Event::BulkUndoPreview`].
    BulkUndoPreview {
        session: SessionId,
        run: crate::bulk::BulkRunId,
        request_id: u64,
    },
    /// Restores the documents of the run `run` from its snapshot; answered with
    /// [`Event::BulkUndoFinished`]. The snapshot is kept so an undo can be retried.
    BulkUndo {
        session: SessionId,
        run: crate::bulk::BulkRunId,
    },
    /// Lists the saved snapshots; no session. Answered with [`Event::Snapshots`].
    SnapshotList {
        request: u64,
    },
    /// Deletes a run's snapshot files; no session. Answered with [`Event::Snapshots`] (the
    /// listing after the deletion).
    SnapshotDelete {
        run: crate::bulk::BulkRunId,
        request: u64,
    },
    /// Best-effort cancel of a running bulk write, answered with [`Event::BulkCancelResult`]. Not
    /// a write. Every command runs in its own task, so it is handled while the write is in flight.
    BulkCancel {
        session: SessionId,
        run: crate::bulk::BulkRunId,
    },
    /// Runs `script` in `mongosh` (at `exe`, which the UI resolved from the tool settings) against
    /// the session's connection. A write: refused on a read-only connection. Streams
    /// [`Event::ShellOutput`] and ends with [`Event::ShellFinished`], or is answered by
    /// [`Event::ShellRefused`] when nothing was started. `run_id` is the caller's number for it.
    ShellRun {
        session: SessionId,
        run_id: u64,
        exe: std::path::PathBuf,
        script: String,
    },
    /// Stops the run `run_id` of the session; any other id is ignored. The run still ends with
    /// [`Event::ShellFinished`] (`Cancelled`).
    ShellCancel {
        session: SessionId,
        run_id: u64,
    },
    /// Reads the remembered shell runs of the session's connection, answered with
    /// [`Event::ShellHistory`] (an empty list when history is off). Needs no live connection:
    /// the history is keyed by the connection id, which is the session id.
    ShellHistory {
        session: SessionId,
    },
    /// Forgets the shell runs of the session's connection, answered with [`Event::ShellHistory`]
    /// (the now empty list).
    ClearShellHistory {
        session: SessionId,
    },
    /// Gives the connection `to` a copy of the shell runs of `from` (a duplicated connection).
    /// No session, no epoch, no answer event; a failure is logged. UNORDERED with a following
    /// `ShellHistory` or `ClearShellHistory` (each runs in its own task), so a reader must not
    /// assume the copy is visible yet.
    CopyShellHistory {
        from: uuid::Uuid,
        to: uuid::Uuid,
    },
}

/// Why a [`Command::ShellRun`] was not started.
#[derive(Clone, Debug, PartialEq)]
pub enum ShellRefusal {
    ReadOnly,
    NotConnected,
    AlreadyRunning,
    Invalid(crate::shell::ShellError),
}

// Events are moved through a channel once; boxing `Query` would only add noise
// for the UI, which pattern-matches on the brief's public shape.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The connection requested by the `Connect` whose epoch is `epoch` is installed.
    /// Epochs come from one counter shared by all sessions and grow with every Connect/Disconnect
    /// request, so a receiver can ignore an outcome older than the latest one it requested. Only a
    /// still-current connect reports `Connected` or `ConnectFailed`; a replaced one reports
    /// `Failed { error: Superseded }`.
    Connected {
        session: SessionId,
        epoch: u64,
        /// How the server is deployed; `None` when it could not be read (never fails a connect).
        topology: Option<Topology>,
    },
    /// The `Disconnect` with this `epoch` took effect.
    Disconnected { session: SessionId, epoch: u64 },
    /// The still-current connect with this `epoch` failed for real. A replaced connect never
    /// reports this: it answers `Failed { context: "connect", error: Superseded }`.
    ConnectFailed {
        session: SessionId,
        epoch: u64,
        error: CoreError,
    },
    Databases {
        session: SessionId,
        names: Vec<String>,
    },
    Collections {
        session: SessionId,
        db: String,
        names: Vec<String>,
    },
    /// Carries the originating `ns`/`query` so the UI can drop stale results.
    Results {
        session: SessionId,
        ns: Namespace,
        query: Query,
        page: Page,
    },
    /// The UI re-runs its current query.
    Mutated { session: SessionId, ns: Namespace },
    /// `context` is an i18n key suffix, e.g. `"connect"` or `"find"`.
    Failed {
        session: SessionId,
        context: &'static str,
        error: CoreError,
    },
    /// A numbered read (`ListIndexes`, `IndexHistory`) failed. Like [`Event::Failed`], plus the
    /// `request` the command carried, so a failure of an older request can be told from the one
    /// of the request still awaited.
    RequestFailed {
        session: SessionId,
        context: &'static str,
        request: u64,
        error: CoreError,
    },
    /// An answer or progress report of a compare session (see [`compare`]).
    Compare(crate::compare::protocol::CompareEvent),
    /// An event of a copy run (see [`copy`]).
    Copy(crate::copy::CopyEvent),
    /// The answer to `Command::TestConnection { request_id, .. }`; carries no secret.
    TestConnection {
        request_id: u64,
        outcome: std::result::Result<TestReport, CoreError>,
    },
    /// The answer to `ListIndexes`.
    Indexes {
        session: SessionId,
        ns: Namespace,
        indexes: Vec<CollectionIndex>,
        /// The `request` of the command this answers.
        request: u64,
    },
    /// The answer to `IndexHistory`: one series per index definition and node, empty when no
    /// usage was recorded or history is off.
    IndexHistory {
        session: SessionId,
        ns: Namespace,
        range: ChartRange,
        series: Vec<IndexSeries>,
        /// The `request` of the command this answers.
        request: u64,
    },
    /// The answer to `ExplainFind`. A failed explain or an unreadable plan is an `Err` outcome
    /// shown inline by the query bar; it is never an [`Event::Failed`], which raises a banner.
    Explain {
        session: SessionId,
        ns: Namespace,
        query: Query,
        outcome: std::result::Result<Verdict, CoreError>,
    },
    /// The answer to `ProfilingState` and `SetProfilingLevel`. A failure, a refusal or a timeout
    /// is an `Err` outcome shown inline; it is never an [`Event::Failed`], which raises a banner
    /// (a superseded command is the one exception and is dropped, like any superseded answer).
    ///
    /// An `Err` outcome of a `SetProfilingLevel`, notably a timeout or a lost connection, is
    /// AMBIGUOUS: the server may have applied the change. The UI must re-read the state with
    /// [`Command::ProfilingState`].
    ProfilingState {
        session: SessionId,
        db: String,
        outcome: std::result::Result<ProfilingState, CoreError>,
    },
    /// The answer to `QueryShapes`: the shapes recorded for the collection, empty when none were
    /// or history is off.
    QueryShapes {
        session: SessionId,
        ns: Namespace,
        range: ChartRange,
        shapes: Vec<ShapeSummary>,
        /// The `request` of the command this answers.
        request: u64,
    },
    /// The answer to `EstimatedCount`.
    EstimatedCount {
        session: SessionId,
        ns: Namespace,
        count: u64,
    },
    /// The answer to a successful index write; the UI re-lists the indexes.
    IndexChanged { session: SessionId, ns: Namespace },
    /// The answer to an administration write: `db` is the database whose collections (or whose
    /// existence) changed.
    AdminChanged { session: SessionId, db: String },
    /// The answer to `CollectionStats`.
    CollectionStats {
        session: SessionId,
        ns: Namespace,
        stats: crate::admin::CollectionStats,
        /// The `request` of the command this answers.
        request: u64,
    },
    /// The answer to `DatabaseStats`.
    DatabaseStats {
        session: SessionId,
        db: String,
        stats: crate::admin::DatabaseStats,
        /// The `request` of the command this answers.
        request: u64,
    },
    /// The answer to `BulkCount`; `count` is `None` when the server-side count timed out. A
    /// client-side timeout is an `Event::RequestFailed { context: "bulk_count", request }` instead
    /// (treat both as "count unavailable").
    BulkCount {
        session: SessionId,
        ns: Namespace,
        request: u64,
        count: Option<u64>,
    },
    /// The answer to `BulkSample`.
    BulkSample {
        session: SessionId,
        ns: Namespace,
        request: u64,
        docs: Vec<Document>,
    },
    /// The answer to `BulkPreview`. A failure is an `Err` outcome shown inline, never an
    /// [`Event::Failed`] and never a write-unknown error.
    BulkPreview {
        session: SessionId,
        ns: Namespace,
        request_id: u64,
        outcome: std::result::Result<crate::bulk::UpdatePreview, CoreError>,
    },
    /// The answer to `BulkUpdate` and `BulkDelete`. A timeout or a lost connection is
    /// `Ok(BulkOutcome::Unknown { cancelled: false })`; an `Err` means the write never ran.
    BulkFinished {
        session: SessionId,
        ns: Namespace,
        run: crate::bulk::BulkRunId,
        outcome: std::result::Result<crate::bulk::BulkOutcome, CoreError>,
        /// Whether the run left a snapshot the person can undo it with.
        undo: crate::bulk::UndoStatus,
    },
    /// The answer to `BulkUndoPreview`. A failure is an `Err` outcome shown inline.
    BulkUndoPreview {
        session: SessionId,
        run: crate::bulk::BulkRunId,
        request_id: u64,
        outcome: std::result::Result<crate::bulk::UndoPreview, CoreError>,
    },
    /// The answer to `BulkUndo`. Like `BulkFinished` it outlives a replaced session, because it
    /// may report documents the server changed. A lost connection is
    /// `Err(WriteOutcomeUnknown)`: some documents may already be restored.
    BulkUndoFinished {
        session: SessionId,
        run: crate::bulk::BulkRunId,
        outcome: std::result::Result<crate::snapshot::UndoReport, CoreError>,
    },
    /// The answer to `SnapshotList` and `SnapshotDelete`; `request` echoes the command's.
    Snapshots {
        request: u64,
        snapshots: Vec<crate::snapshot::SnapshotInfo>,
    },
    /// The answer to `BulkCancel`.
    BulkCancelResult {
        session: SessionId,
        run: crate::bulk::BulkRunId,
        outcome: std::result::Result<crate::bulk::CancelEffect, CoreError>,
    },
    /// SPONTANEOUS (no command behind it): the session's sampler started, resumed or paused. See
    /// the module docs for how this differs from "exactly one event per command".
    Sampler {
        session: SessionId,
        status: SamplerStatus,
    },
    /// SPONTANEOUS, like `Sampler`: the profile poller started, resumed, paused or lost entries.
    Profiler {
        session: SessionId,
        status: ProfilerStatus,
    },
    /// Text a shell run printed, already scrubbed of the password. Not epoch-gated: a run is
    /// tagged with its `session` and `run_id` and the UI ignores the ones it no longer shows.
    ShellOutput {
        session: SessionId,
        run_id: u64,
        stream: crate::shell::Stream,
        text: String,
    },
    /// The run `run_id` ended; follows its last `ShellOutput`. Also sent for a run killed by a
    /// disconnect (`Cancelled`) and for one that could not start (`StartFailed`).
    ShellFinished {
        session: SessionId,
        run_id: u64,
        outcome: crate::shell::Outcome,
    },
    /// The `ShellRun` `run_id` was refused. Nothing was spawned in any refusal. It can arrive
    /// late: after the password lookup (a disconnect or reconnect landed meanwhile, or the
    /// tunnel was gone), not only straight after the command.
    ShellRefused {
        session: SessionId,
        run_id: u64,
        reason: ShellRefusal,
    },
    /// The answer to `ShellHistory` and `ClearShellHistory`: the connection's remembered runs,
    /// newest first. Not epoch-gated. Its `Debug` leaves the scripts out.
    ShellHistory {
        session: SessionId,
        entries: Vec<crate::history::ShellHistoryEntry>,
    },
}

/// A config whose session is [`EMBEDDED_SESSION`], so a test that connects and then sends data
/// commands keeps talking to the one session it always used.
#[cfg(test)]
fn embedded_config(name: &str, uri: &str) -> ConnectionConfig {
    ConnectionConfig {
        id: EMBEDDED_SESSION,
        ..ConnectionConfig::new(name, uri)
    }
}

pub struct RuntimeHandle {
    pub tx: UnboundedSender<Command>,
    pub rx: UnboundedReceiver<Event>,
}

pub type Repaint = Arc<dyn Fn() + Send + Sync>;

/// State shared by all handler tasks.
pub(crate) struct Shared {
    /// Open connections by session id; a session is removed on `Disconnect`.
    sessions: RwLock<HashMap<SessionId, Arc<Session>>>,
    /// The global epoch counter: every Connect/Disconnect, whichever session it names, takes the
    /// next number, so epochs never repeat and a late event of a closed session can never look
    /// current to a new one.
    next_epoch: AtomicU64,
    pub secrets: Option<Arc<dyn SecretStore>>,
    pub connector: Arc<dyn Connector>,
    pub config: RuntimeConfig,
    /// Compare sessions: independent of the connections above.
    compare: compare::Registry,
    /// The one active copy run.
    copy: copy::CopySlot,
    /// The flag of the newest connection test: a newer test ends it, so at most one test's
    /// host-key question is live. Locked only briefly, never across an await.
    latest_test: std::sync::Mutex<Option<job_flag::JobFlag>>,
    /// The read-only flags of the open stores, so a lock reaches stores that are already open.
    pub(crate) read_only: read_only_registry::ReadOnlyRegistry,
    /// Bulk runs the person asked to cancel, so a run still waiting or snapshotting does not go
    /// on to write. Locked only briefly, never across an await.
    pub(crate) cancelled: std::sync::Mutex<std::collections::HashSet<uuid::Uuid>>,
    /// The usage history, opened once at spawn. `None` when `history_path` is unset or opening
    /// failed (logged); then no sampler starts and `IndexHistory` answers empty. When a path was
    /// set but opening failed, every connect reports `Paused(HistoryUnavailable)`.
    pub(crate) history: Option<Arc<HistoryDb>>,
    /// The sampler tunables shared by every sampler; each holds a receiver.
    pub(crate) sampler_config: watch::Sender<SamplerConfig>,
}

impl Shared {
    /// Gets or creates the session and starts its next epoch. Called only from the dispatcher
    /// (single-threaded), so request order decides.
    fn begin_session_epoch(&self, id: SessionId) -> (Arc<Session>, u64) {
        // A copy over this session must stop before its store is replaced.
        self.copy.session_changing(id);
        let generation = self.next_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let session = {
            let mut map = self.sessions.write().unwrap_or_else(|e| e.into_inner());
            map.entry(id)
                .or_insert_with(|| Arc::new(Session::new(generation, None)))
                .clone()
        };
        session.begin(generation);
        (session, generation)
    }

    /// Starts the next epoch for `id`, drops its store and tunnel and removes it from the map.
    /// Tasks that still hold the `Arc<Session>` see a stale generation.
    fn end_session(&self, id: SessionId) -> u64 {
        // A copy over this session must stop before its store is dropped.
        self.copy.session_changing(id);
        let generation = self.next_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let removed = self
            .sessions
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        if let Some(session) = removed {
            session.begin(generation);
        }
        generation
    }

    fn session(&self, id: SessionId) -> Option<Arc<Session>> {
        self.sessions
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .cloned()
    }
}

/// Starts a runtime with no store; `Command::Connect` installs one.
pub fn spawn_runtime(secrets: Arc<dyn SecretStore>, repaint: Repaint) -> RuntimeHandle {
    spawn_runtime_with_config(Some(secrets), None, repaint, RuntimeConfig::default())
}

/// Starts a runtime that is already connected to `store` (tests, embedding).
pub fn spawn_runtime_with_store(store: Arc<dyn DocumentStore>, repaint: Repaint) -> RuntimeHandle {
    spawn_runtime_with_config(None, Some(store), repaint, RuntimeConfig::default())
}

pub fn spawn_runtime_with_config(
    secrets: Option<Arc<dyn SecretStore>>,
    store: Option<Arc<dyn DocumentStore>>,
    repaint: Repaint,
    config: RuntimeConfig,
) -> RuntimeHandle {
    spawn_runtime_with_connector(secrets, store, Arc::new(MongoConnector), repaint, config)
}

pub fn spawn_runtime_with_connector(
    secrets: Option<Arc<dyn SecretStore>>,
    store: Option<Arc<dyn DocumentStore>>,
    connector: Arc<dyn Connector>,
    repaint: Repaint,
    config: RuntimeConfig,
) -> RuntimeHandle {
    let (tx, mut commands) = unbounded_channel::<Command>();
    let (events, rx) = unbounded_channel::<Event>();
    let history = indexes::open_history(config.history_path.as_deref());
    snapshots::spawn_expiry(config.snapshot_dir.clone());
    let (sampler_config, _) = watch::channel(
        SamplerConfig {
            command_timeout: config.command_timeout,
            ..config.sampler.clone()
        }
        .clamped(),
    );
    // A store handed in at spawn time is the embedded session, in generation 0.
    let sessions: HashMap<SessionId, Arc<Session>> = store
        .map(|store| (EMBEDDED_SESSION, Arc::new(Session::new(0, Some(store)))))
        .into_iter()
        .collect();
    let shared = Arc::new(Shared {
        sessions: RwLock::new(sessions),
        next_epoch: AtomicU64::new(0),
        secrets,
        connector,
        config,
        compare: Default::default(),
        copy: Default::default(),
        latest_test: Default::default(),
        read_only: Default::default(),
        cancelled: Default::default(),
        history,
        sampler_config,
    });
    tokio::runtime::Handle::current().spawn(async move {
        while let Some(command) = commands.recv().await {
            dispatch::dispatch(shared.clone(), events.clone(), repaint.clone(), command);
        }
    });
    RuntimeHandle { tx, rx }
}
