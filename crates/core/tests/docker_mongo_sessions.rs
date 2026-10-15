//! Two simultaneous sessions through the real runtime against two real servers: each answers
//! with its own data, and one server dying fails only that session's commands.
//! Tier 2 (docker): own `cb-it-*` containers; run with `scripts/container-tests.sh`.

mod support;

use std::sync::Arc;
use std::time::Duration;

use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event, RuntimeConfig, spawn_runtime_with_config};
use curlybrace_core::secrets::MemorySecretStore;
use curlybrace_core::store::{DocumentStore, Namespace, Query};
use support::{docker_enabled, docker_mongo, next_event, no_repaint};

const LIMIT: Duration = Duration::from_secs(10);
const SLACK: Duration = Duration::from_secs(3);

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn two_sessions_answer_independently_and_a_dead_server_fails_only_its_own() {
    if !docker_enabled("two_sessions_answer_independently_and_a_dead_server_fails_only_its_own") {
        return;
    }
    let (a, _client_a, store_a) = docker_mongo().await;
    let (b, _client_b, store_b) = docker_mongo().await;
    let (ns_a, ns_b) = (
        Namespace::new("cbit_only_a", "c"),
        Namespace::new("cbit_only_b", "c"),
    );
    store_a.insert(&ns_a, doc! { "_id": 1_i32 }).await.unwrap();
    store_b
        .insert(&ns_b, doc! { "_id": 2_i32, "v": "b" })
        .await
        .unwrap();

    let config = RuntimeConfig {
        command_timeout: LIMIT,
        ..RuntimeConfig::default()
    };
    let mut rt = spawn_runtime_with_config(
        Some(Arc::new(MemorySecretStore::new())),
        None,
        no_repaint(),
        config,
    );
    let (cfg_a, cfg_b) = (support::config(&a), support::config(&b));
    assert_ne!(cfg_a.id, cfg_b.id);

    for cfg in [&cfg_a, &cfg_b] {
        rt.tx.send(Command::Connect(cfg.clone())).unwrap();
        match next_event(&mut rt.rx, LIMIT, "connect").await {
            Event::Connected { session, .. } => assert_eq!(session, cfg.id),
            other => panic!("expected Connected, got {other:?}"),
        }
    }

    // Each session lists its own server's databases and not the other's.
    for (cfg, own, other_db) in [
        (&cfg_a, "cbit_only_a", "cbit_only_b"),
        (&cfg_b, "cbit_only_b", "cbit_only_a"),
    ] {
        rt.tx
            .send(Command::ListDatabases { session: cfg.id })
            .unwrap();
        match next_event(&mut rt.rx, LIMIT, "databases").await {
            Event::Databases { session, names } => {
                assert_eq!(session, cfg.id);
                assert!(names.iter().any(|n| n == own), "{names:?}");
                assert!(!names.iter().any(|n| n == other_db), "{names:?}");
            }
            other => panic!("expected Databases, got {other:?}"),
        }
    }

    a.stop();

    // B still answers, stamped with B's session.
    rt.tx
        .send(Command::Find {
            session: cfg_b.id,
            ns: ns_b.clone(),
            query: Query::default(),
        })
        .unwrap();
    match next_event(&mut rt.rx, LIMIT, "find on the live server").await {
        Event::Results { session, page, .. } => {
            assert_eq!(session, cfg_b.id);
            assert_eq!(page.docs, vec![doc! { "_id": 2_i32, "v": "b" }]);
        }
        other => panic!("expected Results, got {other:?}"),
    }

    // A's next command fails with a connection error stamped with A, never B.
    rt.tx
        .send(Command::Find {
            session: cfg_a.id,
            ns: ns_a,
            query: Query::default(),
        })
        .unwrap();
    match next_event(&mut rt.rx, LIMIT + SLACK, "find on the dead server").await {
        Event::Failed {
            session,
            context: "find",
            error,
        } => {
            assert_eq!(session, cfg_a.id);
            assert_ne!(session, cfg_b.id);
            assert!(matches!(error, CoreError::Database(_)), "{error:?}");
        }
        other => panic!("expected Failed{{find}}, got {other:?}"),
    }

    // B is still healthy after A's failure.
    rt.tx
        .send(Command::ListDatabases { session: cfg_b.id })
        .unwrap();
    assert!(matches!(
        next_event(&mut rt.rx, LIMIT, "b after a failed").await,
        Event::Databases { session, .. } if session == cfg_b.id
    ));
}
