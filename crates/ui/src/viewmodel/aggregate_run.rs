//! The Aggregate draft's confirmed write run: whether Run is offered, the typed-confirmation
//! dialog, and the outcome of the last run. Per session, kept through a lost connection like the
//! draft. Every op returns a new state; nothing here talks to a server.
use super::aggregate::{AggregateState, Mode};
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::RunTarget;
use curlybrace_core::pipeline::{PipelineError, pipeline_text, run_pipeline, stage_name};
use curlybrace_core::store::Namespace;

/// The stages that make a pipeline a write run.
const WRITE_STAGES: [&str; 2] = ["$out", "$merge"];

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
    /// The target's document count now, once it arrived.
    pub target_count: Option<u64>,
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

    /// Whether the typed text names the target collection: trimmed, case-sensitive, always.
    pub fn is_confirmed(&self) -> bool {
        self.typed.trim() == self.target.collection
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
}

impl RunUi {
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
                target_count: Some(count),
                ..d
            },
            false => d,
        });
        Self { dialog, ..self }
    }

    /// Run `run` was sent: the dialog is closed and the run is in flight.
    pub fn started(self, run: u64) -> Self {
        Self {
            dialog: None,
            status: RunStatus::Running,
            run,
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
            error => RunStatus::Failed(error),
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

/// What the Run button needs to know about the session around the draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunEnv {
    pub connected: bool,
    pub has_collection: bool,
    pub read_only: bool,
}

/// Why Run is shown but cannot be used.
#[derive(Clone, Debug, PartialEq)]
pub enum RunDisabled {
    ReadOnly,
    NoConnection,
    NoCollection,
    /// A run is already in flight.
    Busy,
    /// The text-mode buffer was edited and not applied to the stages yet.
    TextNotApplied,
    /// The pipeline cannot be run as written.
    Invalid(PipelineError),
}

/// Whether the Run button is shown and usable.
#[derive(Clone, Debug, PartialEq)]
pub enum RunAvailability {
    /// The last enabled stage is not `$out` or `$merge`: there is nothing to run.
    Hidden,
    Disabled(RunDisabled),
    Ready,
}

impl AggregateState {
    /// Whether the last enabled stage writes (`$out` or `$merge`).
    fn ends_in_write_stage(&self) -> bool {
        self.stages
            .iter()
            .rfind(|s| s.enabled)
            .and_then(|s| stage_name(&s.text))
            .is_some_and(|name| WRITE_STAGES.contains(&name.as_str()))
    }

    /// Whether Run is shown and usable for this draft in `env`.
    pub fn can_run(&self, env: RunEnv) -> RunAvailability {
        if !self.ends_in_write_stage() {
            return RunAvailability::Hidden;
        }
        let refusal = match () {
            _ if self.run.is_running() => Some(RunDisabled::Busy),
            _ if env.read_only => Some(RunDisabled::ReadOnly),
            _ if !env.connected => Some(RunDisabled::NoConnection),
            _ if !env.has_collection => Some(RunDisabled::NoCollection),
            _ if self.mode == Mode::Text && self.text != pipeline_text(&self.stages) => {
                Some(RunDisabled::TextNotApplied)
            }
            _ => run_pipeline(&self.stages).err().map(RunDisabled::Invalid),
        };
        match refusal {
            Some(reason) => RunAvailability::Disabled(reason),
            None => RunAvailability::Ready,
        }
    }

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
