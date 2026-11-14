//! Command execution. Every function maps its outcome to a single [`Event`].

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::connection::ConnectionConfig;
use crate::crud::DocumentService;
use crate::error::{CoreError, Result};
use crate::explain::classify;
use crate::history::{HistoryDb, max_continuous_secs};
use crate::read_only_store::ReadOnlyStore;
use crate::secrets::{SecretKind, secret_key};
use crate::store::DocumentStore;

use super::job_flag::JobFlag;
use super::profiler_handlers::{
    profiler_db, profiler_failure, profiling_state, query_shapes, set_profiling_level,
};
use super::session::Session;
use super::tunneling::{
    AttemptCurrent, ConnectionCurrent, Currentness, SshSecretSource, open_tunnel_if_configured,
};
use super::{Command, Event, SessionId, Shared};

/// The text of the `Database` error a client-side timeout becomes, shared by every handler that
/// reports one.
pub const COMMAND_TIMEOUT_MESSAGE: &str = "timeout";

/// What a dispatched command runs as. Only a connect carries a read-only flag, and it always
/// does: the dispatcher attaches it synchronously, so a lock sent while the connect is in flight
/// reaches the store it installs.
pub(super) enum Work {
    Connect {
        cfg: Box<ConnectionConfig>,
        read_only: Arc<AtomicBool>,
    },
    /// Any command but a connect, run against the session's installed store.
    Data(Box<Command>),
}

impl Work {
    fn context(&self) -> &'static str {
        match self {
            Self::Connect { .. } => "connect",
            Self::Data(command) => command.context(),
        }
    }

    /// The database of a profiler command, whose every failure is an outcome of its own event.
    fn profiler_db(&self) -> Option<String> {
        match self {
            Self::Connect { .. } => None,
            Self::Data(command) => profiler_db(command).map(str::to_owned),
        }
    }

    fn request(&self) -> Option<u64> {
        match self {
            Self::Connect { .. } => None,
            Self::Data(command) => command.request(),
        }
    }
}

/// Runs `work` to completion, folding any error into `Event::Failed`.
pub(super) async fn execute(
    shared: Arc<Shared>,
    session: Arc<Session>,
    id: SessionId,
    work: Work,
    generation: u64,
) -> Event {
    let (context, request, profiler) = (work.context(), work.request(), work.profiler_db());
    match run(&shared, &session, id, work, generation).await {
        Ok(event) => event,
        // A profiler command shows its failure inline, wherever it stopped. `Superseded` stays an
        // `Event::Failed` like every superseded command: the UI drops it, as the answer of a
        // connection the person already left, and it is no failure to show.
        Err(error) => match profiler {
            Some(db) if error != CoreError::Superseded => profiler_failure(id, db, error),
            _ => failure_event(id, context, request, generation, error),
        },
    }
}

/// The event for a failed command. A real (not superseded) connect failure is stamped with the
/// connect's own epoch so receivers can tell it from a newer request's outcome; a numbered read
/// (`request`) echoes its number.
pub(super) fn failure_event(
    session: SessionId,
    context: &'static str,
    request: Option<u64>,
    generation: u64,
    error: CoreError,
) -> Event {
    match request {
        _ if context == "connect" && error != CoreError::Superseded => Event::ConnectFailed {
            session,
            epoch: generation,
            error,
        },
        Some(request) => Event::RequestFailed {
            session,
            context,
            request,
            error,
        },
        None => Event::Failed {
            session,
            context,
            error,
        },
    }
}

/// The event to send for a data command: `Superseded` if a newer Connect/Disconnect of its session
/// exists.
pub(super) fn finalize(
    session: &Session,
    id: SessionId,
    generation: u64,
    context: &'static str,
    event: Event,
) -> Event {
    // A finished bulk write is the one answer that outlives its session: it may report changes
    // the server applied, so it is never replaced by `Superseded`.
    if session.is_current(generation)
        || matches!(
            event,
            Event::BulkFinished { .. } | Event::BulkUndoFinished { .. }
        )
    {
        event
    } else {
        Event::Failed {
            session: id,
            context,
            error: CoreError::Superseded,
        }
    }
}

