//! A copy run when something changes under it: a session locked, reconnected or closed, a
//! second run started, a cancel. Counts are exact and no write is started afterwards.
//!
//! The mid-run tests depend on the current-thread test runtime and on the store yielding after it
//! sends a command (see `TestStore::fire_if_armed`), so the dispatcher has handled it before the
//! store call goes on.

use std::sync::atomic::Ordering;
use std::time::Duration;

use super::copy_support::*;
use super::*;
use crate::copy::{ConflictPolicy, CopyCommand, CopyEvent, CopyOutcome};
use crate::error::CopyRefusal;

/// The run ends in `Failed(Superseded)`, with exactly the first batch written through exactly one
/// `bulk_write`, and nothing follows the terminal event.
async fn assert_superseded_after_first_batch(rig: &mut Rig, events: &[CopyEvent]) {
    assert_eq!(
        events.last(),
        Some(&CopyEvent::Failed {
            run: 1,
            error: CoreError::Superseded
        })
    );
    let reports = item_reports(events);
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].progress.written, BATCH);
    assert_eq!(rig.target.writes.load(Ordering::SeqCst), 1);
    let held = rig.target.contents(&ns("t", "orders")).await.len();
    assert_eq!(held as u64, BATCH);
    let later = rig.drain_copy_events(Duration::from_millis(200)).await;
    assert_eq!(later, [], "the failure is the last event of the run");
}

#[tokio::test]
async fn source_reconnect_during_the_next_read_writes_no_second_batch() {
    let mut rig = Rig::new(seeded(MANY), empty()).await;
    let reconnect = Command::Connect(rig.source_cfg.clone());
    rig.source
        .arm_at(&rig.h.tx, Point::DuringScan, 2, vec![reconnect]);
    rig.send(CopyCommand::Start {
        run: 1,
        request: rig.request(one_item(), ConflictPolicy::Skip),
    });
    let events = rig.until_terminal(1).await;
    assert_superseded_after_first_batch(&mut rig, &events).await;
}

#[tokio::test]
async fn target_disconnect_during_the_existing_ids_read_writes_no_second_batch() {
    let mut rig = Rig::new(seeded(MANY), empty()).await;
    let close = Command::Disconnect {
        session: rig.target_cfg.id,
    };
    rig.target
        .arm_at(&rig.h.tx, Point::DuringFind, 2, vec![close]);
    rig.send(CopyCommand::Start {
        run: 1,
        request: rig.request(one_item(), ConflictPolicy::Skip),
    });
    let events = rig.until_terminal(1).await;
    assert_superseded_after_first_batch(&mut rig, &events).await;
}

#[tokio::test]
async fn connecting_an_unrelated_session_mid_run_does_not_cancel_the_run() {
    let mut rig = Rig::new(seeded(MANY), empty()).await;
    let unrelated = Command::Connect(rig.other_cfg.clone());
    rig.source
        .arm_at(&rig.h.tx, Point::DuringScan, 2, vec![unrelated]);
    rig.send(CopyCommand::Start {
        run: 1,
        request: rig.request(one_item(), ConflictPolicy::Skip),
    });
    let events = rig.until_terminal(1).await;
    assert_eq!(
        events.last(),
        Some(&CopyEvent::Done {
            run: 1,
            cancelled: false
        })
    );
    let held = rig.target.contents(&ns("t", "orders")).await.len();
    assert_eq!(held as i32, MANY);
}

#[tokio::test]
async fn target_locked_mid_run_stops_with_exact_written_count() {
    let mut rig = Rig::new(seeded(MANY), empty()).await;
    let lock = Command::LockReadOnly {
        connection: rig.target_cfg.id,
    };
    rig.target.arm(&rig.h.tx, 1, vec![lock]);
    rig.send(CopyCommand::Start {
        run: 1,
        request: rig.request(one_item(), ConflictPolicy::Skip),
    });
    let events = rig.until_terminal(1).await;
    let reports = item_reports(&events);
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].progress.written, BATCH);
    assert_eq!(reports[0].outcome, CopyOutcome::Failed(CoreError::ReadOnly));
    assert_eq!(
        events.last(),
        Some(&CopyEvent::Failed {
            run: 1,
            error: CoreError::ReadOnly
        })
    );
    let written = rig.target.contents(&ns("t", "orders")).await.len();
    assert_eq!(written as u64, BATCH);
}

