//! Shell run history through the runtime: recording in the run's completion, and the
//! `ShellHistory` / `ClearShellHistory` / `CopyShellHistory` commands. A real history file and
//! the fake `mongosh`; every wait is on an event, never on time.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use uuid::Uuid;

use super::*;
use crate::history::{HistoryDb, ShellHistoryEntry, ShellRunStatus};
use crate::shell::Outcome;
use crate::shell::fake::{Behaviour, FakeMongosh};
use crate::test_support::InMemoryStore;

const WAIT: Duration = Duration::from_secs(10);

struct StoreConnector;

#[async_trait]
impl Connector for StoreConnector {
    async fn connect(
        &self,
        _cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        Ok(Arc::new(InMemoryStore::new()))
    }
}

struct Fixture {
    h: RuntimeHandle,
    id: SessionId,
    fake: FakeMongosh,
    /// Holds the history file; the runtime gets a path in it only when history is on.
    dir: tempfile::TempDir,
    history_on: bool,
}

impl Fixture {
    async fn new(history_on: bool, read_only: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = RuntimeConfig {
            history_path: history_on.then(|| dir.path().join("history.sqlite")),
            ..RuntimeConfig::default()
        };
        let repaint: Repaint = Arc::new(|| {});
        let mut h =
            spawn_runtime_with_connector(None, None, Arc::new(StoreConnector), repaint, config);
        let mut cfg = ConnectionConfig::new("t", "mongodb://localhost:27017/app");
        cfg.read_only = read_only;
        let id = cfg.id;
        h.tx.send(Command::Connect(cfg)).unwrap();
        assert!(matches!(next(&mut h).await, Event::Connected { .. }));
        Self {
            h,
            id,
            fake: FakeMongosh::new(Behaviour::Repl),
            dir,
            history_on,
        }
    }

    async fn with_history() -> Self {
        Self::new(true, false).await
    }

    fn path(&self) -> PathBuf {
        self.dir.path().join("history.sqlite")
    }

    /// The stored entries of `connection`, read straight from the file.
    fn stored(&self, connection: Uuid) -> Vec<ShellHistoryEntry> {
        assert!(self.history_on);
        let (db, _) = HistoryDb::open(&self.path()).unwrap();
        db.shell_history(connection).unwrap()
    }

    fn run(&self, run_id: u64, script: &str) {
        self.h
            .tx
            .send(Command::ShellRun {
                session: self.id,
                run_id,
                exe: self.fake.exe.clone(),
                script: script.into(),
            })
            .unwrap();
    }

    fn cancel(&self, run_id: u64) {
        let session = self.id;
        self.h
            .tx
            .send(Command::ShellCancel { session, run_id })
            .unwrap();
    }

    /// The outcome of the run, once its `ShellFinished` arrives.
    async fn finished(&mut self) -> Outcome {
        loop {
            match next(&mut self.h).await {
                Event::ShellFinished { outcome, .. } => return outcome,
                Event::ShellOutput { .. } => {}
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    /// Waits for the first output of a run, so the script is known to be running.
    async fn until_running(&mut self) {
        match next(&mut self.h).await {
            Event::ShellOutput { .. } => {}
            other => panic!("expected output, got {other:?}"),
        }
    }

    async fn history_of(&mut self, session: SessionId) -> Vec<ShellHistoryEntry> {
        self.h.tx.send(Command::ShellHistory { session }).unwrap();
        self.answer(session).await
    }

    async fn answer(&mut self, wanted: SessionId) -> Vec<ShellHistoryEntry> {
        match next(&mut self.h).await {
            Event::ShellHistory { session, entries } => {
                assert_eq!(session, wanted);
                entries
            }
            other => panic!("expected ShellHistory, got {other:?}"),
        }
    }
}

/// The next event, leaving out the spontaneous sampler and profiler reports a connected session
/// sends on its own.
async fn next(h: &mut RuntimeHandle) -> Event {
    loop {
        let event = tokio::time::timeout(WAIT, h.rx.recv())
            .await
            .expect("event in time")
            .expect("channel open");
        if !matches!(event, Event::Sampler { .. } | Event::Profiler { .. }) {
            return event;
        }
    }
}

#[test]
fn the_history_commands_are_classified() {
    let (session, other) = (Uuid::new_v4(), Uuid::new_v4());
    let read = Command::ShellHistory { session };
    let clear = Command::ClearShellHistory { session };
    let copy = Command::CopyShellHistory {
        from: session,
        to: other,
    };
    for command in [&read, &clear, &copy] {
        assert!(!command.is_write());
        assert_eq!(command.request(), None);
    }
    assert_eq!(read.context(), "shell_history");
    assert_eq!(clear.context(), "clear_shell_history");
    assert_eq!(copy.context(), "copy_shell_history");
    assert_eq!(read.session(), Some(session));
    assert_eq!(clear.session(), Some(session));
    assert_eq!(copy.session(), None);
    let answer = Event::ShellHistory {
        session,
        entries: Vec::new(),
    };
    assert_eq!(answer.session(), Some(session));
}

#[tokio::test]
async fn a_finished_run_is_recorded_before_shell_finished_is_sent() {
    let mut f = Fixture::with_history().await;
    f.run(1, "hello shell");
    assert_eq!(f.finished().await, Outcome::Exited(0));
    // Read the instant `ShellFinished` was seen: the entry must already be there.
    let entries = f.stored(f.id);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].script, "hello shell");
    assert_eq!(entries[0].status, ShellRunStatus::Succeeded);
}

#[tokio::test]
async fn a_refused_run_is_not_recorded() {
    let mut f = Fixture::new(true, true).await;
    f.run(1, "hello shell");
    match next(&mut f.h).await {
        Event::ShellRefused { reason, .. } => assert_eq!(reason, ShellRefusal::ReadOnly),
        other => panic!("expected ShellRefused, got {other:?}"),
    }
    assert!(f.history_of(f.id).await.is_empty());
    assert!(f.stored(f.id).is_empty());
}

#[tokio::test]
async fn a_cancelled_run_is_recorded_as_cancelled() {
    let mut f = Fixture::with_history().await;
    f.run(1, "started\nhang");
    f.until_running().await;
    f.cancel(1);
    assert_eq!(f.finished().await, Outcome::Cancelled);
    let entries = f.stored(f.id);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, ShellRunStatus::Cancelled);
    assert_eq!(entries[0].script, "started\nhang");
}

