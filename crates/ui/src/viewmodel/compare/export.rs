//! The export workflow as the view-model knows it: idle, configuring in a modal, waiting for the
//! file dialog, running, and the result. Pure; every step returns a new state, and an event of
//! another run changes nothing. The gates that decide when an export may start are in
//! `export_gate.rs`.
//!
//! The report text for the clipboard is the one piece of document-derived data the UI holds. It
//! lives in `ClipboardText` (redacted `Debug`, no `Display`) from the runtime's `ExportText`
//! until the app layer takes it once (`ExportUi::text_taken`); nothing else reads it.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use curlybrace_core::compare::apply::IdPolicy;
use curlybrace_core::compare::protocol::{ExportFormat, ExportOutcome, RunId};

use super::pairs::EntryId;

/// Most characters a label keeps (the box and the reducer both cap it; core sanitises it again).
pub const LABEL_MAX_CHARS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportKind {
    /// The pending changes as a mongosh script or a JSON plan.
    Plan,
    /// The differences of whole pairs as a CSV report.
    Report,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanOptions {
    pub format: ExportFormat,
    /// Free text, kept as typed (core sanitises it again).
    pub label: String,
    pub id_policy: IdPolicy,
}

impl Default for PlanOptions {
    fn default() -> Self {
        Self {
            format: ExportFormat::Mongosh,
            label: String::new(),
            id_policy: IdPolicy::default(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReportOptions {
    /// The chosen entries; only those with a completed pair are exported.
    pub pairs: BTreeSet<EntryId>,
    pub include_paths: bool,
    pub label: String,
}

/// Everything the modal edits. Both option sets are kept so switching kind loses nothing; `kind`
/// says which one is in use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportDraft {
    pub kind: ExportKind,
    pub plan: PlanOptions,
    pub report: ReportOptions,
}

/// The clipboard report. Carries keys, which can be personal data: `Debug` hides it.
#[derive(Clone, PartialEq, Eq)]
pub struct ClipboardText(String);

impl ClipboardText {
    pub fn new(text: String) -> Self {
        Self(text)
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for ClipboardText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ClipboardText(<{} bytes>)", self.0.len())
    }
}

/// The file chosen for an export. A path can hold a user name: it is shown in the UI (through
/// `as_path`) but `Debug` never prints it, so state dumps and logs stay clean.
#[derive(Clone, PartialEq, Eq)]
pub struct RedactedPath(PathBuf);

impl From<PathBuf> for RedactedPath {
    fn from(path: PathBuf) -> Self {
        Self(path)
    }
}

impl RedactedPath {
    pub fn new(path: PathBuf) -> Self {
        Self(path)
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

impl std::fmt::Debug for RedactedPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RedactedPath(..)")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ExportUi {
    #[default]
    Idle,
    /// The modal is open and edits the draft.
    Configuring(ExportDraft),
    /// The file dialog is open: waiting for `ExportPathChosen` or `ExportPathCancelled`.
    Choosing(ExportDraft),
    Running {
        run: RunId,
        draft: ExportDraft,
        /// Where the file goes; `None` for the clipboard.
        path: Option<RedactedPath>,
        total: u64,
        done: u64,
        cancel_requested: bool,
        /// The clipboard report once it arrived (before `ExportDone`).
        text: Option<ClipboardText>,
    },
    Finished {
        outcome: ExportOutcome,
        kind: ExportKind,
        /// The file written (or attempted); `None` for the clipboard.
        path: Option<RedactedPath>,
    },
    /// The clipboard report is ready: `text` waits for the app layer, then it is `None`.
    Copied {
        rows: u64,
        text: Option<ClipboardText>,
    },
}

impl ExportUi {
    /// The dialog is open or a job runs: other work waits and the session stays.
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Choosing(_) | Self::Running { .. })
    }

    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running { .. })
    }

    pub fn running_run(&self) -> Option<RunId> {
        match self {
            Self::Running { run, .. } => Some(*run),
            _ => None,
        }
    }

    /// The draft the modal shows (while configuring or waiting for the dialog).
    pub fn draft(&self) -> Option<&ExportDraft> {
        match self {
            Self::Configuring(draft) | Self::Choosing(draft) => Some(draft),
            _ => None,
        }
    }

    pub fn configuring(&self) -> Option<&ExportDraft> {
        match self {
            Self::Configuring(draft) => Some(draft),
            _ => None,
        }
    }

    /// `change` applied to the draft; only while configuring.
    pub fn edited(self, change: impl FnOnce(ExportDraft) -> ExportDraft) -> Self {
        match self {
            Self::Configuring(draft) => Self::Configuring(change(draft)),
            other => other,
        }
    }

    /// The file dialog opens: configuring -> choosing.
    pub fn choosing(self) -> Self {
        match self {
            Self::Configuring(draft) => Self::Choosing(draft),
            other => other,
        }
    }

    /// The dialog was cancelled, or the export could not go on: back to the modal, options kept.
    /// From running too (a failed job); every other state stays.
    pub fn reconfiguring(self) -> Self {
        match self {
            Self::Choosing(draft) | Self::Running { draft, .. } => Self::Configuring(draft),
            other => other,
        }
    }

    /// The job `run` was sent for the draft (to `path`, or the clipboard when `None`).
    pub fn started(self, run: RunId, path: Option<PathBuf>) -> Self {
        match self {
            Self::Choosing(draft) | Self::Configuring(draft) => Self::Running {
                run,
                draft,
                path: path.map(RedactedPath::new),
                total: 0,
                done: 0,
                cancel_requested: false,
                text: None,
            },
            other => other,
        }
    }

    pub fn can_cancel(&self) -> bool {
        matches!(
            self,
            Self::Running {
                cancel_requested: false,
                ..
            }
        )
    }

    /// Cancel was sent for the running job; a second call changes nothing.
    pub fn cancelling(self) -> Self {
        match self {
            Self::Running {
                run,
                draft,
                path,
                total,
                done,
                text,
                ..
            } => Self::Running {
                run,
                draft,
                path,
                total,
                done,
                cancel_requested: true,
                text,
            },
            other => other,
        }
    }

    /// Progress of `run`; any other run is stale.
    pub fn progressed(self, event_run: RunId, done: u64, total: u64) -> Self {
        match self {
            Self::Running {
                run,
                draft,
                path,
                cancel_requested,
                text,
                ..
            } if run == event_run => Self::Running {
                run,
                draft,
                path,
                total,
                done,
                cancel_requested,
                text,
            },
            other => other,
        }
    }

    /// The clipboard report of `run` arrived; kept only for a clipboard job of that run.
    pub fn text_arrived(self, event_run: RunId, report: String) -> Self {
        match self {
            Self::Running {
                run,
                draft,
                path: None,
                total,
                done,
                cancel_requested,
                ..
            } if run == event_run => Self::Running {
                run,
                draft,
                path: None,
                total,
                done,
                cancel_requested,
                text: Some(ClipboardText::new(report)),
            },
            other => other,
        }
    }

    /// `run` ended. A clipboard report that arrived and was not cancelled becomes `Copied`;
    /// anything else is the result to show (a cancelled report's text is dropped).
    pub fn finished(self, event_run: RunId, outcome: ExportOutcome) -> Self {
        match self {
            Self::Running {
                run,
                draft,
                path,
                text,
                ..
            } if run == event_run => match (path, text) {
                (None, Some(text)) if !outcome.cancelled => Self::Copied {
                    rows: outcome.written,
                    text: Some(text),
                },
                (path, _) => Self::Finished {
                    outcome,
                    kind: draft.kind,
                    path,
                },
            },
            other => other,
        }
    }

    /// The clipboard text, handed over once: the state keeps no copy afterwards.
    pub fn text_taken(self) -> (Self, Option<String>) {
        match self {
            Self::Copied { rows, text } => (
                Self::Copied { rows, text: None },
                text.map(ClipboardText::into_string),
            ),
            other => (other, None),
        }
    }

    /// The modal is closed; only when nothing is in progress.
    pub fn closed(self) -> Self {
        match self.is_busy() {
            true => self,
            false => Self::Idle,
        }
    }
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
