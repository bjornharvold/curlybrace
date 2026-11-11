//! The shell-history queue on its own: one worker, jobs in push order, a panic ends one job only.
//! A real in-memory history; every wait is on an event, never on time.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use uuid::Uuid;

use super::*;
use crate::history::{HistoryDb, ShellRunStatus};

const WAIT: Duration = Duration::from_secs(10);

fn queue() -> (ShellHistoryQueue, UnboundedReceiver<Event>, Arc<HistoryDb>) {
    let history = Arc::new(HistoryDb::open_in_memory().unwrap());
    let (events, rx) = unbounded_channel();
    let repaint: Repaint = Arc::new(|| {});
    let queue = ShellHistoryQueue::spawn(Some(history.clone()), events, repaint);
    (queue, rx, history)
}

async fn next(rx: &mut UnboundedReceiver<Event>) -> Event {
    tokio::time::timeout(WAIT, rx.recv())
        .await
        .expect("event in time")
        .expect("channel open")
}

fn record(connection: Uuid, script: &str) -> Job {
    Job::Record {
        connection,
        script: script.into(),
        outcome: Outcome::Exited(0),
    }
}

fn scripts(event: Event) -> Vec<String> {
    match event {
        Event::ShellHistory { entries, .. } => entries.into_iter().map(|e| e.script).collect(),
        other => panic!("expected ShellHistory, got {other:?}"),
    }
}

/// A connection with `n` stored runs, written straight to the store.
fn seeded(history: &HistoryDb, n: usize) -> Uuid {
    let session = Uuid::new_v4();
    for i in 0..n {
        let script = format!("db.run{i}.find()");
        history
            .record_shell_run(session, &script, ShellRunStatus::Succeeded, 1_000)
            .unwrap();
    }
    session
}

// ---- Order: each job runs after every job pushed before it ----

#[tokio::test]
async fn a_read_pushed_right_after_a_clear_never_returns_the_cleared_entries() {
    let (queue, mut rx, history) = queue();
    for _ in 0..20 {
        let session = seeded(&history, 50);
        queue.push(Job::Clear { session });
        queue.push(Job::Read { session });
        assert!(
            scripts(next(&mut rx).await).is_empty(),
            "the clear's answer"
        );
        assert!(scripts(next(&mut rx).await).is_empty(), "the read's answer");
    }
}

#[tokio::test]
async fn reads_answer_in_send_order() {
    let (queue, mut rx, history) = queue();
    // The first read has far more rows to read than the second, so only order keeps them in order.
    let (big, small) = (seeded(&history, 200), seeded(&history, 1));
    for _ in 0..20 {
        queue.push(Job::Read { session: big });
        queue.push(Job::Read { session: small });
        assert_eq!(next(&mut rx).await.session(), Some(big));
        assert_eq!(next(&mut rx).await.session(), Some(small));
    }
}

#[tokio::test]
async fn a_record_pushed_after_a_clear_is_not_erased_by_it() {
    let (queue, mut rx, history) = queue();
    for _ in 0..20 {
        let session = seeded(&history, 50);
        queue.push(Job::Clear { session });
        queue.push(record(session, "db.after.find()"));
        queue.push(Job::Read { session });
        assert!(
            scripts(next(&mut rx).await).is_empty(),
            "the clear's answer"
        );
        assert!(matches!(
            next(&mut rx).await,
            Event::ShellHistoryChanged { stored: true, .. }
        ));
        assert_eq!(scripts(next(&mut rx).await), ["db.after.find()"]);
    }
}

// ---- One job's failure ends that job only ----

#[tokio::test]
async fn a_panicking_job_is_answered_and_the_worker_goes_on() {
    let (queue, mut rx, _) = queue();
    let session = Uuid::new_v4();
    queue.push(Job::PanickingRead { session });
    queue.push(Job::Read { session });
    assert!(matches!(
        next(&mut rx).await,
        Event::Failed {
            context: "shell_history",
            ..
        }
    ));
    assert!(scripts(next(&mut rx).await).is_empty());
}

#[tokio::test]
async fn a_record_answers_with_shell_history_changed() {
    let (queue, mut rx, history) = queue();
    let session = Uuid::new_v4();
    queue.push(record(session, "db.a.find()"));
    assert_eq!(
        next(&mut rx).await,
        Event::ShellHistoryChanged {
            session,
            stored: true
        }
    );
    assert_eq!(history.shell_history(session).unwrap().len(), 1);
}

#[tokio::test]
async fn a_run_that_never_executed_is_not_recorded_and_not_announced() {
    let (queue, mut rx, _) = queue();
    let session = Uuid::new_v4();
    queue.push(Job::Record {
        connection: session,
        script: "db.a.find()".into(),
        outcome: Outcome::ConnectFailed,
    });
    queue.push(Job::Read { session });
    assert!(scripts(next(&mut rx).await).is_empty());
}
