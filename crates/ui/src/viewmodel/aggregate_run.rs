//! The Aggregate draft's confirmed write run: whether Run is offered, the typed-confirmation
//! dialog, and the outcome of the last run. Per session, kept through a lost connection like the
//! draft. Every op returns a new state; nothing here talks to a server.
use super::aggregate::AggregateState;
pub use super::aggregate_run_availability::{RunAvailability, RunDisabled, RunEnv};
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::RunTarget;
use curlybrace_core::store::Namespace;

/// The stage whose failure can leave part of its result in the target (it is not atomic).
const MERGE_STAGE: &str = "$merge";

/// The document count of the run's target, as the dialog knows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CountState {
    /// Asked, not answered yet.
    #[default]
    Loading,
    Known(u64),
    /// The count could not be taken (the ask failed).
    Unavailable,
}

/// What the person is asked to type, and what the run is about. Closed by anything that changes
/// the pipeline, the collection, the session or the connection.
#[derive(Clone, Debug, PartialEq)]
pub struct RunDialog {
    /// Where the last stage writes; `db: None` is the source database.
    pub target: RunTarget,
    /// `"$out"` or `"$merge"`.
    pub stage: String,
    /// The collection the pipeline reads: the run is refused when another one is open.
    pub source: Namespace,
    /// What the person typed to confirm.
    pub typed: String,
    /// The target's document count now: loading, known or not available.
    pub count: CountState,
}

impl RunDialog {
    /// The namespace the run writes to.
    pub fn target_ns(&self) -> Namespace {
        Namespace::new(
            self.target
                .db
                .clone()
                .unwrap_or_else(|| self.source.db.clone()),
            self.target.collection.clone(),
        )
    }

    /// What must be typed: the collection, or `db.collection` when the target is in a database
    /// other than the source's or its name contains a dot (the bare name would be ambiguous).
    pub fn required_text(&self) -> String {
        let target = self.target_ns();
        match target.db != self.source.db || target.collection.contains('.') {
            true => format!("{}.{}", target.db, target.collection),
            false => target.collection,
        }
    }

    /// Whether `typed` is the required text: both trimmed, case-sensitive, always. The one
    /// comparison of the view (Enter, the Run button) and the reducer, so they cannot disagree
    /// about a name with a space at its end.
    pub fn confirms(&self, typed: &str) -> bool {
        typed.trim() == self.required_text().trim()
    }

    /// Whether what is typed so far is the required text.
    pub fn is_confirmed(&self) -> bool {
        self.confirms(&self.typed)
    }
}

/// How the last run went.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum RunStatus {
    #[default]
    Idle,
    Running,
    /// The run finished; `count` is the target's document count after it, when it could be read.
    Done {
        target: Namespace,
        count: Option<u64>,
    },
    Failed(CoreError),
    /// A `$merge` failed on the server, possibly part-way: the target may hold part of the result.
    FailedPartial(CoreError),
    /// The client gave up waiting or the connection was lost: the server may or may not have
    /// finished the write.
    Unknown,
}

/// The run state of one session's draft.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunUi {
    pub dialog: Option<RunDialog>,
    pub status: RunStatus,
    /// The number of the newest run sent; an answer to another number is stale.
    pub run: u64,
    /// The write stage of the run in flight or last sent (`"$out"` or `"$merge"`).
    pub stage: String,
}

impl RunUi {
    /// The run's write stage, known once it was sent.
    pub fn with_stage(self, stage: String) -> Self {
        Self { stage, ..self }
    }

    /// The status of a run that failed with `error`. Partial is the exception: only a `$merge`
    /// whose error came from the server mid-run (`Database`) may have written part of its
    /// result. A `$out` leaves the target untouched, and every other error (a refusal, a
    /// connection that never opened, an unsupported store, a TLS failure) means nothing was
    /// written: a plain failure.
    pub fn failure(&self, error: CoreError) -> RunStatus {
        let from_the_server = matches!(error, CoreError::Database(_));
        match self.stage == MERGE_STAGE && from_the_server {
            true => RunStatus::FailedPartial(error),
            false => RunStatus::Failed(error),
        }
    }

    /// The dialog is open on `dialog`.
    pub fn opened(self, dialog: RunDialog) -> Self {
        Self {
            dialog: Some(dialog),
            ..self
        }
    }

    /// The dialog is closed; the status stays.
    pub fn dialog_closed(self) -> Self {
        Self {
            dialog: None,
            ..self
        }
    }

    /// The confirmation text changed.
    pub fn typed(self, typed: String) -> Self {
        Self {
            dialog: self.dialog.map(|d| RunDialog { typed, ..d }),
            ..self
        }
    }

    /// The count of `ns` arrived: it fills the dialog when `ns` is its target.
    pub fn count_arrived(self, ns: &Namespace, count: u64) -> Self {
        let dialog = self.dialog.map(|d| match d.target_ns() == *ns {
            true => RunDialog {
                count: CountState::Known(count),
                ..d
            },
            false => d,
        });
        Self { dialog, ..self }
    }

    /// The ask for the count failed: a dialog still waiting for it says it is not available.
    pub fn count_failed(self) -> Self {
        let dialog = self.dialog.map(|d| match d.count {
            CountState::Loading => RunDialog {
                count: CountState::Unavailable,
                ..d
            },
            _ => d,
        });
        Self { dialog, ..self }
    }

    /// Run `run` was sent: the dialog is closed and the run is in flight.
    pub fn started(self, run: u64) -> Self {
        Self {
            dialog: None,
            status: RunStatus::Running,
            run,
            ..self
        }
    }

    /// The answer of run `run`; any other number, or one when no run is in flight, is stale.
    pub fn answered(self, run: u64, status: RunStatus) -> Self {
        match run == self.run && self.status == RunStatus::Running {
            true => Self { status, ..self },
            false => self,
        }
    }

    /// The command of the run in flight failed as a command (not as an outcome).
    pub fn command_failed(self, error: CoreError) -> Self {
        let status = match error {
            CoreError::WriteOutcomeUnknown | CoreError::Unreachable => RunStatus::Unknown,
            error => self.failure(error),
        };
        match self.status == RunStatus::Running {
            true => Self { status, ..self },
            false => self,
        }
    }

    /// The draft now runs against another collection: the dialog closes and a finished run is
    /// forgotten; one in flight and an unknown outcome stay in view.
    pub fn collection_changed(self) -> Self {
        let status = match self.status {
            RunStatus::Done { .. } | RunStatus::Failed(_) => RunStatus::Idle,
            other => other,
        };
        Self {
            dialog: None,
            status,
            ..self
        }
    }

    /// The connection is gone or replaced: a run in flight has an unknown outcome.
    pub fn connection_reset(self) -> Self {
        let status = match self.status {
            RunStatus::Running => RunStatus::Unknown,
            other => other,
        };
        Self {
            dialog: None,
            status,
            ..self
        }
    }

    /// Whether a run is in flight.
    pub fn is_running(&self) -> bool {
        self.status == RunStatus::Running
    }
}

impl AggregateState {
    /// The draft with its run dialog closed (the view the person leaves behind).
    pub fn with_run_dialog_closed(self) -> Self {
        let run = self.run.clone().dialog_closed();
        self.with_run(run)
    }

    /// The draft with `run` as its run state.
    pub fn with_run(self, run: RunUi) -> Self {
        Self { run, ..self }
    }
}

#[cfg(test)]
#[path = "aggregate_run_tests.rs"]
mod tests;
