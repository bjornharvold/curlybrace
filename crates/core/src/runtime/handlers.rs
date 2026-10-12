//! Command execution. Every function maps its outcome to a single [`Event`].

use std::future::Future;
use std::sync::Arc;

use crate::connection::ConnectionConfig;
use crate::crud::DocumentService;
use crate::error::{CoreError, Result};
use crate::secrets::{SecretKind, secret_key};
use crate::store::DocumentStore;

use super::tunneling::open_tunnel_if_configured;
use super::{Command, Event, Shared};

/// Runs `command` to completion, folding any error into `Event::Failed`.
pub(super) async fn execute(shared: Arc<Shared>, command: Command, generation: u64) -> Event {
    let context = command.context();
    match run(&shared, command, generation).await {
        Ok(event) => event,
        Err(error) => Event::Failed { context, error },
    }
}

async fn run(shared: &Shared, command: Command, generation: u64) -> Result<Event> {
    match command {
        Command::Connect(cfg) => return connect_store(shared, cfg, generation).await,
        // The store was already dropped by the dispatcher.
        Command::Disconnect => return Ok(Event::Disconnected),
        _ => {}
    }
    let store = current_store(shared).await?;
    let limit = shared.config.command_timeout;
    bounded(limit, data_command(store, command)).await
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
        Command::Connect(_) | Command::Disconnect => {
            unreachable!("connection commands are handled before data dispatch")
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
async fn connect_store(shared: &Shared, cfg: ConnectionConfig, generation: u64) -> Result<Event> {
    cfg.validate()?;
    let limit = shared.config.command_timeout;
    // The tunnel lives in this scope until installed; a failure, timeout or
    // cancellation anywhere below drops it, which closes it.
    let (store, tunnel) = bounded(limit, async {
        let password = read_password(shared, &cfg).await?;
        let tunnel = open_tunnel_if_configured(shared, &cfg).await?;
        let addr = tunnel.as_ref().map(|t| t.local_addr());
        let store = shared
            .connector
            .connect(&cfg, password.as_deref(), addr)
            .await?;
        Ok((store, tunnel))
    })
    .await?;
    if shared.install_if_current(generation, store, tunnel) {
        Ok(Event::Connected)
    } else {
        Err(CoreError::Unsupported("connection superseded".into()))
    }
}

/// The keychain call blocks, so it runs off the async workers.
async fn read_password(shared: &Shared, cfg: &ConnectionConfig) -> Result<Option<String>> {
    let Some(secrets) = shared.secrets.clone() else {
        return Ok(None);
    };
    let key = secret_key(cfg.id, SecretKind::Password);
    tokio::task::spawn_blocking(move || secrets.get(&key))
        .await
        .map_err(|_| CoreError::Secret("secret lookup failed".into()))?
}