/// Sends a command's event. For data commands the last currentness check and the send happen
/// under the READ side of the session's `epoch_lock`, so `Session::begin` (WRITE side) cannot
/// interleave: a stale answer is either already queued before the new epoch's events or replaced
/// by `Superseded`.
/// The std lock is held only across non-blocking calls, never across an `.await`.
pub(super) fn send_answer(
    session: &Session,
    id: SessionId,
    generation: u64,
    is_data: bool,
    context: &'static str,
    event: Event,
    events: &tokio::sync::mpsc::UnboundedSender<Event>,
) -> bool {
    if !is_data {
        return events.send(event).is_ok();
    }
    let _reading = session.epoch_lock.read().unwrap_or_else(|e| e.into_inner());
    events
        .send(finalize(session, id, generation, context, event))
        .is_ok()
}

/// `error` if no newer Connect/Disconnect replaced this command's session, else `Superseded`:
/// whatever went wrong belongs to a session the user has already left.
pub(super) fn gate(session: &Session, generation: u64, error: CoreError) -> CoreError {
    if session.is_current(generation) {
        error
    } else {
        CoreError::Superseded
    }
}

async fn run(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    id: SessionId,
    work: Work,
    generation: u64,
) -> Result<Event> {
    let command = match work {
        Work::Connect { cfg, read_only } => {
            return connect_store(shared, session, *cfg, generation, read_only)
                .await
                .map_err(|e| gate(session, generation, e));
        }
        Work::Data(command) => *command,
    };
    let store = session
        .current_store()
        .ok_or_else(|| CoreError::Unsupported("not connected".into()))
        .map_err(|e| gate(session, generation, e))?;
    let limit = command_time_limit(&shared.config, &command);
    let is_write = command.is_write();
    // An explain that outruns the bound is a missing verdict, not a failure: kept to answer it.
    let explain_target = match &command {
        Command::ExplainFind { ns, query, .. } => Some((ns.clone(), query.clone())),
        _ => None,
    };
    // Likewise a guard check: its timeout is an outcome, answered under the request's number.
    let guard_request = match &command {
        Command::GuardCheck { request, .. } => Some(*request),
        _ => None,
    };
    // And an aggregate preview: a slow pipeline says nothing about the server.
    let preview_request = match &command {
        Command::AggregatePreview { request, .. } => Some(*request),
        _ => None,
    };
    // A write run that outruns the client bound has an unknown outcome: answered, never pinged.
    let run_namespaces = match &command {
        Command::AggregateRun {
            ns, pipeline, run, ..
        } => Some((
            *run,
            ns.clone(),
            super::aggregate_run::target_namespace(ns, pipeline),
        )),
        _ => None,
    };
    // A profiler command is the one write whose read-only refusal is not decided here: the store
    // wrapper is the authority and its refusal is the outcome. Every other check (session,
    // currency, generation) still applies to it, `execute` turns its failures into outcomes.
    let is_profiler = profiler_db(&command).is_some();
    // Refused before any timeout or ping bookkeeping: a refusal is an answer, never a lost write.
    if is_write && !is_profiler && store.is_read_only() {
        return Err(gate(session, generation, CoreError::ReadOnly));
    }
    // Bulk commands settle their own outcomes: a lost write is `Unknown`, never a ping-settled
    // failure.
    let command = match super::bulk::Bulk::from_command(command) {
        Ok(bulk) => return super::bulk::run(shared, session, id, store, generation, bulk).await,
        Err(command) => command,
    };
    // A command issued for a connection that was replaced meanwhile must not run against the
    // new one (the store installed now may be the new connection's).
    if is_profiler && !session.is_current(generation) {
        return Err(CoreError::Superseded);
    }
    // The silence that is not yet a gap follows the sampling interval, a runtime tunable.
    let max_continuous = max_continuous_secs(shared.sampler_config.borrow().interval);
    let work = data_command(
        id,
        store.clone(),
        shared.history.clone(),
        max_continuous,
        super::aggregate_run::RunLimits::new(&shared.config),
        command,
    );
    // Only the client-side bound counts as a possible black hole: a server-side time limit is
    // an answer from a server that is there.
    let (result, timed_out) = match tokio::time::timeout(limit, work).await {
        Ok(result) => (result, false),
        Err(_) => (
            Err(CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into())),
            true,
        ),
    };
    // A newer Connect/Disconnect ended this command's session while it ran: its answer belongs
    // to a previous server and must not reach the UI (mutations may have committed; the UI
    // re-fetches after reconnecting).
    if !session.is_current(generation) {
        return Err(CoreError::Superseded);
    }
    // No ping settle for an explain: `Unreachable` would mark the session lost and raise a
    // banner, and a slow `executionStats` run says nothing about the server.
    if let (true, Some((ns, query))) = (timed_out, explain_target) {
        return Ok(Event::Explain {
            session: id,
            ns,
            query,
            outcome: Err(CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into())),
        });
    }
    // The same holds for a guard check: it is a batch of plan-only explains, so a timeout is an
    // outcome, never a ping-settled failure.
    if let (true, Some(request)) = (timed_out, guard_request) {
        return Ok(Event::GuardChecked {
            session: id,
            request,
            outcome: Err(CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into())),
        });
    }
    if let (true, Some(request)) = (timed_out, preview_request) {
        return Ok(Event::AggregatePreviewed {
            session: id,
            request,
            outcome: Err(CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into())),
        });
    }
    if let (true, Some((run, ns, target))) = (timed_out, run_namespaces.clone()) {
        return Ok(Event::AggregateRan {
            session: id,
            run,
            ns,
            target,
            outcome: Err(CoreError::WriteOutcomeUnknown),
        });
    }
    // A profiler command is not settled by a ping either, whether it timed out or lost the
    // connection: a timeout is an outcome, and the loss is detected by the next ordinary command
    // or the sampler. `execute` turns the error into its outcome.
    if is_profiler {
        return result;
    }
    match result {
        Err(error) if timed_out || matches!(error, CoreError::ConnectionInterrupted(_)) => {
            let settled = settle_by_ping(&store, shared.config.ping_timeout, error, is_write).await;
            // A run whose write may have gone through while the server still answers is an
            // outcome under the run's own number; only a lost server fails the command.
            // No `gate` here on purpose: the answer carries the run number and can only settle that run.
            if let (CoreError::WriteOutcomeUnknown, Some((run, ns, target))) =
                (&settled, run_namespaces)
            {
                return Ok(Event::AggregateRan {
                    session: id,
                    run,
                    ns,
                    target,
                    outcome: Err(CoreError::WriteOutcomeUnknown),
                });
            }
            Err(gate(session, generation, settled))
        }
        other => other,
    }
}

