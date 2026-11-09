//! The runtime side of undo: the snapshot is taken before the write, a snapshot that cannot be
//! taken stops the write, undo is offered after every outcome that may have changed documents,
//! and a snapshot only restores on the connection it came from.

use bson::{Document, doc};
use tempfile::tempdir;

use super::bulk_support::*;
use super::*;
use crate::bulk::{BulkOutcome, BulkRunId, UndoOffer, UndoPolicy};
use crate::error::CoreError;
use crate::snapshot::{
    SnapshotHeader, SnapshotKind, SnapshotReader, SnapshotWriter, UndoReport, snapshot_path,
};
use crate::test_support::mock_objects::mock_namespace;

fn originals() -> Vec<Document> {
    vec![doc! { "_id": 1, "v": 1 }, doc! { "_id": 2, "v": 1 }]
}

fn store_with_originals(write: Write) -> std::sync::Arc<FakeStore> {
    FakeStore::new(write).with(|s| s.snapshot_docs = originals())
}

fn delete(session: SessionId, run: BulkRunId, undo: UndoPolicy) -> Command {
    Command::BulkDelete {
        session,
        ns: mock_namespace(),
        request: delete_request(),
        run,
        undo,
    }
}

async fn finished(h: &mut RuntimeHandle) -> (Result<BulkOutcome>, Option<UndoOffer>) {
    match next(h).await {
        Event::BulkFinished { outcome, undo, .. } => (outcome, undo),
        other => panic!("expected BulkFinished, got {other:?}"),
    }
}

fn file_count(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir).map_or(0, Iterator::count)
}

#[tokio::test]
async fn the_snapshot_is_taken_before_the_write_and_undo_is_offered() {
    let dir = tempdir().unwrap();
    let store = store_with_originals(Write::Answer(Ok(applied())));
    let (mut h, session) =
        connected_with(&store, false, runtime_config_with_snapshots(dir.path())).await;
    let run = run_id();
    h.tx.send(delete(session, run, UndoPolicy::Snapshot))
        .unwrap();

    let (outcome, undo) = finished(&mut h).await;

    assert_eq!(outcome, Ok(applied()));
    assert_eq!(undo, Some(UndoOffer { documents: 2 }));
    assert_eq!(*store.order.lock().unwrap(), ["snapshot", "write"]);
    let saved: Vec<Document> = SnapshotReader::open(&snapshot_path(dir.path(), run))
        .unwrap()
        .collect::<Result<_>>()
        .unwrap();
    assert_eq!(saved, originals());
}

#[tokio::test]
async fn a_snapshot_that_cannot_be_taken_stops_the_write() {
    for error in [CoreError::SnapshotTooLarge, CoreError::SnapshotWriteFailed] {
        let dir = tempdir().unwrap();
        let store = store_with_originals(Write::Answer(Ok(applied()))).with(|s| {
            s.snapshot_error = Some(error.clone());
        });
        let (mut h, session) =
            connected_with(&store, false, runtime_config_with_snapshots(dir.path())).await;
        h.tx.send(delete(session, run_id(), UndoPolicy::Snapshot))
            .unwrap();

        let (outcome, undo) = finished(&mut h).await;

        assert_eq!(outcome, Err(error));
        assert_eq!(undo, None);
        assert_eq!(
            *store.order.lock().unwrap(),
            ["snapshot"],
            "the write never ran"
        );
        assert_eq!(file_count(dir.path()), 0, "no partial snapshot is left");
    }
}

#[tokio::test]
async fn without_a_snapshot_directory_a_snapshot_write_is_refused() {
    let store = store_with_originals(Write::Answer(Ok(applied())));
    let (mut h, session) = connected(&store, false).await;
    h.tx.send(delete(session, run_id(), UndoPolicy::Snapshot))
        .unwrap();

    let (outcome, undo) = finished(&mut h).await;

    assert_eq!(outcome, Err(CoreError::SnapshotWriteFailed));
    assert_eq!(undo, None);
    assert!(
        store.order.lock().unwrap().is_empty(),
        "nothing reached the store"
    );
}

