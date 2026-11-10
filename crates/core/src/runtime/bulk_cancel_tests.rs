//! A superseded epoch, a write finishing after a reconnect, and a cancel that reaches a write
//! still in flight.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bson::doc;

use super::bulk_support::*;
use super::*;
use crate::bulk::{BulkOutcome, BulkRunId, CancelEffect};
use crate::test_support::mock_objects::mock_namespace;

/// The old store's thread is held inside its read-only check, between the runtime reading the
/// store and the currentness guard, while a newer `Connect` installs the new store.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn superseded_epoch_does_not_run_a_bulk_write() {
    let old = FakeStore::new(Write::Answer(Ok(applied())));
    let new = FakeStore::new(Write::Answer(Ok(applied())));
    let (release, gate) = std::sync::mpsc::channel();
    *old.hold.lock().unwrap() = Some(gate);
    let old_store: Arc<dyn DocumentStore> = old.clone();
    let mut h = spawn_runtime_with_connector(
        None,
        Some(old_store),
        Arc::new(FakeConnector(new.clone())),
        Arc::new(|| {}),
        runtime_config(),
    );
    h.tx.send(Command::BulkUpdate {
        session: EMBEDDED_SESSION,
        ns: mock_namespace(),
        request: update_request(),
        run: run_id(),
        undo: crate::bulk::UndoPolicy::None,
    })
    .unwrap();
    tokio::time::timeout(WAIT, old.held.notified())
        .await
        .expect("the command reached the old store");
    h.tx.send(Command::Connect(ConnectionConfig {
        id: EMBEDDED_SESSION,
        ..config(false)
    }))
    .unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    release.send(()).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::Failed {
            context: "bulk_update",
            error: CoreError::Superseded,
            ..
        }
    ));
    assert_eq!(old.calls.load(Ordering::SeqCst), 0);
    assert_eq!(new.calls.load(Ordering::SeqCst), 0);
}

/// A write that completes after its session was replaced is still reported.
async fn finished_after_reconnect(command: impl FnOnce(SessionId, BulkRunId) -> Command) {
    let store = FakeStore::new(Write::UntilReleased);
    let config = RuntimeConfig {
        bulk_write_timeout: Duration::from_secs(60),
        ..runtime_config()
    };
    let (mut h, session) = connected_with(&store, false, config).await;
    let run = run_id();
    h.tx.send(command(session, run)).unwrap();
    tokio::time::timeout(WAIT, store.entered.notified())
        .await
        .expect("the write started");
    h.tx.send(Command::Connect(ConnectionConfig {
        id: session,
        ..config_for(session)
    }))
    .unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    store.release.notify_one();
    assert_eq!(
        next(&mut h).await,
        Event::BulkFinished {
            session,
            ns: mock_namespace(),
            run,
            outcome: Ok(applied()),
            undo: crate::bulk::UndoStatus::NotRequested,
        }
    );
}

fn config_for(session: SessionId) -> ConnectionConfig {
    ConnectionConfig {
        id: session,
        ..config(false)
    }
}

#[tokio::test]
async fn a_bulk_update_finishing_after_a_reconnect_is_still_reported() {
    finished_after_reconnect(|session, run| Command::BulkUpdate {
        session,
        ns: mock_namespace(),
        request: update_request(),
        run,
        undo: crate::bulk::UndoPolicy::None,
    })
    .await;
}

#[tokio::test]
async fn a_bulk_delete_finishing_after_a_reconnect_is_still_reported() {
    finished_after_reconnect(|session, run| Command::BulkDelete {
        session,
        ns: mock_namespace(),
        request: delete_request(),
        run,
        undo: crate::bulk::UndoPolicy::None,
    })
    .await;
}

#[tokio::test]
async fn cancel_reaches_a_bulk_write_that_is_still_in_flight() {
    let store = FakeStore::new(Write::UntilCancelled);
    let config = RuntimeConfig {
        bulk_write_timeout: Duration::from_secs(60),
        ..runtime_config()
    };
    let (mut h, session) = connected_with(&store, false, config).await;
    let run = run_id();
    h.tx.send(Command::BulkUpdate {
        session,
        ns: mock_namespace(),
        request: update_request(),
        run,
        undo: crate::bulk::UndoPolicy::None,
    })
    .unwrap();
    tokio::time::timeout(WAIT, store.entered.notified())
        .await
        .expect("the write started");
    h.tx.send(Command::BulkCancel { session, run }).unwrap();
    let events = [next(&mut h).await, next(&mut h).await];
    assert!(events.contains(&Event::BulkCancelResult {
        session,
        run,
        outcome: Ok(CancelEffect::Stopped)
    }));
    assert!(events.contains(&Event::BulkFinished {
        session,
        ns: mock_namespace(),
        run,
        outcome: Ok(BulkOutcome::Unknown { cancelled: true }),
        undo: crate::bulk::UndoStatus::NotRequested,
    }));
}

#[tokio::test]
async fn a_cancel_that_found_nothing_to_stop_says_so_and_the_write_carries_on() {
    let store = FakeStore::new(Write::UntilReleased).with(|s| {
        s.cancel_effect = CancelEffect::NothingToStop;
    });
    let config = RuntimeConfig {
        bulk_write_timeout: Duration::from_secs(60),
        ..runtime_config()
    };
    let (mut h, session) = connected_with(&store, false, config).await;
    let run = run_id();
    h.tx.send(Command::BulkUpdate {
        session,
        ns: mock_namespace(),
        request: update_request(),
        run,
        undo: crate::bulk::UndoPolicy::None,
    })
    .unwrap();
    tokio::time::timeout(WAIT, store.entered.notified())
        .await
        .expect("the write started");
    h.tx.send(Command::BulkCancel { session, run }).unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::BulkCancelResult {
            session,
            run,
            outcome: Ok(CancelEffect::NothingToStop)
        }
    );
    // Nothing was stopped: the write is still running and finishes on its own.
    store.release.notify_one();
    assert!(matches!(
        next(&mut h).await,
        Event::BulkFinished {
            outcome: Ok(BulkOutcome::Applied { .. }),
            ..
        }
    ));
}
