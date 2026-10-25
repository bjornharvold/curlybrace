//! Copy runs over two open sessions: event order, previews, refusals and what an event may carry.

use std::sync::atomic::Ordering;

use bson::doc;
use uuid::Uuid;

use super::copy_support::*;
use super::*;
use crate::copy::{
    ConflictPolicy, CopyCommand, CopyEvent, CopyOutcome, CopyRequest, CopyScope, PreviewCounts,
};
use crate::test_support::mock_objects::mock_numbered_docs;

#[tokio::test]
async fn preview_emits_planned_then_counts_then_done() {
    let target_docs = vec![doc! { "_id": 0, "n": 0 }, doc! { "_id": 1, "n": 99 }];
    let mut rig = Rig::new(seeded(3), TestStore::new(&[("t", "orders", target_docs)])).await;
    let scope = one_item();
    rig.send(CopyCommand::Preview {
        run: 1,
        request: rig.request(scope.clone(), ConflictPolicy::Skip),
    });
    let events = rig.until_terminal(1).await;
    let CopyScope::Collections(items) = scope else {
        unreachable!()
    };
    assert_eq!(events[0], CopyEvent::Planned { run: 1, items });
    let counts: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, CopyEvent::PreviewItemDone { .. }))
        .collect();
    assert_eq!(
        counts,
        [&CopyEvent::PreviewItemDone {
            run: 1,
            index: 0,
            counts: Some(PreviewCounts {
                new: 1,
                changed: 1,
                unchanged: 1
            })
        }]
    );
    assert_eq!(events.last(), Some(&CopyEvent::PreviewDone { run: 1 }));
    assert_eq!(
        rig.target.writes.load(Ordering::SeqCst),
        0,
        "a preview writes nothing"
    );
}

#[tokio::test]
async fn preview_with_counts_over_the_compare_cap_is_not_a_failure() {
    let source = TestStore::new(&[
        ("s", "big", mock_numbered_docs(2)),
        ("s", "small", mock_numbered_docs(2)),
    ])
    .failing_scan("big", CoreError::CompareCapExceeded);
    let mut rig = Rig::new(source, empty()).await;
    let scope = CopyScope::Collections(vec![
        item(("s", "big"), ("t", "big")),
        item(("s", "small"), ("t", "small")),
    ]);
    rig.send(CopyCommand::Preview {
        run: 1,
        request: rig.request(scope, ConflictPolicy::Skip),
    });
    let events = rig.until_terminal(1).await;
    let done: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            CopyEvent::PreviewItemDone { index, counts, .. } => Some((*index, *counts)),
            _ => None,
        })
        .collect();
    let small = PreviewCounts {
        new: 2,
        changed: 0,
        unchanged: 0,
    };
    assert_eq!(done, [(0, None), (1, Some(small))]);
    assert_eq!(events.last(), Some(&CopyEvent::PreviewDone { run: 1 }));
}

#[tokio::test]
async fn preview_fails_on_any_other_error() {
    let source = seeded(2).failing_scan("orders", CoreError::Database("boom".into()));
    let mut rig = Rig::new(source, empty()).await;
    rig.send(CopyCommand::Preview {
        run: 1,
        request: rig.request(one_item(), ConflictPolicy::Skip),
    });
    let events = rig.until_terminal(1).await;
    assert_eq!(
        events.last(),
        Some(&CopyEvent::Failed {
            run: 1,
            error: CoreError::Database("boom".into())
        })
    );
}

#[tokio::test]
async fn start_copies_and_emits_item_done_then_done() {
    let mut rig = Rig::new(seeded(3), empty()).await;
    rig.send(CopyCommand::Start {
        run: 7,
        request: rig.request(one_item(), ConflictPolicy::Skip),
    });
    let events = rig.until_terminal(7).await;
    assert!(matches!(events[0], CopyEvent::Planned { run: 7, .. }));
    let reports = item_reports(&events);
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].outcome, CopyOutcome::Completed);
    assert_eq!(reports[0].progress.written, 3);
    assert_eq!(
        events.last(),
        Some(&CopyEvent::Done {
            run: 7,
            cancelled: false
        })
    );
    let done_at = events.len() - 1;
    assert!(matches!(events[done_at - 1], CopyEvent::ItemDone { .. }));
    assert_eq!(rig.target.contents(&ns("t", "orders")).await.len(), 3);
}

