//! View-model of undoing a bulk run: a preview of the first documents beside what they are now,
//! the typed confirmation naming the collection, the run, and its report. Undo is a write, so it
//! always asks for the typed name, whatever the size. Pure and egui-free; the updaters return new
//! values.
use super::bulk_text::compact;
use super::typed_confirmation::typed_text_matches;
use curlybrace_core::bulk::{BulkRunId, UndoPreview};
use curlybrace_core::error::CoreError;
use curlybrace_core::snapshot::UndoReport;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UndoStep {
    /// The preview and the typed confirmation, on one screen.
    Confirm,
    Running,
    Report,
}

#[derive(Clone, Debug, PartialEq)]
pub enum UndoPreviewState {
    Pending,
    Ready(UndoPreview),
    /// The preview could not be produced (the copy is missing or damaged, or it belongs to
    /// another connection): the undo is not offered.
    Failed(CoreError),
}

/// One preview document as shown: what it is now (`None`: gone) and what undo restores.
#[derive(Clone, Debug, PartialEq)]
pub struct UndoDocText {
    pub current: Option<String>,
    pub original: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UndoFlow {
    pub run: BulkRunId,
    /// The collection the typed confirmation names.
    pub collection: String,
    pub step: UndoStep,
    pub preview: UndoPreviewState,
    /// The preview documents as shown, cut once when the preview is stored.
    preview_text: Vec<UndoDocText>,
    pub typed: String,
    pub report: Option<Result<UndoReport, CoreError>>,
}

impl UndoFlow {
    pub fn open(run: BulkRunId, collection: String) -> Self {
        Self {
            run,
            collection,
            step: UndoStep::Confirm,
            preview: UndoPreviewState::Pending,
            preview_text: Vec::new(),
            typed: String::new(),
            report: None,
        }
    }

    /// Stores the answer of the preview; a different preview is a different risk, so the typed
    /// text must be given again. Only the confirm step takes it.
    pub fn with_preview(self, preview: UndoPreviewState) -> Self {
        if self.step != UndoStep::Confirm {
            return self;
        }
        Self {
            preview_text: texts_of(&preview),
            preview,
            typed: String::new(),
            ..self
        }
    }

    pub fn with_typed(self, typed: String) -> Self {
        if self.step != UndoStep::Confirm {
            return self;
        }
        Self { typed, ..self }
    }

    pub fn can_run(&self) -> bool {
        self.step == UndoStep::Confirm
            && matches!(self.preview, UndoPreviewState::Ready(_))
            && typed_text_matches(&self.collection, &self.typed)
    }

    pub fn running(self) -> Self {
        if !self.can_run() {
            return self;
        }
        Self {
            step: UndoStep::Running,
            ..self
        }
    }

    /// Records the answer; only a running undo takes it.
    pub fn reported(self, report: Result<UndoReport, CoreError>) -> Self {
        if self.step != UndoStep::Running {
            return self;
        }
        Self {
            step: UndoStep::Report,
            report: Some(report),
            ..self
        }
    }

    /// Back to the confirm step after a report, with the preview to be read again and the
    /// confirmation to be typed again. The snapshot is still there, so this is a retry.
    pub fn retried(self) -> Self {
        if self.step != UndoStep::Report {
            return self;
        }
        Self::open(self.run, self.collection)
    }

    /// The preview documents as shown (cut once when the preview was stored).
    pub fn preview_docs(&self) -> &[UndoDocText] {
        &self.preview_text
    }

    /// Whether a second try can get anywhere: the preview or the undo failed for a reason that
    /// can clear (the connection, a timeout), or some documents could not be restored. A copy
    /// that is gone, damaged or from another connection fails the same way every time.
    pub fn can_retry(&self) -> bool {
        match (&self.step, &self.preview, &self.report) {
            (UndoStep::Confirm, UndoPreviewState::Failed(error), _) => transient(error),
            (UndoStep::Report, _, Some(Err(error))) => transient(error),
            (UndoStep::Report, _, Some(Ok(report))) => report.errors > 0,
            _ => false,
        }
    }

    /// A failed preview, to be read again; the typed name stays empty.
    pub fn retry_preview(self) -> Self {
        let retry = self.step == UndoStep::Confirm
            && matches!(&self.preview, UndoPreviewState::Failed(e) if transient(e));
        match retry {
            true => Self {
                preview: UndoPreviewState::Pending,
                preview_text: Vec::new(),
                typed: String::new(),
                ..self
            },
            false => self,
        }
    }
}

/// Errors a second try can get past: the connection or the server was not there, or the answer
/// was lost. Everything else (a copy that is missing, damaged or from another connection, a
/// connection that is read-only) fails the same way again.
fn transient(error: &CoreError) -> bool {
    matches!(
        error,
        CoreError::WriteOutcomeUnknown
            | CoreError::Unreachable
            | CoreError::UnreachableSsh
            | CoreError::UnreachableWrite
            | CoreError::UnreachableWriteSsh
            | CoreError::ConnectionInterrupted(_)
            | CoreError::TlsHandshake
            | CoreError::TlsHandshakeSsh
            | CoreError::Database(_)
            | CoreError::Superseded
    )
}

fn texts_of(preview: &UndoPreviewState) -> Vec<UndoDocText> {
    match preview {
        UndoPreviewState::Ready(p) => p
            .docs
            .iter()
            .map(|d| UndoDocText {
                current: d.current.as_ref().map(compact),
                original: compact(&d.original),
            })
            .collect(),
        UndoPreviewState::Pending | UndoPreviewState::Failed(_) => Vec::new(),
    }
}

#[cfg(test)]
#[path = "bulk_undo_tests.rs"]
mod tests;
