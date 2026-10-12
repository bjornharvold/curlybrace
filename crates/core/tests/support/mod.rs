//! Driver-specific helpers for the core integration tests. The generic parts (env gates, the
//! `ms-it-*` docker guards, the SSH stack) live in `mongostudio-testkit` and are re-exported.
//! Tests are `#[ignore]` and additionally gated on `MONGOSTUDIO_IT=1` via [`it_enabled`], which
//! prints why a test was skipped.
#![allow(dead_code, unused_imports)]

use std::time::{Duration, Instant};

use mongodb::Client;
use mongostudio_core::client;
use mongostudio_core::connection::ConnectionConfig;
use mongostudio_core::mongo_store::MongoStore;

pub use mongostudio_testkit::{Container, SSH_PASSWORD, SSH_USER, SshStack, it_enabled};

const READY_TIMEOUT: Duration = Duration::from_secs(90);

/// The connection config for a test container.
pub fn config(container: &Container) -> ConnectionConfig {
    ConnectionConfig::new("it", &container.uri())
}

/// Polls until the server answers a ping (and authenticates, if configured).
pub async fn wait_ready(cfg: &ConnectionConfig, password: Option<&str>) -> Client {
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        match client::connect(cfg, password, None).await {
            Ok(client) => return client,
            Err(e) if Instant::now() > deadline => panic!("mongod never became ready: {e}"),
            Err(_) => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
}

/// A plain (no auth) server plus a connected client and store.
pub async fn plain_mongo() -> (Container, Client, MongoStore) {
    let container = Container::start_mongo(&[], &[]);
    let client = wait_ready(&config(&container), None).await;
    let store = MongoStore::new(client.clone());
    (container, client, store)
}

/// Waits up to `limit` for the next runtime event; panics with context on timeout.
pub async fn next_event(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<mongostudio_core::runtime::Event>,
    limit: Duration,
    what: &str,
) -> mongostudio_core::runtime::Event {
    match tokio::time::timeout(limit, rx.recv()).await {
        Ok(Some(event)) => event,
        Ok(None) => panic!("runtime closed while waiting for {what}"),
        Err(_) => panic!("no event for {what} within {limit:?}"),
    }
}

/// A repaint callback that does nothing.
pub fn no_repaint() -> mongostudio_core::runtime::Repaint {
    std::sync::Arc::new(|| {})
}
