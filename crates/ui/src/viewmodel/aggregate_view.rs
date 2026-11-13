//! What the Aggregate tab keeps for itself between frames: the preview's tree and view mode, the
//! page built from the preview's documents, the stage edit waiting for the typing to pause, and
//! the last request it asked a preview for on its own. View-only state, parked with its session
//! like the Documents tab's tree; nothing here is sent anywhere. Also the pure helpers the tab
//! draws from: a stage's one-line status and the i18n key of a pipeline error.
use super::aggregate::{Preview, PreviewState};
use super::{PageKey, TreeState, ViewMode, page_key};
use bson::Document;
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::{PipelineError, stage_name};
use curlybrace_core::runtime::COMMAND_TIMEOUT_MESSAGE;
use curlybrace_core::store::Page;

/// How long the typing in a stage pauses before the edit is sent (and previewed).
pub const EDIT_PAUSE_SECS: f64 = 0.6;

/// A stage text typed but not sent yet.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingEdit {
    pub index: usize,
    pub text: String,
    /// When it was last typed in (egui input time, seconds).
    pub at: f64,
}

/// The page shown for a preview's documents, built once per distinct list (not every frame).
#[derive(Clone, Debug, Default)]
pub struct PageCache {
    built: Option<(Page, PageKey)>,
}

impl PageCache {
    /// The page of `docs` and its content key.
    pub fn of(&mut self, docs: &[Document]) -> (&Page, PageKey) {
        let stale = self
            .built
            .as_ref()
            .is_none_or(|(page, _)| page.docs.as_slice() != docs);
        if stale {
            let page = Page {
                docs: docs.to_vec(),
                skip: 0,
                has_more: false,
            };
            let key = page_key(&page);
            self.built = Some((page, key));
        }
        let (page, key) = self.built.as_ref().expect("built above");
        (page, *key)
    }
}

/// The Aggregate tab's own view state of one session.
#[derive(Clone, Debug, Default)]
pub struct AggregateView {
    /// The preview's expansion and caches (the preview never offers a write).
    pub tree: TreeState,
    /// Tree, Table or JSON for the preview.
    pub mode: ViewMode,
    /// The request the tab last asked a preview for on its own (see
    /// `AggregateState::wants_auto_refresh`).
    pub auto_sent: Option<u64>,
    pub pending: Option<PendingEdit>,
    pub page: PageCache,
}

impl AggregateView {
    /// The view after its session was reset (connection lost, reconnect): the typing and the
    /// chosen view mode stay, the rest belongs to the documents that are gone.
    pub fn carried(&self) -> Self {
        Self {
            mode: self.mode,
            pending: self.pending.clone(),
            ..Self::default()
        }
    }

    /// Stage `index` was typed in at `now`. An edit still waiting in another stage is handed
    /// back, to be sent first: only one stage's typing waits at a time, and it must not be lost.
    pub fn edited(self, index: usize, text: String, now: f64) -> (Self, Option<(usize, String)>) {
        let displaced = self
            .pending
            .filter(|p| p.index != index)
            .map(|p| (p.index, p.text));
        let pending = Some(PendingEdit {
            index,
            text,
            at: now,
        });
        (Self { pending, ..self }, displaced)
    }

    /// The text stage `index` shows: what is being typed there, else `text`.
    pub fn shown_text<'a>(&'a self, index: usize, text: &'a str) -> &'a str {
        match &self.pending {
            Some(p) if p.index == index => &p.text,
            _ => text,
        }
    }

    /// Seconds left before the waiting edit is due, if one waits.
    pub fn due_in(&self, now: f64) -> Option<f64> {
        self.pending
            .as_ref()
            .map(|p| (p.at + EDIT_PAUSE_SECS - now).max(0.0))
    }

    /// Takes the waiting edit out once the typing paused, or at once when `force` (another
    /// action follows and must see the typed text first).
    pub fn take_due(self, now: f64, force: bool) -> (Self, Option<(usize, String)>) {
        let due = force || self.due_in(now).is_some_and(|left| left <= 0.0);
        match (due, self.pending) {
            (true, Some(p)) => (
                Self {
                    pending: None,
                    ..self
                },
                Some((p.index, p.text)),
            ),
            (_, pending) => (Self { pending, ..self }, None),
        }
    }
}

/// The documents the preview pane shows: the current preview, or the last good one kept while a
/// newer one loads or after one failed.
pub fn shown_preview(state: &PreviewState) -> Option<&Preview> {
    match state {
        PreviewState::Idle => None,
        PreviewState::Shown(p) => Some(p),
        PreviewState::Loading { last, .. } | PreviewState::Failed { last, .. } => last.as_ref(),
    }
}

/// The one-line status of a stage text.
#[derive(Clone, Debug, PartialEq)]
pub enum StageStatus {
    /// A stage named like this.
    Valid(String),
    /// Not JSON.
    Syntax {
        line: usize,
        column: usize,
        message: String,
    },
    /// JSON, but not an object with exactly one `$`-prefixed key.
    NotAStage,
}

/// The status of stage text `text`.
pub fn stage_status(text: &str) -> StageStatus {
    if let Err(e) = serde_json::from_str::<serde_json::Value>(text) {
        return StageStatus::Syntax {
            line: e.line(),
            column: e.column(),
            message: bare_message(&e.to_string()),
        };
    }
    match stage_name(text) {
        Some(name) => StageStatus::Valid(name),
        None => StageStatus::NotAStage,
    }
}

/// A parser message without its trailing " at line L column C" (the position is shown apart).
fn bare_message(message: &str) -> String {
    message
        .split(" at line ")
        .next()
        .unwrap_or(message)
        .to_string()
}

/// The i18n key of a preview failure that has a message of its own (a timeout, which core maps to
/// `Database("timeout")` for both the server's code 50 and the client-side bound); `None` for
/// the rest, which show as database errors.
pub fn preview_error_key(error: &CoreError) -> Option<&'static str> {
    match error {
        CoreError::Database(text) if text == COMMAND_TIMEOUT_MESSAGE => {
            Some("aggregate.error.timeout")
        }
        _ => None,
    }
}

/// The i18n key and parameters of a pipeline error; positions are 1-based.
pub fn pipeline_error_message(
    error: &PipelineError,
) -> (&'static str, Vec<(&'static str, String)>) {
    let position = |index: &usize| ("n", (index + 1).to_string());
    match error {
        PipelineError::Syntax {
            line,
            column,
            message,
        } => (
            "aggregate.error.syntax",
            vec![
                ("line", line.to_string()),
                ("column", column.to_string()),
                ("message", bare_message(message)),
            ],
        ),
        PipelineError::NotAnArray => ("aggregate.error.not_an_array", Vec::new()),
        PipelineError::NotAStage { index } => {
            ("aggregate.error.not_a_stage", vec![position(index)])
        }
        PipelineError::WriteStageNotLast { index, name } => (
            "aggregate.error.write_not_last",
            vec![position(index), ("name", name.clone())],
        ),
        PipelineError::NestedWriteStage { index, name } => (
            "aggregate.error.nested_write",
            vec![position(index), ("name", name.clone())],
        ),
        PipelineError::Empty => ("aggregate.error.empty", Vec::new()),
    }
}

#[cfg(test)]
#[path = "aggregate_view_tests.rs"]
mod tests;