#[tokio::test]
async fn progress_carries_the_estimated_total() {
    let mut rig = Rig::new(seeded(3), empty()).await;
    rig.send(CopyCommand::Start {
        run: 1,
        request: rig.request(one_item(), ConflictPolicy::Skip),
    });
    let events = rig.until_terminal(1).await;
    let progress: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            CopyEvent::Progress {
                progress,
                estimated_total,
                ..
            } => Some((progress.written, *estimated_total)),
            _ => None,
        })
        .collect();
    assert_eq!(progress, [(3, Some(3))]);
}

#[tokio::test]
async fn a_finished_run_frees_the_slot_before_its_last_event() {
    let mut rig = Rig::new(seeded(3), empty()).await;
    for run in [1, 2] {
        rig.send(CopyCommand::Start {
            run,
            request: rig.request(one_item(), ConflictPolicy::Skip),
        });
        let events = rig.until_terminal(run).await;
        assert!(
            matches!(events.last(), Some(CopyEvent::Done { .. })),
            "run {run} ends with Done, not a refusal"
        );
    }
}

#[tokio::test]
async fn database_scope_skips_system_collections_and_views() {
    let source = TestStore::new(&[
        ("s", "orders", mock_numbered_docs(2)),
        ("s", "items", mock_numbered_docs(1)),
        ("s", "system.profile", mock_numbered_docs(1)),
    ])
    .with_views(&["vw"]);
    let mut rig = Rig::new(source, empty()).await;
    let scope = CopyScope::Database {
        source_db: "s".into(),
        target_db: "t".into(),
    };
    rig.send(CopyCommand::Start {
        run: 1,
        request: rig.request(scope, ConflictPolicy::Skip),
    });
    let events = rig.until_terminal(1).await;
    assert_eq!(
        events[0],
        CopyEvent::Planned {
            run: 1,
            items: vec![
                item(("s", "items"), ("t", "items")),
                item(("s", "orders"), ("t", "orders")),
            ]
        }
    );
    assert_eq!(
        events.last(),
        Some(&CopyEvent::Done {
            run: 1,
            cancelled: false
        })
    );
    let target = &rig.target;
    assert_eq!(target.contents(&ns("t", "orders")).await.len(), 2);
    assert_eq!(target.contents(&ns("t", "items")).await.len(), 1);
    assert!(target.contents(&ns("t", "system.profile")).await.is_empty());
    assert!(target.contents(&ns("t", "vw")).await.is_empty());
}

#[tokio::test]
async fn target_read_only_store_fails_before_any_write() {
    let mut rig = Rig::with_read_only(seeded(3), empty(), true).await;
    rig.send(CopyCommand::Start {
        run: 1,
        request: rig.request(one_item(), ConflictPolicy::Skip),
    });
    let events = rig.until_terminal(1).await;
    assert_eq!(
        events,
        [CopyEvent::Failed {
            run: 1,
            error: CoreError::ReadOnly
        }]
    );
    assert_eq!(rig.target.writes.load(Ordering::SeqCst), 0);
    assert_eq!(
        rig.source.scans.load(Ordering::SeqCst),
        0,
        "nothing was read either"
    );
    assert!(rig.target.contents(&ns("t", "orders")).await.is_empty());
}

#[tokio::test]
async fn a_read_only_target_can_still_be_previewed() {
    let mut rig = Rig::with_read_only(seeded(3), empty(), true).await;
    rig.send(CopyCommand::Preview {
        run: 1,
        request: rig.request(one_item(), ConflictPolicy::Skip),
    });
    let events = rig.until_terminal(1).await;
    assert_eq!(events.last(), Some(&CopyEvent::PreviewDone { run: 1 }));
}