#[tokio::test]
async fn choosing_no_undo_runs_the_write_without_a_snapshot() {
    let dir = tempdir().unwrap();
    let store = store_with_originals(Write::Answer(Ok(applied())));
    let (mut h, session) =
        connected_with(&store, false, runtime_config_with_snapshots(dir.path())).await;
    h.tx.send(delete(session, run_id(), UndoPolicy::None))
        .unwrap();

    let (outcome, undo) = finished(&mut h).await;

    assert_eq!(outcome, Ok(applied()));
    assert_eq!(undo, None);
    assert_eq!(*store.order.lock().unwrap(), ["write"]);
    assert_eq!(file_count(dir.path()), 0);
}

#[tokio::test]
async fn undo_is_offered_after_outcomes_that_may_have_changed_documents() {
    use crate::bulk::BulkFailureReason;
    let outcomes = [
        BulkOutcome::Unknown { cancelled: false },
        BulkOutcome::Unknown { cancelled: true },
        BulkOutcome::Rejected {
            reason: BulkFailureReason::Other,
        },
        BulkOutcome::Unconfirmed {
            matched: 2,
            modified: 0,
            upserted: vec![],
            deleted: 2,
        },
    ];
    for outcome in outcomes {
        let dir = tempdir().unwrap();
        let store = store_with_originals(Write::Answer(Ok(outcome.clone())));
        let (mut h, session) =
            connected_with(&store, false, runtime_config_with_snapshots(dir.path())).await;
        h.tx.send(delete(session, run_id(), UndoPolicy::Snapshot))
            .unwrap();

        let (got, undo) = finished(&mut h).await;

        assert_eq!(got, Ok(outcome.clone()));
        assert_eq!(undo, Some(UndoOffer { documents: 2 }), "{outcome:?}");
    }
}

#[tokio::test]
async fn a_run_that_touched_nothing_leaves_no_snapshot() {
    let dir = tempdir().unwrap();
    let store = FakeStore::new(Write::Answer(Ok(applied())));
    let (mut h, session) =
        connected_with(&store, false, runtime_config_with_snapshots(dir.path())).await;
    h.tx.send(delete(session, run_id(), UndoPolicy::Snapshot))
        .unwrap();

    let (_, undo) = finished(&mut h).await;

    assert_eq!(undo, None);
    assert_eq!(file_count(dir.path()), 0);
}

#[tokio::test]
async fn a_write_that_never_ran_leaves_no_snapshot() {
    let dir = tempdir().unwrap();
    let store = store_with_originals(Write::Answer(Err(CoreError::BulkInvalidUpdate)));
    let (mut h, session) =
        connected_with(&store, false, runtime_config_with_snapshots(dir.path())).await;
    h.tx.send(delete(session, run_id(), UndoPolicy::Snapshot))
        .unwrap();

    let (outcome, undo) = finished(&mut h).await;

    assert_eq!(outcome, Err(CoreError::BulkInvalidUpdate));
    assert_eq!(undo, None);
    assert_eq!(file_count(dir.path()), 0);
}

#[tokio::test]
async fn undo_restores_from_the_snapshot_and_keeps_it_for_a_retry() {
    let dir = tempdir().unwrap();
    let store = store_with_originals(Write::Answer(Ok(applied())));
    let (mut h, session) =
        connected_with(&store, false, runtime_config_with_snapshots(dir.path())).await;
    let run = run_id();
    h.tx.send(delete(session, run, UndoPolicy::Snapshot))
        .unwrap();
    let _ = finished(&mut h).await;

    h.tx.send(Command::BulkUndo { session, run }).unwrap();

    match next(&mut h).await {
        Event::BulkUndoFinished { outcome, .. } => assert_eq!(
            outcome,
            Ok(UndoReport {
                restored: 2,
                ..UndoReport::default()
            })
        ),
        other => panic!("expected BulkUndoFinished, got {other:?}"),
    }
    assert!(snapshot_path(dir.path(), run).exists(), "kept for a retry");
}

