//! The one ordered queue of shell-history work of a runtime.
//!
//! Every shell-history job (a finished run's record, `ShellHistory`, `ClearShellHistory`,
//! `CopyShellHistory`) is pushed on one unbounded channel and run by a single task, one job at a
//! time and in push order, each on the blocking pool. So a read sent after a clear never returns
//! the cleared rows, two reads answer in send order, and a record pushed after a clear is kept.
//! Pushing never waits, so a run's completion is never held up by the history file.
//!
//! A job's failure is logged with ids only (neither the script nor an error that may quote it
//! belongs in a log); a panic inside a job is caught and ends only that job.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use uuid::Uuid;

use crate::clock;
use crate::error::{CoreError, Result};
use crate::history::{HistoryDb, Recorded, ShellHistoryEntry, ShellRunStatus};
use crate::shell::Outcome;

use super::indexes::blocking;
use super::{Event, Repaint, SessionId, handlers};

/// One piece of shell-history work.
pub(super) enum Job {
    /// Remember a finished run of `connection` (the session id), if its script executed.
    Record {
        connection: Uuid,
        script: String,
        outcome: Outcome,
    },
    /// A job answered with the session's list (or a `Failed` in its context).
    Answer { session: SessionId, kind: Answer },
    /// Copy the entries of `from` to `to`; no answer.
    Copy { from: Uuid, to: Uuid },
}

/// What an answered job does; it also names the job's failure context, so the two never
/// disagree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Answer {
    /// Answer with the session's entries (`ShellHistory`).
    Read,
    /// Forget the session's entries, then answer with the (empty) list (`ClearShellHistory`).
    Clear,
    /// A read that panics inside the job (tests only).
    #[cfg(test)]
    PanickingRead,
}

impl Answer {
    /// The `Failed` context: the i18n key suffix of the command this answers.
    pub(super) fn context(self) -> &'static str {
        match self {
            Self::Read => "shell_history",
            Self::Clear => "clear_shell_history",
            #[cfg(test)]
            Self::PanickingRead => "shell_history",
        }
    }

    fn entries(self, history: &HistoryDb, session: SessionId) -> Result<Vec<ShellHistoryEntry>> {
        match self {
            Self::Read => history.shell_history(session),
            Self::Clear => history.clear_shell_history(session).map(|()| Vec::new()),
            #[cfg(test)]
            Self::PanickingRead => panic!("injected history job panic"),
        }
    }
}

impl Job {
    pub(super) fn read(session: SessionId) -> Self {
        Self::Answer {
            session,
            kind: Answer::Read,
        }
    }

    pub(super) fn clear(session: SessionId) -> Self {
        Self::Answer {
            session,
            kind: Answer::Clear,
        }
    }

    /// The session and failure context of a job that is answered, so even a job that panicked
    /// gets its `Failed` answer.
    fn answered(&self) -> Option<(SessionId, &'static str)> {
        match self {
            Self::Answer { session, kind } => Some((*session, kind.context())),
            Self::Record { .. } | Self::Copy { .. } => None,
        }
    }
}

/// The sending side of the queue; cheap to clone.
#[derive(Clone)]
pub(crate) struct ShellHistoryQueue {
    jobs: UnboundedSender<Job>,
}

impl ShellHistoryQueue {
    /// Starts the queue's one worker on the current Tokio runtime. It ends when every sender is
    /// gone.
    pub(super) fn spawn(
        history: Option<Arc<HistoryDb>>,
        events: UnboundedSender<Event>,
        repaint: Repaint,
    ) -> Self {
        let (jobs, mut queue) = unbounded_channel::<Job>();
        tokio::runtime::Handle::current().spawn(async move {
            while let Some(job) = queue.recv().await {
                let Some(event) = run(history.clone(), job).await else {
                    continue;
                };
                if events.send(event).is_ok() {
                    repaint();
                }
            }
        });
        Self { jobs }
    }

    /// A queue nobody drains, for tests that build `Shared` by hand and never touch the history.
    #[cfg(test)]
    pub(super) fn detached() -> Self {
        let (jobs, _) = unbounded_channel();
        Self { jobs }
    }

    /// Queues `job` behind every job pushed before it. Never waits.
    pub(super) fn push(&self, job: Job) {
        if self.jobs.send(job).is_err() {
            log::warn!("the shell history queue is closed; a history job was dropped");
        }
    }
}

/// Runs one job to its end on the blocking pool; its answer, if it has one.
async fn run(history: Option<Arc<HistoryDb>>, job: Job) -> Option<Event> {
    let answered = job.answered();
    let caught = blocking(move || {
        catch_unwind(AssertUnwindSafe(|| perform(history.as_deref(), job)))
            .map_err(|_| CoreError::Database("internal error".into()))
    })
    .await;
    match caught {
        Ok(event) => event,
        Err(error) => {
            log::warn!("a shell history job panicked");
            answered.map(|(session, context)| failed(session, context, error))
        }
    }
}

fn perform(history: Option<&HistoryDb>, job: Job) -> Option<Event> {
    match job {
        Job::Record {
            connection,
            script,
            outcome,
        } => record(history?, connection, &script, outcome),
        Job::Answer { session, kind } => Some(answer(history, session, kind)),
        Job::Copy { from, to } => {
            // Ids only: the error text is not logged.
            if history?.copy_shell_history(from, to).is_err() {
                log::warn!("copying the shell history of {from} to {to} failed");
            }
            None
        }
    }
}

/// Remembers a finished run whose script executed. `ShellHistoryChanged` once it was stored, or
/// with `stored: false` when it was too long to store; nothing when nothing was recorded (the
/// script never executed, the connection was deleted) or the write failed.
fn record(history: &HistoryDb, connection: Uuid, script: &str, outcome: Outcome) -> Option<Event> {
    let status = ShellRunStatus::from_outcome(&outcome)?;
    let changed = |stored| Event::ShellHistoryChanged {
        session: connection,
        stored,
    };
    match history.record_shell_run(connection, script, status, clock::now_secs()) {
        Ok(Recorded::Stored) => Some(changed(true)),
        Ok(Recorded::TooLarge) => Some(changed(false)),
        Ok(Recorded::Skipped) => None,
        Err(_) => {
            log::warn!("recording a shell run of {connection} in the history failed");
            None
        }
    }
}

/// The list, or a `Failed` with the job's context: without history too (never an empty list,
/// which would read as "no scripts yet"). The error carries no path or system text.
fn answer(history: Option<&HistoryDb>, session: SessionId, kind: Answer) -> Event {
    let entries = match history {
        None => Err(CoreError::NotFound),
        Some(history) => kind.entries(history, session),
    };
    match entries {
        Ok(entries) => Event::ShellHistory { session, entries },
        Err(error) => failed(session, kind.context(), error),
    }
}

fn failed(session: SessionId, context: &'static str, error: CoreError) -> Event {
    handlers::failure_event(session, context, None, 0, error)
}

#[cfg(test)]
#[path = "shell_history_queue_tests.rs"]
mod tests;