#[tokio::test]
async fn history_off_records_nothing_and_reads_empty() {
    let mut f = Fixture::new(false, false).await;
    f.run(1, "hello shell");
    assert_eq!(f.finished().await, Outcome::Exited(0));
    assert!(f.history_of(f.id).await.is_empty());
    f.h.tx
        .send(Command::ClearShellHistory { session: f.id })
        .unwrap();
    assert!(f.answer(f.id).await.is_empty());
}

#[tokio::test]
async fn shell_history_command_answers_with_the_connections_entries() {
    let mut f = Fixture::with_history().await;
    f.run(1, "first");
    f.finished().await;
    f.run(2, "throw boom");
    assert_eq!(f.finished().await, Outcome::ScriptErrors);
    let entries = f.history_of(f.id).await;
    let seen: Vec<(&str, ShellRunStatus)> = entries
        .iter()
        .map(|e| (e.script.as_str(), e.status))
        .collect();
    // Newest first.
    assert_eq!(
        seen,
        [
            ("throw boom", ShellRunStatus::ScriptErrors),
            ("first", ShellRunStatus::Succeeded)
        ]
    );
    assert!(f.history_of(Uuid::new_v4()).await.is_empty());
}

#[tokio::test]
async fn clear_shell_history_answers_with_an_empty_list() {
    let mut f = Fixture::with_history().await;
    f.run(1, "first");
    f.finished().await;
    assert_eq!(f.history_of(f.id).await.len(), 1);
    f.h.tx
        .send(Command::ClearShellHistory { session: f.id })
        .unwrap();
    assert!(f.answer(f.id).await.is_empty());
    assert!(f.stored(f.id).is_empty());
}

#[tokio::test]
async fn copy_shell_history_copies_between_connection_ids() {
    let mut f = Fixture::with_history().await;
    f.run(1, "first");
    f.finished().await;
    let to = Uuid::new_v4();
    f.h.tx
        .send(Command::CopyShellHistory { from: f.id, to })
        .unwrap();
    // The copy has no answer and runs in the background: ask until it shows. Each ask is a
    // round trip on an event, not a sleep, and the loop is bounded.
    let mut copied = Vec::new();
    for _ in 0..10_000 {
        copied = f.history_of(to).await;
        if !copied.is_empty() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(copied.len(), 1);
    assert_eq!(copied[0].script, "first");
    assert_eq!(f.stored(f.id).len(), 1, "the source keeps its entries");
}

#[tokio::test]
async fn a_run_finishing_after_its_connection_was_purged_is_not_recorded() {
    let mut f = Fixture::with_history().await;
    f.run(1, "started\nhang");
    f.until_running().await;
    f.h.tx
        .send(Command::PurgeHistory { connection: f.id })
        .unwrap();
    f.cancel(1);
    // The run did end (a live connection would have recorded it as cancelled).
    assert_eq!(f.finished().await, Outcome::Cancelled);
    assert!(f.stored(f.id).is_empty());
}

#[test]
fn an_answer_never_debug_prints_a_script() {
    let entry = ShellHistoryEntry {
        id: 1,
        ran_at_secs: 5,
        script: "db.secret.find('hunter2')".into(),
        status: ShellRunStatus::Succeeded,
    };
    let event = Event::ShellHistory {
        session: Uuid::new_v4(),
        entries: vec![entry],
    };
    assert!(!format!("{event:?}").contains("hunter2"));
}