#[tokio::test]
async fn the_undo_preview_shows_originals_beside_what_is_there_now() {
    let dir = tempdir().unwrap();
    let now = doc! { "_id": 1, "v": 9 };
    let store = store_with_originals(Write::Answer(Ok(applied())))
        .with(|s| s.current = vec![doc! { "_id": 1, "v": 9 }]);
    let (mut h, session) =
        connected_with(&store, false, runtime_config_with_snapshots(dir.path())).await;
    let run = run_id();
    h.tx.send(delete(session, run, UndoPolicy::Snapshot))
        .unwrap();
    let _ = finished(&mut h).await;

    h.tx.send(Command::BulkUndoPreview {
        session,
        run,
        request_id: 7,
    })
    .unwrap();

    match next(&mut h).await {
        Event::BulkUndoPreview {
            request_id: 7,
            outcome: Ok(preview),
            ..
        } => {
            assert_eq!(preview.documents, 2);
            assert_eq!(preview.docs.len(), 2);
            assert_eq!(preview.docs[0].original, originals()[0]);
            assert_eq!(preview.docs[0].current, Some(now));
            assert_eq!(preview.docs[1].current, None, "gone now");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn a_snapshot_from_another_connection_is_never_restored() {
    let dir = tempdir().unwrap();
    let run = run_id();
    let header = SnapshotHeader {
        run,
        connection: uuid::Uuid::new_v4(), // not this session
        ns: mock_namespace(),
        kind: SnapshotKind::Delete { filter: doc! {} },
        created_unix: 0,
    };
    let mut w = SnapshotWriter::create(dir.path(), header, u64::MAX).unwrap();
    w.push(&doc! { "_id": 1 }).unwrap();
    w.finish().unwrap();
    let store = FakeStore::new(Write::Answer(Ok(applied())));
    let (mut h, session) =
        connected_with(&store, false, runtime_config_with_snapshots(dir.path())).await;

    h.tx.send(Command::BulkUndo { session, run }).unwrap();

    match next(&mut h).await {
        Event::BulkUndoFinished { outcome, .. } => {
            assert_eq!(outcome, Err(CoreError::SnapshotOtherConnection));
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(store.undone.lock().unwrap().is_empty());
}

#[tokio::test]
async fn snapshots_can_be_listed_and_deleted_without_a_session() {
    let dir = tempdir().unwrap();
    let store = store_with_originals(Write::Answer(Ok(applied())));
    let (mut h, session) =
        connected_with(&store, false, runtime_config_with_snapshots(dir.path())).await;
    let run = run_id();
    h.tx.send(delete(session, run, UndoPolicy::Snapshot))
        .unwrap();
    let _ = finished(&mut h).await;

    h.tx.send(Command::SnapshotList { request: 1 }).unwrap();
    match next(&mut h).await {
        Event::Snapshots {
            request: 1,
            snapshots,
        } => {
            assert_eq!(snapshots.len(), 1);
            assert_eq!(snapshots[0].run, run);
        }
        other => panic!("unexpected {other:?}"),
    }

    h.tx.send(Command::SnapshotDelete { run, request: 2 })
        .unwrap();
    match next(&mut h).await {
        Event::Snapshots {
            request: 2,
            snapshots,
        } => assert!(snapshots.is_empty()),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(file_count(dir.path()), 0);
}

#[tokio::test]
async fn a_read_only_connection_never_creates_a_snapshot() {
    let dir = tempdir().unwrap();
    let store = store_with_originals(Write::Answer(Ok(applied())));
    let (mut h, session) =
        connected_with(&store, true, runtime_config_with_snapshots(dir.path())).await;
    h.tx.send(delete(session, run_id(), UndoPolicy::Snapshot))
        .unwrap();

    match next(&mut h).await {
        Event::Failed {
            error: CoreError::ReadOnly,
            ..
        } => {}
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(file_count(dir.path()), 0);
}
