//! A numbered read that fails reports the number, so the caller can tell a late failure from the
//! one of the request it is waiting for.

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use super::*;
use crate::history::ChartRange;
use crate::test_support::InMemoryStore;

async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(Duration::from_secs(5), h.rx.recv())
        .await
        .expect("event within bound")
        .expect("channel open")
}

#[tokio::test]
async fn a_failed_numbered_read_echoes_its_request_number() {
    let mut h = spawn_runtime_with_config(
        None,
        Some(Arc::new(InMemoryStore::new())),
        Arc::new(|| {}),
        RuntimeConfig::default(),
    );
    // No session answers to this id: the reads fail before reaching a store.
    let session = Uuid::from_u128(404);
    let ns = Namespace::new("testdb", "people");
    h.tx.send(Command::ListIndexes {
        session,
        ns: ns.clone(),
        request: 7,
    })
    .unwrap();
    h.tx.send(Command::IndexHistory {
        session,
        ns,
        range: ChartRange::Day,
        request: 8,
    })
    .unwrap();

    let mut failed = Vec::new();
    for _ in 0..2 {
        match next(&mut h).await {
            Event::RequestFailed {
                context, request, ..
            } => failed.push((context, request)),
            other => panic!("expected RequestFailed, got {other:?}"),
        }
    }
    failed.sort_unstable();
    assert_eq!(failed, [("index_history", 8), ("list_indexes", 7)]);
}
