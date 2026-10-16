//! Command execution. Every function maps its outcome to a single [`Event`].

use std::future::Future;
use std::sync::Arc;

use crate::connection::ConnectionConfig;
use crate::crud::DocumentService;
use crate::error::{CoreError, Result};
use crate::secrets::{SecretKind, secret_key};
use crate::store::DocumentStore;

use super::job_flag::JobFlag;
use super::session::Session;
use super::tunneling::{
    AttemptCurrent, ConnectionCurrent, Currentness, SshSecretSource, open_tunnel_if_configured,
};
use super::{Command, Event, SessionId, Shared};

/// Runs `command` to completion, folding any error into `Event::Failed`.
pub(super) async fn execute(
    shared: Arc<Shared>,
    session: Arc<Session>,
    id: SessionId,
    command: Command,
    generation: u64,
) -> Event {
    let context = command.context();
    match run(&shared, &session, id, command, generation).await {
        Ok(event) => event,
        Err(error) => failure_event(id, context, generation, error),
    }
}

/// The event for a failed command. A real (not superseded) connect failure is stamped with the
/// connect's own epoch so receivers can tell it from a newer request's outcome.
pub(super) fn failure_event(
    session: SessionId,
    context: &'static str,
    generation: u64,
    error: CoreError,
) -> Event {
    if context == "connect" && error != CoreError::Superseded {
        Event::ConnectFailed {
            session,
            epoch: generation,
            error,
        }
    } else {
        Event::Failed {
            session,
            context,
            error,
        }
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
    if session.is_current(generation) {
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
    command: Command,
    generation: u64,
) -> Result<Event> {
    if let Command::Connect(cfg) = command {
        return connect_store(shared, session, cfg, generation)
            .await
            .map_err(|e| gate(session, generation, e));
    }
    let store = session
        .current_store()
        .ok_or_else(|| CoreError::Unsupported("not connected".into()))
        .map_err(|e| gate(session, generation, e))?;
    let limit = shared.config.command_timeout;
    let is_write = command.is_write();
    let work = data_command(id, store.clone(), command);
    // Only the client-side bound counts as a possible black hole: a server-side time limit is
    // an answer from a server that is there.
    let (result, timed_out) = match tokio::time::timeout(limit, work).await {
        Ok(result) => (result, false),
        Err(_) => (Err(CoreError::Database("timeout".into())), true),
    };
    // A newer Connect/Disconnect ended this command's session while it ran: its answer belongs
    // to a previous server and must not reach the UI (mutations may have committed; the UI
    // re-fetches after reconnecting).
    if !session.is_current(generation) {
        return Err(CoreError::Superseded);
    }
    match result {
        Err(error) if timed_out || matches!(error, CoreError::ConnectionInterrupted(_)) => {
            let settled = settle_by_ping(&store, shared.config.ping_timeout, error, is_write).await;
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

async fn data_command(
    session: SessionId,
    store: Arc<dyn DocumentStore>,
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
        Command::Connect(_)
        | Command::Disconnect { .. }
        | Command::Compare(_)
        | Command::TestConnection { .. } => {
            unreachable!("connection and compare commands are handled before data dispatch")
        }
    })
}

/// Applies the timeout; an elapsed deadline becomes `Database("timeout")`.
async fn bounded<T>(
    limit: std::time::Duration,
    work: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::time::timeout(limit, work)
        .await
        .map_err(|_| CoreError::Database("timeout".into()))?
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
) -> Result<Event> {
    cfg.validate()?;
    let limit = shared.config.connect_timeout;
    // The tunnel lives in this scope until installed; a failure, timeout or
    // cancellation anywhere below drops it, which closes it.
    let attempt = JobFlag::new();
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
        // Informational: a slow or failing read is `None`, it never fails the connect.
        let topology = bounded(shared.config.command_timeout, store.topology())
            .await
            .unwrap_or(None);
        Ok((store, tunnel, topology))
    })
    .await?;
    if session.install_if_current(generation, store, tunnel) {
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
