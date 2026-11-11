//! Shell run history through the runtime: recording after the run's completion, and the
//! `ShellHistory` / `ClearShellHistory` / `CopyShellHistory` commands, all on one ordered queue. A
//! real history file and the fake `mongosh`; every wait is on an event, never on time.

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
/// Shorter than the history file's busy timeout (5 s): an event that had to wait for a lock the
/// test holds arrives later than this.
const BEFORE_BUSY_TIMEOUT: Duration = Duration::from_secs(4);

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

    /// Whether the run's record was stored, from its `ShellHistoryChanged` (the next event).
    async fn changed(&mut self) -> bool {
        match next(&mut self.h).await {
            Event::ShellHistoryChanged { session, stored } => {
                assert_eq!(session, self.id);
                stored
            }
            other => panic!("expected ShellHistoryChanged, got {other:?}"),
        }
    }

    /// The number of stored runs, read with a plain connection that waits for no lock.
    fn stored_count(&self) -> i64 {
        let conn = rusqlite::Connection::open(self.path()).unwrap();
        conn.query_row("SELECT COUNT(*) FROM shell_history", [], |r| r.get(0))
            .unwrap()
    }

    async fn history_of(&mut self, session: SessionId) -> Vec<ShellHistoryEntry> {
        self.h.tx.send(Command::ShellHistory { session }).unwrap();
        self.answer(session).await
    }

    /// The context and error of the next event, which must be a `Failed` for `wanted`.
    async fn failure(&mut self, wanted: SessionId) -> (&'static str, CoreError) {
        match next(&mut self.h).await {
            Event::Failed {
                session,
                context,
                error,
            } => {
                assert_eq!(session, wanted);
                (context, error)
            }
            other => panic!("expected Failed, got {other:?}"),
        }
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
    let changed = Event::ShellHistoryChanged {
        session,
        stored: true,
    };
    assert_eq!(changed.session(), Some(session));
}

/// The record waits for the history file; the run's end does not. Another connection holds the
/// file's write lock while the run finishes: `ShellFinished` still arrives at once, and the
/// record follows once the lock is released.
#[tokio::test]
async fn shell_finished_is_not_delayed_by_the_record() {
    let mut f = Fixture::with_history().await;
    let holder = rusqlite::Connection::open(f.path()).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    f.run(1, "hello shell");
    let outcome = tokio::time::timeout(BEFORE_BUSY_TIMEOUT, f.finished())
        .await
        .expect("ShellFinished while the history file is locked");
    assert_eq!(outcome, Outcome::Exited(0));
    assert_eq!(f.stored_count(), 0, "the record still waits for the lock");
    holder.execute_batch("COMMIT").unwrap();
    assert!(f.changed().await, "recorded once the lock is free");
    let entries = f.stored(f.id);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].script, "hello shell");
    assert_eq!(entries[0].status, ShellRunStatus::Succeeded);
}

#[tokio::test]
async fn a_read_sent_after_shell_history_changed_sees_the_row() {
    let mut f = Fixture::with_history().await;
    f.run(1, "hello shell");
    assert_eq!(f.finished().await, Outcome::Exited(0));
    assert!(f.changed().await);
    let entries = f.history_of(f.id).await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].script, "hello shell");
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
    assert!(f.changed().await);
    let entries = f.stored(f.id);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, ShellRunStatus::Cancelled);
    assert_eq!(entries[0].script, "started\nhang");
}

#[tokio::test]
async fn history_off_records_nothing_and_reads_as_unavailable() {
    let mut f = Fixture::new(false, false).await;
    f.run(1, "hello shell");
    assert_eq!(f.finished().await, Outcome::Exited(0));
    f.h.tx
        .send(Command::ShellHistory { session: f.id })
        .unwrap();
    assert_eq!(
        f.failure(f.id).await,
        ("shell_history", CoreError::NotFound)
    );
    f.h.tx
        .send(Command::ClearShellHistory { session: f.id })
        .unwrap();
    assert_eq!(
        f.failure(f.id).await,
        ("clear_shell_history", CoreError::NotFound)
    );
}

#[tokio::test]
async fn shell_history_command_answers_with_the_connections_entries() {
    let mut f = Fixture::with_history().await;
    f.run(1, "first");
    f.finished().await;
    assert!(f.changed().await);
    f.run(2, "throw boom");
    assert_eq!(f.finished().await, Outcome::ScriptErrors);
    assert!(f.changed().await);
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
    assert!(f.changed().await);
    assert_eq!(f.history_of(f.id).await.len(), 1);
    f.h.tx
        .send(Command::ClearShellHistory { session: f.id })
        .unwrap();
    assert!(f.answer(f.id).await.is_empty());
    assert!(f.stored(f.id).is_empty());
}

/// The UI's read after a confirmed Clear (a run finishing meanwhile triggers one) is sent after
/// the clear, so it can never bring the cleared list back.
#[tokio::test]
async fn a_read_sent_right_after_a_clear_answers_after_it() {
    let mut f = Fixture::with_history().await;
    f.run(1, "first");
    f.finished().await;
    assert!(f.changed().await);
    let session = f.id;
    f.h.tx.send(Command::ClearShellHistory { session }).unwrap();
    f.h.tx.send(Command::ShellHistory { session }).unwrap();
    assert!(f.answer(session).await.is_empty(), "the clear's answer");
    assert!(f.answer(session).await.is_empty(), "the read's answer");
}

#[tokio::test]
async fn copy_shell_history_copies_between_connection_ids() {
    let mut f = Fixture::with_history().await;
    f.run(1, "first");
    f.finished().await;
    assert!(f.changed().await);
    let to = Uuid::new_v4();
    f.h.tx
        .send(Command::CopyShellHistory { from: f.id, to })
        .unwrap();
    // The copy has no answer, but it is queued before the read, which therefore sees it.
    let copied = f.history_of(to).await;
    assert_eq!(copied.len(), 1);
    assert_eq!(copied[0].script, "first");
    assert_eq!(f.stored(f.id).len(), 1, "the source keeps its entries");
}

#[tokio::test]
async fn a_run_finishing_after_its_connection_was_purged_is_not_recorded() {
    let mut f = Fixture::with_history().await;
    // An earlier run leaves a row, so the purge's completion can be seen: the row is gone.
    f.run(1, "earlier");
    f.finished().await;
    assert!(f.changed().await);
    assert_eq!(f.stored(f.id).len(), 1);
    f.run(2, "started\nhang");
    f.until_running().await;
    f.h.tx
        .send(Command::PurgeHistory { connection: f.id })
        .unwrap();
    // The purge has no answer and runs in the background: wait (bounded, no sleep) until it
    // has deleted the row. Only then does the run finish, so nothing but the guard can keep
    // its record out.
    let mut polls = 0;
    while !f.stored(f.id).is_empty() {
        polls += 1;
        assert!(polls < 100_000, "the purge never completed");
        tokio::task::yield_now().await;
    }
    f.cancel(2);
    // The run did end (a live connection would have recorded it as cancelled).
    assert_eq!(f.finished().await, Outcome::Cancelled);
    // Until the store says what it skipped, its answer is the same as for a stored record.
    f.changed().await;
    assert!(f.history_of(f.id).await.is_empty());
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
