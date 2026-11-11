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
use crate::history::{HistoryDb, Recorded, ShellRunStatus};
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
    /// Answer with the session's entries.
    Read { session: SessionId },
    /// Forget the session's entries, then answer with the (empty) list.
    Clear { session: SessionId },
    /// Copy the entries of `from` to `to`; no answer.
    Copy { from: Uuid, to: Uuid },
    /// A read that panics inside the job (tests only).
    #[cfg(test)]
    PanickingRead { session: SessionId },
}

impl Job {
    /// The session and failure context of a job that is answered, so even a job that panicked
    /// gets its `Failed` answer.
    fn answered(&self) -> Option<(SessionId, &'static str)> {
        match self {
            Self::Read { session } => Some((*session, "shell_history")),
            Self::Clear { session } => Some((*session, "clear_shell_history")),
            #[cfg(test)]
            Self::PanickingRead { session } => Some((*session, "shell_history")),
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
        Job::Read { session } => Some(answer(
            session,
            "shell_history",
            history.map_or(Err(CoreError::NotFound), |h| h.shell_history(session)),
        )),
        Job::Clear { session } => Some(answer(
            session,
            "clear_shell_history",
            history.map_or(Err(CoreError::NotFound), |h| {
                h.clear_shell_history(session).map(|()| Vec::new())
            }),
        )),
        Job::Copy { from, to } => {
            // Ids only: the error text is not logged.
            if history?.copy_shell_history(from, to).is_err() {
                log::warn!("copying the shell history of {from} to {to} failed");
            }
            None
        }
        #[cfg(test)]
        Job::PanickingRead { .. } => panic!("injected history job panic"),
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

/// The list, or a `Failed` with the job's context (never an empty list, which would read as "no
/// scripts yet"); the error carries no path or system text.
fn answer(
    session: SessionId,
    context: &'static str,
    entries: Result<Vec<crate::history::ShellHistoryEntry>>,
) -> Event {
    match entries {
        Ok(entries) => Event::ShellHistory { session, entries },
        Err(error) => failed(session, context, error),
    }
}

fn failed(session: SessionId, context: &'static str, error: CoreError) -> Event {
    handlers::failure_event(session, context, None, 0, error)
}

#[cfg(test)]
#[path = "shell_history_queue_tests.rs"]
mod tests;
