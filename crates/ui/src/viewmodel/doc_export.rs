//! The document export as the view-model knows it: idle, configuring in a modal, waiting for the
//! file dialog, running, and the result. Pure; every step returns a new state, and an event of
//! another run changes nothing. The gate that decides when an export may start is
//! `doc_export_block`.
//!
//! The source (namespace, filter) and the chosen path are user data: every `Debug` here hides
//! them, so state dumps and logs stay clean.
use std::fmt;
use std::path::PathBuf;

use bson::Document;
use curlybrace_core::compare::protocol::RunId;
use curlybrace_core::error::CoreError;
use curlybrace_core::export::protocol::{DocExportOutcome, DocFormat, JsonMode};
use curlybrace_core::runtime::SessionId;
use curlybrace_core::store::{Namespace, Query};

use super::compare::RedactedPath;

/// What the modal edits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DocExportDraft {
    pub format: DocFormat,
    pub mode: JsonMode,
}

/// What is exported: the documents of `ns` matching `filter` (`None` = the whole collection).
#[derive(Clone, PartialEq)]
pub struct DocExportSource {
    pub session: SessionId,
    pub ns: Namespace,
    pub filter: Option<Document>,
    /// The applied query has a sort or a projection, which the export does not apply.
    pub ignores_options: bool,
}

impl fmt::Debug for DocExportSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocExportSource")
            .field("session", &self.session)
            .field("filtered", &self.filter.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Default, PartialEq)]
pub enum DocExportUi {
    #[default]
    Idle,
    /// The modal is open and edits the draft.
    Configuring {
        source: DocExportSource,
        draft: DocExportDraft,
    },
    /// The file dialog is open: waiting for the path or its cancellation.
    Choosing {
        source: DocExportSource,
        draft: DocExportDraft,
    },
    Running {
        run: RunId,
        source: DocExportSource,
        draft: DocExportDraft,
        path: RedactedPath,
        /// The document count when known up front.
        total: Option<u64>,
        done: u64,
        cancel_requested: bool,
    },
    Finished {
        outcome: Result<DocExportOutcome, CoreError>,
        path: Option<RedactedPath>,
    },
}

impl fmt::Debug for DocExportUi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => f.write_str("Idle"),
            Self::Configuring { draft, .. } => f.debug_tuple("Configuring").field(draft).finish(),
            Self::Choosing { draft, .. } => f.debug_tuple("Choosing").field(draft).finish(),
            Self::Running {
                run,
                draft,
                total,
                done,
                cancel_requested,
                ..
            } => f
                .debug_struct("Running")
                .field("run", run)
                .field("draft", draft)
                .field("total", total)
                .field("done", done)
                .field("cancel_requested", cancel_requested)
                .finish_non_exhaustive(),
            Self::Finished { outcome, .. } => f
                .debug_struct("Finished")
                .field("outcome", &outcome.as_ref().map_err(|_| "error"))
                .finish_non_exhaustive(),
        }
    }
}

impl DocExportUi {
    /// The dialog is open or a job runs: other work waits.
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Choosing { .. } | Self::Running { .. })
    }

    /// The modal opens for `source`; ignored while busy.
    pub fn open(self, source: DocExportSource) -> Self {
        match self.is_busy() {
            true => self,
            false => Self::Configuring {
                source,
                draft: DocExportDraft::default(),
            },
        }
    }

    /// `change` applied to the draft; only while configuring.
    pub fn edited(self, change: impl FnOnce(DocExportDraft) -> DocExportDraft) -> Self {
        match self {
            Self::Configuring { source, draft } => Self::Configuring {
                source,
                draft: change(draft),
            },
            other => other,
        }
    }

    /// The file dialog opens: configuring -> choosing.
    pub fn choosing(self) -> Self {
        match self {
            Self::Configuring { source, draft } => Self::Choosing { source, draft },
            other => other,
        }
    }

    /// The job `run` was sent, writing to `path`; `total` is the count when known.
    pub fn started(self, run: RunId, path: PathBuf, total: Option<u64>) -> Self {
        match self {
            Self::Choosing { source, draft } | Self::Configuring { source, draft } => {
                Self::Running {
                    run,
                    source,
                    draft,
                    path: RedactedPath::new(path),
                    total,
                    done: 0,
                    cancel_requested: false,
                }
            }
            other => other,
        }
    }

    /// Cancel was sent for the running job; a second call changes nothing.
    pub fn cancelling(self) -> Self {
        match self {
            Self::Running {
                run,
                source,
                draft,
                path,
                total,
                done,
                ..
            } => Self::Running {
                run,
                source,
                draft,
                path,
                total,
                done,
                cancel_requested: true,
            },
            other => other,
        }
    }

    /// Progress of `event_run`; any other run is stale. `done` is the real count written. A total
    /// is only a hint: the event's total (when real, non-zero) replaces the known one, and a
    /// total that `done` has passed is dropped, so the view shows the plain count.
    pub fn progressed(self, event_run: RunId, done: u64, total: Option<u64>) -> Self {
        match self {
            Self::Running {
                run,
                source,
                draft,
                path,
                total: known,
                cancel_requested,
                ..
            } if run == event_run => {
                let event_total = total.filter(|t| *t > 0);
                let total = event_total.or(known).filter(|t| done <= *t);
                Self::Running {
                    run,
                    source,
                    draft,
                    path,
                    total,
                    done,
                    cancel_requested,
                }
            }
            other => other,
        }
    }

    /// `event_run` ended with `outcome`; any other run is stale.
    pub fn finished(self, event_run: RunId, outcome: Result<DocExportOutcome, CoreError>) -> Self {
        match self {
            Self::Running { run, path, .. } if run == event_run => Self::Finished {
                outcome,
                path: Some(path),
            },
            other => other,
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

/// Why the export entry is disabled. The first reason that holds is the one shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocExportBlock {
    NotConnected,
    /// The export reads in binary order, which a collation would change.
    Collated,
    Busy,
}

impl DocExportBlock {
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::NotConnected => "export.reason.not_connected",
            Self::Collated => "export.reason.collated",
            Self::Busy => "export.reason.busy",
        }
    }
}

/// Why an export of `query`'s documents cannot start now; `None` when it can.
pub fn doc_export_block(connected: bool, query: &Query, busy: bool) -> Option<DocExportBlock> {
    let reason = |holds: bool, block| holds.then_some(block);
    reason(!connected, DocExportBlock::NotConnected)
        .or(reason(
            query.options.collation.is_some(),
            DocExportBlock::Collated,
        ))
        .or(reason(busy, DocExportBlock::Busy))
}

#[cfg(test)]
#[path = "doc_export_tests.rs"]
mod tests;
