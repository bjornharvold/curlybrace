//! A server that disappears mid-session must surface as `Failed`, never as a
//! hang, and a fresh `Connect` must recover once it is back.
//! Tier 2 (docker): own `cb-it-*` container. Run with `scripts/container-tests.sh`.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event, RuntimeConfig, spawn_runtime_with_config};
use curlybrace_core::secrets::MemorySecretStore;
use curlybrace_core::store::{DocumentStore, Namespace, Query};
use support::{docker_enabled, docker_mongo, next_event, no_repaint};

const COMMAND_LIMIT: Duration = Duration::from_secs(10);
/// Slack on top of the command timeout for scheduling and event delivery.
const SLACK: Duration = Duration::from_secs(3);

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn stopped_server_yields_failed_find_in_time_and_restart_recovers() {
    if !docker_enabled("stopped_server_yields_failed_find_in_time_and_restart_recovers") {
        return;
    }
    let (container, _client, store) = docker_mongo().await;
    let ns = Namespace::new("cbit_drop", "drop");
    store
        .insert(&ns, doc! { "_id": 1_i32, "v": "kept" })
        .await
        .unwrap();

    let config = RuntimeConfig {
        command_timeout: COMMAND_LIMIT,
        ..RuntimeConfig::default()
    };
    let mut rt = spawn_runtime_with_config(
        Some(Arc::new(MemorySecretStore::new())),
        None,
        no_repaint(),
        config,
    );
    let cfg = support::config(&container);
    let find = || Command::Find {
        session: cfg.id,
        ns: ns.clone(),
        query: Query::default(),
    };

    rt.tx.send(Command::Connect(cfg.clone())).unwrap();
    assert!(matches!(
        next_event(&mut rt.rx, COMMAND_LIMIT, "connect").await,
        Event::Connected { .. }
    ));
    rt.tx.send(find()).unwrap();
    assert!(matches!(
        next_event(&mut rt.rx, COMMAND_LIMIT, "first find").await,
        Event::Results { .. }
    ));

    container.stop();
    let started = Instant::now();
    rt.tx.send(find()).unwrap();
    match next_event(&mut rt.rx, COMMAND_LIMIT + SLACK, "find on a dead server").await {
        Event::Failed {
            context: "find",
            error,
            ..
        } => {
            assert!(
                matches!(error, CoreError::Database(_)),
                "unexpected error {error:?}"
            );
        }
        other => panic!("expected Failed{{find}}, got {other:?}"),
    }
    assert!(
        started.elapsed() < COMMAND_LIMIT + SLACK,
        "took {:?}",
        started.elapsed()
    );

    container.start();
    // A fresh Connect (retried while mongod is still starting) works again.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        rt.tx
            .send(Command::Connect(support::config(&container)))
            .unwrap();
        match next_event(&mut rt.rx, COMMAND_LIMIT + SLACK, "reconnect").await {
            Event::Connected { .. } => break,
            Event::ConnectFailed { .. } if Instant::now() < deadline => {}
            other => panic!("reconnect failed: {other:?}"),
        }
    }
    rt.tx.send(find()).unwrap();
    match next_event(&mut rt.rx, COMMAND_LIMIT, "find after restart").await {
        Event::Results { page, .. } => {
            assert_eq!(page.docs, vec![doc! { "_id": 1_i32, "v": "kept" }])
        }
        other => panic!("expected Results, got {other:?}"),
    }
}
