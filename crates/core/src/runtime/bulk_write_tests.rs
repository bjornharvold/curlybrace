//! The bulk writes: the outcome each store answer becomes, the write bound and the loss of the
//! connection.

use std::sync::atomic::Ordering;

use super::bulk_support::*;
use super::*;
use crate::bulk::{BulkFailureReason, BulkOutcome};
use crate::test_support::mock_objects::mock_namespace;

#[tokio::test]
async fn bulk_update_answers_finished_with_the_store_outcome() {
    let store = FakeStore::new(Write::Answer(Ok(applied())));
    let (mut h, session) = connected(&store, false).await;
    let run = run_id();
    h.tx.send(Command::BulkUpdate {
        session,
        ns: mock_namespace(),
        request: update_request(),
        run,
        undo: crate::bulk::UndoPolicy::None,
    })
    .unwrap();
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
    no_more_events(&mut h).await;
}

#[tokio::test]
async fn bulk_delete_and_store_errors_are_outcomes_of_finished() {
    let error = CoreError::Database("scope".into());
    let store = FakeStore::new(Write::Answer(Err(error.clone())));
    let (mut h, session) = connected(&store, false).await;
    let run = run_id();
    h.tx.send(Command::BulkDelete {
        session,
        ns: mock_namespace(),
        request: delete_request(),
        run,
        undo: crate::bulk::UndoPolicy::None,
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::BulkFinished {
            session,
            ns: mock_namespace(),
            run,
            outcome: Err(error),
            undo: crate::bulk::UndoStatus::NotRequested,
        }
    );
    assert_eq!(store.pings.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn bulk_write_timeout_answers_unknown_not_failed() {
    let store = FakeStore::new(Write::Hang);
    let (mut h, session) = connected(&store, false).await;
    let run = run_id();
    h.tx.send(Command::BulkUpdate {
        session,
        ns: mock_namespace(),
        request: update_request(),
        run,
        undo: crate::bulk::UndoPolicy::None,
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::BulkFinished {
            session,
            ns: mock_namespace(),
            run,
            outcome: Ok(BulkOutcome::Unknown { cancelled: false }),
            undo: crate::bulk::UndoStatus::NotRequested,
        }
    );
    no_more_events(&mut h).await;
    assert_eq!(store.pings.load(Ordering::SeqCst), 0, "no settle by ping");
}

#[tokio::test]
async fn a_write_that_outruns_its_bound_is_cancelled_on_the_server_before_the_answer() {
    let store = FakeStore::new(Write::Hang);
    let (mut h, session) = connected(&store, false).await;
    let run = run_id();
    h.tx.send(Command::BulkDelete {
        session,
        ns: mock_namespace(),
        request: delete_request(),
        run,
        undo: crate::bulk::UndoPolicy::None,
    })
    .unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::BulkFinished {
            outcome: Ok(BulkOutcome::Unknown { cancelled: false }),
            ..
        }
    ));
    assert_eq!(*store.cancels.lock().unwrap(), vec![run]);
}

#[tokio::test]
async fn a_rejected_write_is_reported_as_rejected_with_its_reason() {
    let rejected = BulkOutcome::Rejected {
        reason: BulkFailureReason::DuplicateKey,
    };
    let store = FakeStore::new(Write::Answer(Ok(rejected.clone())));
    let (mut h, session) = connected(&store, false).await;
    let run = run_id();
    h.tx.send(Command::BulkUpdate {
        session,
        ns: mock_namespace(),
        request: update_request(),
        run,
        undo: crate::bulk::UndoPolicy::None,
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::BulkFinished {
            session,
            ns: mock_namespace(),
            run,
            outcome: Ok(rejected),
            undo: crate::bulk::UndoStatus::NotRequested,
        }
    );
    assert!(store.cancels.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_lost_connection_answers_unknown_without_a_ping() {
    let lost = CoreError::ConnectionInterrupted("reset".into());
    let store = FakeStore::new(Write::Answer(Err(lost)));
    let (mut h, session) = connected(&store, false).await;
    let run = run_id();
    h.tx.send(Command::BulkDelete {
        session,
        ns: mock_namespace(),
        request: delete_request(),
        run,
        undo: crate::bulk::UndoPolicy::None,
    })
    .unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::BulkFinished {
            outcome: Ok(BulkOutcome::Unknown { cancelled: false }),
            ..
        }
    ));
    no_more_events(&mut h).await;
    assert_eq!(store.pings.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_unconfirmed_write_is_passed_through_with_its_counts() {
    let unconfirmed = BulkOutcome::Unconfirmed {
        matched: 5000,
        modified: 0,
        upserted: vec![],
        deleted: 5000,
    };
    let store = FakeStore::new(Write::Answer(Ok(unconfirmed.clone())));
    let (mut h, session) = connected(&store, false).await;
    let run = run_id();
    h.tx.send(Command::BulkDelete {
        session,
        ns: mock_namespace(),
        request: delete_request(),
        run,
        undo: crate::bulk::UndoPolicy::None,
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::BulkFinished {
            session,
            ns: mock_namespace(),
            run,
            outcome: Ok(unconfirmed),
            undo: crate::bulk::UndoStatus::NotRequested,
        }
    );
    assert!(store.cancels.lock().unwrap().is_empty());
}