/// A suspect failure (one lost connection, or a command that outran its bound) says little about
/// the server. A short ping on the same store tells: if it answers, the failure stays an ordinary
/// retryable error; if it does not (or not in time), the server cannot be reached. A write the
/// live server may have applied before the failure is `WriteOutcomeUnknown` instead, so the
/// person is not told it simply failed (saving again could duplicate it).
async fn settle_by_ping(
    store: &Arc<dyn DocumentStore>,
    limit: std::time::Duration,
    error: CoreError,
    is_write: bool,
) -> CoreError {
    match bounded(limit, store.ping()).await {
        Ok(()) if is_write => CoreError::WriteOutcomeUnknown,
        Ok(()) => match error {
            CoreError::ConnectionInterrupted(text) => CoreError::Database(text),
            other => other,
        },
        Err(_) => CoreError::Unreachable,
    }
}

/// Whether `error` says the connection is gone or suspect rather than that the command failed.
pub(super) fn is_connection_error(error: &CoreError) -> bool {
    matches!(
        error,
        CoreError::Unreachable | CoreError::TlsHandshake | CoreError::ConnectionInterrupted(_)
    )
}

async fn data_command(
    session: SessionId,
    store: Arc<dyn DocumentStore>,
    history: Option<Arc<HistoryDb>>,
    max_continuous: i64,
    run_limits: super::aggregate_run::RunLimits,
    command: Command,
) -> Result<Event> {
    let service = DocumentService::new(store.clone());
    Ok(match command {
        Command::ListDatabases { .. } => Event::Databases {
            session,
            names: store.list_databases().await?,
        },
        Command::ListCollections { db, .. } => {
            let names = store.list_collections(&db).await?;
            Event::Collections { session, db, names }
        }
        Command::Find { ns, query, .. } => {
            let page = store.find(&ns, &query).await?;
            Event::Results {
                session,
                ns,
                query,
                page,
            }
        }
        Command::ExplainFind { ns, query, .. } => {
            // Both the server call and the classification fold into the outcome: a miss verdict
            // is advice, not a failure, so nothing here may reach `Event::Failed`.
            let outcome = match store.explain_find(&ns, &query).await {
                Ok(explain) => classify(&explain.document, &query),
                Err(error) => Err(error),
            };
            // Only a verdict is recorded; a failed explain says nothing about the query.
            if let (Ok(verdict), Some(history)) = (&outcome, history.as_ref()) {
                let kind = verdict.kind;
                let now = crate::clock::now_secs;
                super::typed_record::record_typed(history, session, &ns, &query, kind, &now);
            }
            Event::Explain {
                session,
                ns,
                query,
                outcome,
            }
        }
        Command::GuardCheck {
            request,
            targets,
            min_docs,
            ..
        } => {
            let budget = std::time::Duration::from_millis(crate::guard::GUARD_BUDGET_MS);
            let report = crate::guard::check(&*store, targets, min_docs, budget).await;
            Event::GuardChecked {
                session,
                request,
                outcome: Ok(report),
            }
        }
        Command::AggregatePreview {
            request,
            ns,
            pipeline,
            ..
        } => {
            // Every failure of the pipeline, the store's write-stage refusal included, is the
            // outcome: the preview pane shows it inline, never as a banner. A lost connection is
            // not the pipeline's failure: it fails the command like any read's, so `run` settles
            // an interruption by a ping and the UI learns the server is gone.
            let max_time = std::time::Duration::from_millis(crate::pipeline::PREVIEW_MAX_TIME_MS);
            let outcome = match store.aggregate_preview(&ns, pipeline, max_time).await {
                Err(error) if is_connection_error(&error) => return Err(error),
                other => other,
            };
            Event::AggregatePreviewed {
                session,
                request,
                outcome,
            }
        }
        Command::AggregateRun {
            ns, pipeline, run, ..
        } => super::aggregate_run::run(session, &*store, ns, pipeline, run, run_limits).await?,
        Command::Insert { ns, json, .. } => {
            service.insert_from_json(&ns, &json).await?;
            Event::Mutated { session, ns }
        }
        Command::Replace { ns, id, json, .. } => {
            service.replace_from_json(&ns, &id, &json).await?;
            Event::Mutated { session, ns }
        }
        Command::Delete { ns, id, .. } => {
            service.delete(&ns, &id).await?;
            Event::Mutated { session, ns }
        }
        Command::Clone { ns, source, .. } => {
            service.clone_document(&ns, &source).await?;
            Event::Mutated { session, ns }
        }
        Command::ListIndexes { ns, request, .. } => {
            let indexes = store.list_indexes(&ns).await?;
            Event::Indexes {
                session,
                ns,
                indexes,
                request,
            }
        }
        Command::IndexHistory {
            ns, range, request, ..
        } => {
            // Without history there is nothing to restrict, so skip the round trip. Only names and
            // definitions are needed, so the sizes (a `collStats` call) are not read.
            let listed = match history {
                Some(_) => store.list_index_specs(&ns).await?,
                None => Vec::new(),
            };
            let series =
                super::indexes::series_for(history, session, &ns, range, &listed, max_continuous)
                    .await?;
            Event::IndexHistory {
                session,
                ns,
                range,
                series,
                request,
            }
        }
        Command::ProfilingState { db, .. } => profiling_state(session, &store, db).await,
        Command::SetProfilingLevel { db, level, .. } => {
            set_profiling_level(session, &store, db, level).await
        }
        Command::QueryShapes {
            ns, range, request, ..
        } => query_shapes(session, history, ns, range, request).await?,
        Command::EstimatedCount { ns, .. } => {
            let count = store.estimated_count(&ns).await?;
            Event::EstimatedCount { session, ns, count }
        }
        Command::CreateIndex { ns, spec, .. } => {
            store.create_index(&ns, &spec).await?;
            Event::IndexChanged { session, ns }
        }
        Command::DropIndex { ns, name, .. } => {
            store.drop_index(&ns, &name).await?;
            Event::IndexChanged { session, ns }
        }
        Command::SetIndexHidden {
            ns, name, hidden, ..
        } => {
            store.set_index_hidden(&ns, &name, hidden).await?;
            Event::IndexChanged { session, ns }
        }
        Command::SetIndexTtl {
            ns, name, seconds, ..
        } => {
            store.set_index_ttl(&ns, &name, seconds).await?;
            Event::IndexChanged { session, ns }
        }
        Command::DropCollection { ns, .. } => {
            store.drop_collection(&ns).await?;
            Event::AdminChanged { session, db: ns.db }
        }
        Command::DropDatabase { db, .. } => {
            store.drop_database(&db).await?;
            Event::AdminChanged { session, db }
        }
        Command::RenameCollection {
            ns,
            new_name,
            drop_target,
            ..
        } => {
            store.rename_collection(&ns, &new_name, drop_target).await?;
            Event::AdminChanged { session, db: ns.db }
        }
        Command::CreateView {
            ns,
            source,
            pipeline_json,
            ..
        } => {
            let pipeline = crate::admin::parse_pipeline(&pipeline_json)?;
            store.create_view(&ns, &source, &pipeline).await?;
            Event::AdminChanged { session, db: ns.db }
        }
        Command::DuplicateCollection { ns, target, .. } => {
            store.duplicate_collection(&ns, &target).await?;
            Event::AdminChanged { session, db: ns.db }
        }
        Command::CollectionStats { ns, request, .. } => {
            let stats = store.collection_stats(&ns).await?;
            Event::CollectionStats {
                session,
                ns,
                stats,
                request,
            }
        }
        Command::DatabaseStats { db, request, .. } => {
            let stats = store.database_stats(&db).await?;
            Event::DatabaseStats {
                session,
                db,
                stats,
                request,
            }
        }
        Command::Connect(_)
        | Command::Disconnect { .. }
        | Command::Compare(_)
        | Command::Copy(_)
        | Command::TestConnection { .. }
        | Command::LockReadOnly { .. }
        | Command::SetSamplingExcluded { .. }
        | Command::PurgeHistory { .. }
        | Command::BulkCount { .. }
        | Command::BulkSample { .. }
        | Command::BulkPreview { .. }
        | Command::BulkUpdate { .. }
        | Command::BulkDelete { .. }
        | Command::BulkCancel { .. }
        | Command::BulkUndoPreview { .. }
        | Command::BulkUndo { .. }
        | Command::SnapshotList { .. }
        | Command::SnapshotDelete { .. }
        | Command::ShellRun { .. }
        | Command::ShellCancel { .. }
        | Command::ShellHistory { .. }
        | Command::ClearShellHistory { .. }
        | Command::CopyShellHistory { .. } => {
            unreachable!(
                "connection, compare, copy and bulk commands are handled before data dispatch"
            )
        }
    })
}

