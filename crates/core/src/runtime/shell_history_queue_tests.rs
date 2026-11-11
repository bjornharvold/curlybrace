//! The shell-history queue on its own: one worker, jobs in push order, a panic ends one job only.
//! A real in-memory history; every wait is on an event, never on time.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use uuid::Uuid;

use super::*;
use crate::history::HistoryDb;

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