#[tokio::test]
async fn source_reconnect_mid_run_fails_with_written_count_and_no_further_writes() {
    let mut rig = Rig::new(seeded(MANY), empty()).await;
    let reconnect = Command::Connect(rig.source_cfg.clone());
    rig.target.arm(&rig.h.tx, 1, vec![reconnect]);
    rig.send(CopyCommand::Start {
        run: 1,
        request: rig.request(one_item(), ConflictPolicy::Skip),
    });
    let events = rig.until_terminal(1).await;
    let reports = item_reports(&events);
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].progress.written, BATCH);
    assert_eq!(reports[0].outcome, CopyOutcome::Cancelled);
    assert_eq!(
        events.last(),
        Some(&CopyEvent::Failed {
            run: 1,
            error: CoreError::Superseded
        })
    );
    assert_eq!(rig.target.writes.load(Ordering::SeqCst), 1);
    assert_eq!(
        rig.target.contents(&ns("t", "orders")).await.len() as u64,
        BATCH
    );
}

#[tokio::test]
async fn closed_session_emits_nothing_more() {
    let mut rig = Rig::new(seeded(MANY), empty()).await;
    let close = Command::Disconnect {
        session: rig.source_cfg.id,
    };
    rig.target.arm(&rig.h.tx, 1, vec![close]);
    rig.send(CopyCommand::Start {
        run: 1,
        request: rig.request(one_item(), ConflictPolicy::Skip),
    });
    let events = rig.until_terminal(1).await;
    assert_eq!(
        events.last(),
        Some(&CopyEvent::Failed {
            run: 1,
            error: CoreError::Superseded
        })
    );
    let later = rig.drain_copy_events(Duration::from_millis(200)).await;
    assert_eq!(later, [], "the failure is the last event of the run");
    assert_eq!(rig.target.writes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn second_start_while_running_is_refused() {
    let mut rig = Rig::new(seeded(MANY), empty()).await;
    let second = Command::Copy(CopyCommand::Start {
        run: 2,
        request: rig.request(one_item(), ConflictPolicy::Skip),
    });
    let preview = Command::Copy(CopyCommand::Preview {
        run: 3,
        request: rig.request(one_item(), ConflictPolicy::Skip),
    });
    rig.target.arm(&rig.h.tx, 1, vec![second, preview]);
    rig.send(CopyCommand::Start {
        run: 1,
        request: rig.request(one_item(), ConflictPolicy::Skip),
    });
    let events = rig.until_terminal(1).await;
    for run in [2, 3] {
        let refusal = CopyEvent::Failed {
            run,
            error: CoreError::CopyRefused(CopyRefusal::AlreadyRunning),
        };
        assert!(events.contains(&refusal), "run {run} is refused");
    }
    assert_eq!(
        events.last(),
        Some(&CopyEvent::Done {
            run: 1,
            cancelled: false
        })
    );
    assert_eq!(
        rig.target.contents(&ns("t", "orders")).await.len() as i32,
        MANY
    );
}

#[tokio::test]
async fn cancel_emits_done_cancelled_with_exact_counts() {
    let mut rig = Rig::new(seeded(MANY), empty()).await;
    rig.target.arm(
        &rig.h.tx,
        1,
        vec![
            Command::Copy(CopyCommand::Cancel { run: 99 }),
            Command::Copy(CopyCommand::Cancel { run: 1 }),
        ],
    );
    rig.send(CopyCommand::Start {
        run: 1,
        request: rig.request(one_item(), ConflictPolicy::Skip),
    });
    let events = rig.until_terminal(1).await;
    let reports = item_reports(&events);
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].progress.written, BATCH);
    assert_eq!(reports[0].outcome, CopyOutcome::Cancelled);
    assert_eq!(
        events.last(),
        Some(&CopyEvent::Done {
            run: 1,
            cancelled: true
        })
    );
    assert_eq!(
        rig.target.contents(&ns("t", "orders")).await.len() as u64,
        BATCH
    );
}