/// Applies the timeout; an elapsed deadline becomes `Database("timeout")`.
pub(super) async fn bounded<T>(
    limit: std::time::Duration,
    work: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::time::timeout(limit, work)
        .await
        .map_err(|_| CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into()))?
}

/// Opens and installs a store. Superseded connects (a newer Connect or a
/// Disconnect was requested meanwhile) drop their client and report so.
/// A timed-out keychain read keeps running in the background (blocking
/// tasks cannot be cancelled); its result is discarded.
async fn connect_store(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    cfg: ConnectionConfig,
    generation: u64,
    read_only: Arc<AtomicBool>,
) -> Result<Event> {
    cfg.validate()?;
    let limit = shared.config.connect_timeout;
    // The tunnel lives in this scope until installed; a failure, timeout or
    // cancellation anywhere below drops it, which closes it.
    let attempt = JobFlag::new();
    let kept = cfg.clone();
    let (store, tunnel, topology) = bounded(limit, async {
        // Ends the attempt however this block ends (success, error, timeout, cancellation): a
        // host-key answer arriving later is then rejected, not remembered.
        let _end = attempt.end_on_drop();
        let password = read_password(shared, &cfg).await?;
        let job = Arc::new(AttemptCurrent {
            connection: ConnectionCurrent {
                session: session.clone(),
                generation,
            },
            attempt: attempt.clone(),
        });
        let tunnel = open_tunnel_if_configured(shared, job, &cfg, SshSecretSource::Stored).await?;
        let addr = tunnel.as_ref().map(|t| t.local_addr());
        let store = shared
            .connector
            .connect(&cfg, password.as_deref(), addr)
            .await?;
        let store = ReadOnlyStore::wrap(store, read_only.clone());
        // Informational: a slow or failing read is `None`, it never fails the connect.
        let topology = bounded(shared.config.command_timeout, store.topology())
            .await
            .unwrap_or(None);
        Ok((store, tunnel, topology))
    })
    .await?;
    if session.install_with_config(generation, store, tunnel, Some(kept)) {
        Ok(Event::Connected {
            session: cfg.id,
            epoch: generation,
            topology,
        })
    } else {
        Err(CoreError::Superseded)
    }
}