#[tokio::test]
async fn refusals_come_before_any_read_or_write() {
    let source = TestStore::new(&[
        ("admin", "x", mock_numbered_docs(1)),
        ("s", "orders", vec![]),
    ]);
    let mut rig = Rig::new(source, empty()).await;
    let database = |source_db: &str, target_db: &str| CopyScope::Database {
        source_db: source_db.into(),
        target_db: target_db.into(),
    };
    let refused = [
        database("admin", "t"),
        database("s", "bad name"),
        database("s", "config"),
        CopyScope::Collections(vec![item(("s", "orders"), ("t", "system.users"))]),
        CopyScope::Collections(vec![item(("s", "system.profile"), ("t", "x"))]),
        CopyScope::Collections(vec![item(("local", "oplog"), ("t", "x"))]),
        CopyScope::Collections(vec![item(("s", "orders"), ("t", "a$b"))]),
        CopyScope::Collections(vec![item(("s", "orders"), ("t", ""))]),
        CopyScope::Collections(Vec::new()),
    ];
    for (run, scope) in (1..).zip(refused) {
        rig.send(CopyCommand::Start {
            run,
            request: rig.request(scope, ConflictPolicy::Skip),
        });
        let events = rig.until_terminal(run).await;
        assert_eq!(events.len(), 1, "run {run}: a single refusal, no Planned");
        assert!(
            matches!(
                &events[0],
                CopyEvent::Failed {
                    error: CoreError::Unsupported(_),
                    ..
                }
            ),
            "run {run}: {events:?}"
        );
    }
    assert_eq!(rig.source.scans.load(Ordering::SeqCst), 0);
    assert_eq!(rig.target.writes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_session_that_is_not_open_is_refused() {
    let mut rig = Rig::new(seeded(1), empty()).await;
    let request = CopyRequest {
        source: Uuid::new_v4(),
        ..rig.request(one_item(), ConflictPolicy::Skip)
    };
    rig.send(CopyCommand::Start { run: 1, request });
    let events = rig.until_terminal(1).await;
    assert_eq!(
        events,
        [CopyEvent::Failed {
            run: 1,
            error: CoreError::Unsupported("not connected".into())
        }]
    );
}

#[tokio::test]
async fn copying_a_collection_onto_itself_stays_unsupported() {
    let mut rig = Rig::new(seeded(2), empty()).await;
    let request = CopyRequest {
        target: rig.source_cfg.id,
        ..rig.request(
            CopyScope::Collections(vec![item(("s", "orders"), ("s", "orders"))]),
            ConflictPolicy::Upsert,
        )
    };
    rig.send(CopyCommand::Start { run: 1, request });
    let events = rig.until_terminal(1).await;
    let reports = item_reports(&events);
    assert!(matches!(
        reports[0].outcome,
        CopyOutcome::Failed(CoreError::Unsupported(_))
    ));
    assert_eq!(reports[0].progress.written, 0);
    assert!(matches!(
        events.last(),
        Some(CopyEvent::Failed {
            error: CoreError::Unsupported(_),
            ..
        })
    ));
}

#[tokio::test]
async fn no_event_contains_a_document_value() {
    const SECRET: &str = "SECRET-VALUE-4711";
    let docs = vec![
        doc! { "_id": 1, "card": SECRET },
        doc! { "_id": 2, "card": SECRET },
    ];
    let target_docs = vec![doc! { "_id": 2, "card": "other" }];
    let source = TestStore::new(&[("s", "orders", docs)]);
    let mut rig = Rig::new(source, TestStore::new(&[("t", "orders", target_docs)])).await;
    let mut seen = Vec::new();
    rig.send(CopyCommand::Preview {
        run: 1,
        request: rig.request(one_item(), ConflictPolicy::Upsert),
    });
    seen.extend(rig.until_terminal(1).await);
    rig.send(CopyCommand::Start {
        run: 2,
        request: rig.request(one_item(), ConflictPolicy::Upsert),
    });
    seen.extend(rig.until_terminal(2).await);
    assert!(seen.len() > 4, "both runs produced events");
    assert!(
        !format!("{seen:?}").contains(SECRET),
        "no event may carry a document value"
    );
}
