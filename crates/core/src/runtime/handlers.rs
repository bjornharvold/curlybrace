//! Command execution. Every function maps its outcome to a single [`Event`].

use std::future::Future;
use std::sync::Arc;

use crate::connection::ConnectionConfig;
use crate::crud::DocumentService;
use crate::error::{CoreError, Result};
use crate::secrets::{SecretKind, secret_key};
use crate::store::DocumentStore;

use super::tunneling::{ConnectionCurrent, Currentness, open_tunnel_if_configured};
use super::{Command, Event, Shared};

/// Runs `command` to completion, folding any error into `Event::Failed`.
pub(super) async fn execute(shared: Arc<Shared>, command: Command, generation: u64) -> Event {
    let context = command.context();
    match run(&shared, command, generation).await {
        Ok(event) => event,
        Err(error) => failure_event(context, generation, error),
    }
}

/// The event for a failed command. A real (not superseded) connect failure is stamped with the
/// connect's own epoch so receivers can tell it from a newer request's outcome.
pub(super) fn failure_event(context: &'static str, generation: u64, error: CoreError) -> Event {
    if context == "connect" && error != CoreError::Superseded {
        Event::ConnectFailed {
            epoch: generation,
            error,
        }
    } else {
        Event::Failed { context, error }
    }
}

/// The event to send for a data command: `Superseded` if a newer Connect/Disconnect exists.
pub(super) fn finalize(
    shared: &Shared,
    generation: u64,
    context: &'static str,
    event: Event,
) -> Event {
    if shared.is_current(generation) {
        event
    } else {
        Event::Failed {
            context,
            error: CoreError::Superseded,
        }
    }
}

/// Sends a command's event. For data commands the last currentness check and the send happen
/// under the READ side of `epoch_lock`, so `begin_epoch` (WRITE side) cannot interleave: a stale
/// answer is either already queued before the new epoch's events or replaced by `Superseded`.
/// The std lock is held only across non-blocking calls, never across an `.await`.
pub(super) fn send_answer(
    shared: &Shared,
    generation: u64,
    is_data: bool,
    context: &'static str,
    event: Event,
    events: &tokio::sync::mpsc::UnboundedSender<Event>,
) -> bool {
    if !is_data {
        return events.send(event).is_ok();
    }
    let _reading = shared.epoch_lock.read().unwrap_or_else(|e| e.into_inner());
    events
        .send(finalize(shared, generation, context, event))
        .is_ok()
}

/// `error` if no newer Connect/Disconnect replaced this command's session, else `Superseded`:
/// whatever went wrong belongs to a session the user has already left.
pub(super) fn gate(shared: &Shared, generation: u64, error: CoreError) -> CoreError {
    if shared.is_current(generation) {
        error
    } else {
        CoreError::Superseded
    }
}

async fn run(shared: &Arc<Shared>, command: Command, generation: u64) -> Result<Event> {
    match command {
        Command::Connect(cfg) => {
            return connect_store(shared, cfg, generation)
                .await
                .map_err(|e| gate(shared, generation, e));
        }
        // The store was already dropped by the dispatcher.
        Command::Disconnect => return Ok(Event::Disconnected { epoch: generation }),
        _ => {}
    }
    let store = current_store(shared)
        .await
        .map_err(|e| gate(shared, generation, e))?;
    let limit = shared.config.command_timeout;
    let result = bounded(limit, data_command(store, command)).await;
    // A newer Connect/Disconnect ended this command's session while it ran: its answer belongs
    // to a previous server and must not reach the UI (mutations may have committed; the UI
    // re-fetches after reconnecting).
    if shared.is_current(generation) {
        result
    } else {
        Err(CoreError::Superseded)
    }
}

async fn data_command(store: Arc<dyn DocumentStore>, command: Command) -> Result<Event> {
    let service = DocumentService::new(store.clone());
    Ok(match command {
        Command::ListDatabases => Event::Databases(store.list_databases().await?),
        Command::ListCollections { db } => {
            let names = store.list_collections(&db).await?;
            Event::Collections { db, names }
        }
        Command::Find { ns, query } => {
            let page = store.find(&ns, &query).await?;
            Event::Results { ns, query, page }
        }
        Command::Insert { ns, json } => {
            service.insert_from_json(&ns, &json).await?;
            Event::Mutated { ns }
        }
        Command::Replace { ns, id, json } => {
            service.replace_from_json(&ns, &id, &json).await?;
            Event::Mutated { ns }
        }
        Command::Delete { ns, id } => {
            service.delete(&ns, &id).await?;
            Event::Mutated { ns }
        }
        Command::Clone { ns, source } => {
            service.clone_document(&ns, &source).await?;
            Event::Mutated { ns }
        }
        Command::Connect(_)
        | Command::Disconnect
        | Command::Compare(_)
        | Command::TestConnection { .. } => {
            unreachable!("connection and compare commands are handled before data dispatch")
        }
    })
}

async fn current_store(shared: &Shared) -> Result<Arc<dyn DocumentStore>> {
    shared
        .current_store()
        .ok_or_else(|| CoreError::Unsupported("not connected".into()))
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
    cfg: ConnectionConfig,
    generation: u64,
) -> Result<Event> {
    cfg.validate()?;
    let limit = shared.config.connect_timeout;
    // The tunnel lives in this scope until installed; a failure, timeout or
    // cancellation anywhere below drops it, which closes it.
    let (store, tunnel) = bounded(limit, async {
        let password = read_password(shared, &cfg).await?;
        let job = Arc::new(ConnectionCurrent {
            shared: shared.clone(),
            generation,
        });
        let tunnel = open_tunnel_if_configured(shared, job, &cfg, None).await?;
        let addr = tunnel.as_ref().map(|t| t.local_addr());
        let store = shared
            .connector
            .connect(&cfg, password.as_deref(), addr)
            .await?;
        Ok((store, tunnel))
    })
    .await?;
    if shared.install_if_current(generation, store, tunnel) {
        Ok(Event::Connected { epoch: generation })
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

/// A test is never replaced by a newer Connect/Disconnect, so its host-key question stays live.
struct AlwaysCurrent;

impl Currentness for AlwaysCurrent {
    fn is_current(&self) -> bool {
        true
    }
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
    tokio::spawn(async move {
        let work = tokio::spawn(async move { test_connection(&shared, cfg, secrets).await });
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
) -> Result<super::TestReport> {
    cfg.validate()?;
    let limit = shared.config.connect_timeout;
    bounded(limit, async {
        let password = match &secrets.password {
            super::PasswordChoice::Typed(p) => Some(p.expose().to_owned()),
            super::PasswordChoice::Without => None,
            super::PasswordChoice::Stored => read_password(shared, &cfg).await?,
        };
        let job: Arc<dyn Currentness> = Arc::new(AlwaysCurrent);
        let ssh_secret = secrets.ssh_secret.as_ref().map(|s| s.expose());
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