/// The keychain call blocks, so it runs off the async workers.
pub(super) async fn read_password(
    shared: &Shared,
    cfg: &ConnectionConfig,
) -> Result<Option<String>> {
    let Some(secrets) = shared.secrets.clone() else {
        return Ok(None);
    };
    let key = secret_key(cfg.id, SecretKind::Password);
    tokio::task::spawn_blocking(move || secrets.get(&key))
        .await
        .map_err(|_| CoreError::Secret("secret lookup failed".into()))?
}

/// Runs one connection test in its own task, apart from the epoch machinery. The work runs in an
/// inner task so a panic still produces exactly one answer.
pub(super) fn spawn_test(
    shared: Arc<Shared>,
    events: tokio::sync::mpsc::UnboundedSender<Event>,
    repaint: super::Repaint,
    request_id: u64,
    cfg: ConnectionConfig,
    secrets: super::TestSecrets,
) {
    // A newer test ends the previous one's flag, so only one test's question is ever live. A test
    // is never replaced by a Connect/Disconnect, so only this flag decides its currentness.
    let flag = JobFlag::new();
    let previous = shared
        .latest_test
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .replace(flag.clone());
    if let Some(previous) = previous {
        previous.end();
    }
    tokio::spawn(async move {
        let work = tokio::spawn(async move { test_connection(&shared, cfg, secrets, flag).await });
        let outcome = work
            .await
            .unwrap_or_else(|_| Err(CoreError::Database("internal error".into())));
        if events
            .send(Event::TestConnection {
                request_id,
                outcome,
            })
            .is_ok()
        {
            repaint();
        }
    });
}

