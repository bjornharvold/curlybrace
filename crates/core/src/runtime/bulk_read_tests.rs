//! The bulk commands that read, and the classification of every bulk command: read-only
//! refusal, count, sample and preview answers.

use std::sync::atomic::Ordering;
use std::time::Duration;

use bson::doc;

use super::bulk_support::*;
use super::*;
use crate::bulk::BULK_PREVIEW_DOCS;
use crate::test_support::mock_objects::{mock_doc_with_id, mock_namespace};

#[test]
fn bulk_commands_are_classified_as_writes_or_reads() {
    let expected = [
        (false, "bulk_count", Some(1)),
        (false, "bulk_sample", Some(2)),
        (true, "bulk_preview", None),
        (true, "bulk_update", None),
        (true, "bulk_delete", None),
        (false, "bulk_cancel", None),
    ];
    let session = uuid::Uuid::new_v4();
    for (command, (write, context, request)) in all_bulk_commands(session).iter().zip(expected) {
        assert_eq!(command.is_write(), write, "{context}");
        assert_eq!(command.context(), context);
        assert_eq!(command.session(), Some(session));
        assert_eq!(command.request(), request, "{context}");
    }
}

#[test]
fn the_default_bulk_write_bound_is_an_hour() {
    assert_eq!(BULK_WRITE_TIMEOUT, Duration::from_secs(3600));
    assert_eq!(
        RuntimeConfig::default().bulk_write_timeout,
        BULK_WRITE_TIMEOUT
    );
}

#[tokio::test]
async fn read_only_session_refuses_preview_update_and_delete_before_the_store() {
    let store = FakeStore::new(Write::Answer(Ok(applied())));
    let (mut h, session) = connected(&store, true).await;
    for command in all_bulk_commands(session)
        .into_iter()
        .filter(Command::is_write)
    {
        let context = command.context();
        h.tx.send(command).unwrap();
        assert_eq!(
            next(&mut h).await,
            Event::Failed {
                session,
                context,
                error: CoreError::ReadOnly
            }
        );
    }
    assert_eq!(store.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn bulk_count_is_allowed_on_read_only() {
    let store = FakeStore::new(Write::Answer(Ok(applied())));
    let (mut h, session) = connected(&store, true).await;
    h.tx.send(Command::BulkCount {
        session,
        ns: mock_namespace(),
        filter: doc! { "a": 1 },
        request: 5,
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::BulkCount {
            session,
            ns: mock_namespace(),
            request: 5,
            count: Some(7)
        }
    );
}

#[tokio::test]
async fn bulk_count_answers_none_when_the_count_timed_out() {
    let store = FakeStore::new(Write::Hang).with(|s| s.count = Ok(None));
    let (mut h, session) = connected(&store, false).await;
    h.tx.send(Command::BulkCount {
        session,
        ns: mock_namespace(),
        filter: doc! {},
        request: 6,
    })
    .unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::BulkCount {
            request: 6,
            count: None,
            ..
        }
    ));
}

#[tokio::test]
async fn bulk_count_failure_echoes_its_request() {
    let store =
        FakeStore::new(Write::Hang).with(|s| s.count = Err(CoreError::Database("x".into())));
    let (mut h, session) = connected(&store, false).await;
    h.tx.send(Command::BulkCount {
        session,
        ns: mock_namespace(),
        filter: doc! {},
        request: 8,
    })
    .unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::RequestFailed {
            context: "bulk_count",
            request: 8,
            ..
        }
    ));
}

#[tokio::test]
async fn bulk_sample_reads_the_first_documents_with_the_filter_alone() {
    let store = FakeStore::new(Write::Hang);
    let (mut h, session) = connected(&store, true).await;
    h.tx.send(Command::BulkSample {
        session,
        ns: mock_namespace(),
        filter: doc! { "a": 1 },
        request: 4,
    })
    .unwrap();
    assert_eq!(
        next(&mut h).await,
        Event::BulkSample {
            session,
            ns: mock_namespace(),
            request: 4,
            docs: vec![mock_doc_with_id(1_i32)],
        }
    );
    let expected = Query {
        filter: doc! { "a": 1 },
        limit: BULK_PREVIEW_DOCS as u64,
        ..Query::default()
    };
    assert_eq!(*store.queries.lock().unwrap(), vec![expected]);
}

#[tokio::test]
async fn preview_failures_are_inline_outcomes() {
    for error in [
        CoreError::Database("boom".into()),
        CoreError::ConnectionInterrupted("reset".into()),
    ] {
        let store = FakeStore::new(Write::Hang).with(|s| s.preview = Err(error.clone()));
        let (mut h, session) = connected(&store, false).await;
        h.tx.send(Command::BulkPreview {
            session,
            ns: mock_namespace(),
            request: update_request(),
            request_id: 11,
        })
        .unwrap();
        assert_eq!(
            next(&mut h).await,
            Event::BulkPreview {
                session,
                ns: mock_namespace(),
                request_id: 11,
                outcome: Err(error)
            }
        );
        assert_eq!(store.pings.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn a_hung_count_arrives_as_a_request_failed_timeout() {
    let store = FakeStore::new(Write::Hang).with(|s| s.count_hangs = true);
    let (mut h, session) = connected(&store, false).await;
    h.tx.send(Command::BulkCount {
        session,
        ns: mock_namespace(),
        filter: doc! {},
        request: 9,
    })
    .unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::RequestFailed {
            context: "bulk_count",
            request: 9,
            error: CoreError::Database(_),
            ..
        }
    ));
}
