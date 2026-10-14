//! Driver-specific helpers for the core integration tests. The generic parts (env gates, the
//! `cb-it-*` docker guards, the SSH stack) live in `curlybrace-testkit` and are re-exported.
//! Tests are `#[ignore]` and additionally gated on `CURLYBRACE_IT=1` via `local_enabled` / `docker_enabled`, which
//! prints why a test was skipped.
#![allow(dead_code, unused_imports)]

use std::time::{Duration, Instant};

use curlybrace_core::client;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::mongo_store::MongoStore;
use mongodb::Client;

pub mod compare_flow;
pub mod export_flow;
pub mod mongosh;

pub use curlybrace_testkit::{
    Container, SSH_PASSWORD, SSH_USER, SshStack, docker_enabled, local_enabled,
};

const READY_TIMEOUT: Duration = Duration::from_secs(90);

/// The connection config for a test container.
pub fn config(container: &Container) -> ConnectionConfig {
    ConnectionConfig::new("it", &container.uri())
}

/// Polls until the server answers a ping (and authenticates, if configured).
pub async fn wait_ready(cfg: &ConnectionConfig, password: Option<&str>) -> Client {
    wait_ready_for(cfg, password, READY_TIMEOUT).await
}

async fn wait_ready_for(cfg: &ConnectionConfig, password: Option<&str>, limit: Duration) -> Client {
    let started = Instant::now();
    loop {
        match client::connect(cfg, password, None).await {
            Ok(client) => return client,
            Err(e) if started.elapsed() > limit => {
                panic!("mongod at {} never became ready: {e}", cfg.uri)
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
}

/// Tier 2: a fresh `mongo:7` container (fixed port, so it can be stopped and started) plus a
/// connected client and store.
pub async fn docker_mongo() -> (Container, Client, MongoStore) {
    let container = Container::start_mongo_restartable(&[], &[]);
    let client = wait_ready(&config(&container), None).await;
    let store = MongoStore::new(client.clone());
    (container, client, store)
}

/// Tier 1: the already-running local server. All writes go to databases owned by the returned
/// guard (`cbit_<id>` names), which are dropped when it goes out of scope.
pub struct LocalDbs {
    uri: String,
    pub db: String,
    pub unicode_db: String,
}

impl LocalDbs {
    pub fn ns(&self, collection: &str) -> curlybrace_core::store::Namespace {
        curlybrace_core::store::Namespace::new(&self.db, collection)
    }
}

impl Drop for LocalDbs {
    /// Best effort: removes only this test's own databases (on a helper thread, because Drop can
    /// run inside an async runtime).
    fn drop(&mut self) {
        let (uri, dbs) = (
            self.uri.clone(),
            vec![self.db.clone(), self.unicode_db.clone()],
        );
        let cleanup = std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Runtime::new() else {
                return;
            };
            rt.block_on(async {
                let Ok(client) = Client::with_uri_str(&uri).await else {
                    return;
                };
                for db in dbs {
                    if let Err(e) = client.database(&db).drop().await {
                        eprintln!("could not drop test database {db}: {e}");
                    }
                }
            });
        });
        let _ = cleanup.join();
    }
}

/// Connects to the local server (a failure to reach it FAILS the test with instructions).
pub async fn local_mongo() -> (LocalDbs, Client, MongoStore) {
    let uri = curlybrace_testkit::local_uri();
    let cfg = ConnectionConfig::new("it", &uri);
    let client = match tokio::time::timeout(
        Duration::from_secs(15),
        wait_ready_for(&cfg, None, Duration::from_secs(10)),
    )
    .await
    {
        Ok(client) => client,
        Err(_) => panic!(
            "{}",
            curlybrace_testkit::unreachable_message(&uri, "timeout")
        ),
    };
    let dbs = LocalDbs {
        uri,
        db: curlybrace_testkit::unique_db_name(""),
        // MongoDB forbids spaces and dots in database names, so this one is unicode only.
        unicode_db: curlybrace_testkit::unique_db_name("база_данных"),
    };
    let store = MongoStore::new(client.clone());
    (dbs, client, store)
}

/// Waits up to `limit` for the next runtime event; panics with context on timeout.
pub async fn next_event(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<curlybrace_core::runtime::Event>,
    limit: Duration,
    what: &str,
) -> curlybrace_core::runtime::Event {
    match tokio::time::timeout(limit, rx.recv()).await {
        Ok(Some(event)) => event,
        Ok(None) => panic!("runtime closed while waiting for {what}"),
        Err(_) => panic!("no event for {what} within {limit:?}"),
    }
}

/// A repaint callback that does nothing.
pub fn no_repaint() -> curlybrace_core::runtime::Repaint {
    std::sync::Arc::new(|| {})
}

/// Tier 1: like [`local_mongo`], plus a runtime wired to the real driver and the config that
/// points a compare session at the same server.
pub async fn local_runtime() -> (
    LocalDbs,
    Client,
    curlybrace_core::runtime::RuntimeHandle,
    ConnectionConfig,
) {
    let (dbs, client, _store) = local_mongo().await;
    let config = ConnectionConfig::new("local", &dbs.uri);
    let runtime = curlybrace_core::runtime::spawn_runtime_with_config(
        None,
        None,
        no_repaint(),
        curlybrace_core::runtime::RuntimeConfig::default(),
    );
    (dbs, client, runtime, config)
}

/// The next compare event from the runtime (other events are skipped); panics after 10 seconds.
pub async fn next_compare(
    runtime: &mut curlybrace_core::runtime::RuntimeHandle,
) -> curlybrace_core::compare::protocol::CompareEvent {
    loop {
        let event = next_event(&mut runtime.rx, Duration::from_secs(10), "a compare event").await;
        if let curlybrace_core::runtime::Event::Compare(e) = event {
            return e;
        }
    }
}

/// Inserts `docs` into `ns` through the driver.
pub async fn insert_labelled(
    client: &mongodb::Client,
    ns: &curlybrace_core::store::Namespace,
    docs: &[bson::Document],
) {
    client
        .database(&ns.db)
        .collection::<bson::Document>(&ns.collection)
        .insert_many(docs.to_vec())
        .await
        .unwrap();
}