async fn test_connection(
    shared: &Arc<Shared>,
    cfg: ConnectionConfig,
    secrets: super::TestSecrets,
    flag: JobFlag,
) -> Result<super::TestReport> {
    // Ends the test however it ends (answer, error, timeout, panic, cancellation): a host-key
    // answer arriving later is rejected, so nothing reaches known_hosts.
    let _end = flag.end_on_drop();
    cfg.validate()?;
    let limit = shared.config.connect_timeout;
    bounded(limit, async {
        let password = match &secrets.password {
            super::PasswordChoice::Typed(p) => Some(p.expose().to_owned()),
            super::PasswordChoice::Without => None,
            super::PasswordChoice::Stored => read_password(shared, &cfg).await?,
        };
        let job: Arc<dyn Currentness> = Arc::new(flag.clone());
        let ssh_secret = match &secrets.ssh_secret {
            super::PasswordChoice::Typed(s) => SshSecretSource::Typed(s.expose()),
            super::PasswordChoice::Without => SshSecretSource::Without,
            super::PasswordChoice::Stored => SshSecretSource::Stored,
        };
        let tunnel = open_tunnel_if_configured(shared, job, &cfg, ssh_secret).await?;
        let addr = tunnel.as_ref().map(|t| t.local_addr());
        let store = shared
            .connector
            .connect(&cfg, password.as_deref(), addr)
            .await?;
        let server_version = store.server_version().await?;
        // Dropped here, before the answer: a test leaves no client and no tunnel behind.
        drop(store);
        drop(tunnel);
        Ok(super::TestReport {
            server_version,
            tls: cfg.tls.enabled,
            authenticated: cfg.auth != crate::connection::AuthMechanism::None,
        })
    })
    .await
}

/// The bound a command runs under. An index build and a collection duplicate (a full copy, then
/// its index builds) can run for a long time and must not be cut off half-way; every other
/// command keeps the short bound. An aggregate preview is cut off shortly after the server's own
/// time limit, and never later than the short bound.
pub(super) fn run_write_limit(config: &super::RuntimeConfig) -> std::time::Duration {
    config
        .index_build_timeout
        .min(super::config::AGGREGATE_RUN_TIMEOUT)
}

pub(super) fn command_time_limit(
    config: &super::RuntimeConfig,
    command: &Command,
) -> std::time::Duration {
    match command {
        Command::CreateIndex { .. } | Command::DuplicateCollection { .. } => {
            config.index_build_timeout
        }
        Command::AggregatePreview { .. } => config
            .command_timeout
            .min(super::config::AGGREGATE_PREVIEW_TIMEOUT),
        Command::AggregateRun { .. } => {
            run_write_limit(config)
                + super::config::AGGREGATE_RUN_COUNT_TIMEOUT
                + super::config::AGGREGATE_RUN_OUTER_SLACK
        }
        _ => config.command_timeout,
    }
}
