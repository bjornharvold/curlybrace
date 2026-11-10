//! View-model of the bulk update / delete flow: the steps from defining the write to its
//! report, and the rules for when a typed confirmation and the all-documents tick are
//! required. Pure and egui-free; the updaters return new values.
use super::bulk_text::{DocText, UpdateCheck};
use super::bulk_undo::UndoFlow;
use super::typed_confirmation::typed_text_matches;
use bson::Document;
use curlybrace_core::bulk::{
    BulkOutcome, BulkRunId, MatchScope, PreviewGap, UndoPolicy, UndoStatus, UpdatePreview,
};
use curlybrace_core::ejson::doc_to_view_json;
use curlybrace_core::error::CoreError;
use curlybrace_core::store::Namespace;
use std::borrow::Cow;

/// An update of more documents than this asks for the typed confirmation; the number itself
/// does not.
pub const BULK_UPDATE_TYPED_CONFIRM_THRESHOLD: u64 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BulkKind {
    Update,
    Delete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BulkStep {
    Define,
    Preview,
    Confirm,
    Running,
    Report,
}

/// How many documents the filter matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountState {
    Pending,
    Known(u64),
    /// The count could not be taken (it timed out or failed).
    Unavailable,
}

/// What the preview step shows.
#[derive(Clone, Debug, PartialEq)]
pub enum PreviewState {
    Pending,
    /// The answer of the update preview.
    Update(UpdatePreview),
    /// The first documents a delete would remove.
    Sample(Vec<Document>),
    /// The preview errored: `BulkInvalidUpdate` means it proved the update fails on the matching
    /// documents; any other error means the preview is unavailable.
    Failed(CoreError),
}

#[derive(Clone, Debug, PartialEq)]
pub struct BulkFlow {
    pub kind: BulkKind,
    pub ns: Namespace,
    pub filter: Document,
    pub update_text: String,
    /// The verdict on `update_text`, worked out once when the text is set (see
    /// [`update_check`](Self::update_check)).
    pub update_check: UpdateCheck,
    /// The filter as it is shown, serialized once when the flow opens: the filter never changes
    /// afterwards.
    pub filter_text: String,
    pub upsert: bool,
    pub all_documents_ticked: bool,
    pub count: CountState,
    pub preview: PreviewState,
    /// The preview documents as shown, cut once when the preview is stored (see
    /// [`preview_docs`](Self::preview_docs)).
    pub preview_text: Vec<DocText>,
    pub typed: String,
    pub step: BulkStep,
    pub production: bool,
    pub run: Option<BulkRunId>,
    /// The answer of the run, once it came: the outcome, or why it failed.
    pub report: Option<Result<BulkOutcome, CoreError>>,
    /// What the last cancel request of the run came to. The run carries on whatever it was and
    /// its report still comes.
    pub cancel_note: CancelNote,
    /// Whether the write saves an undo copy first. Always `Snapshot` until the person, told that
    /// the copy could not be saved, chooses to run without undo.
    pub undo_policy: UndoPolicy,
    /// Whether the finished run can be undone (recorded with the report).
    pub undo: UndoStatus,
    /// The undo of this run, once the person started it from the report.
    pub undo_flow: Option<UndoFlow>,
    /// Where the undo copy is kept, as said to the person; `None` when the system gives no
    /// settings folder. Set by whoever opens the flow, so the view reads no environment.
    pub undo_folder: Option<String>,
}

/// The answer to the person's last "Stop the write", shown while the run is still going.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CancelNote {
    /// Stop was not pressed, or has not been answered yet.
    #[default]
    None,
    /// The runtime could not cancel (no permission, or the server was unreachable).
    Unavailable,
    /// The server has no operation of the run registered: it may not have started yet, so the
    /// person can try again.
    NothingToStop,
    /// A kill was sent; the write's report is awaited.
    Requested,
}

impl BulkFlow {
    pub fn open(kind: BulkKind, ns: Namespace, filter: Document, production: bool) -> Self {
        Self {
            kind,
            ns,
            filter_text: doc_to_view_json(&filter),
            filter,
            update_text: String::new(),
            update_check: UpdateCheck::of(""),
            upsert: false,
            all_documents_ticked: false,
            count: CountState::Pending,
            preview: PreviewState::Pending,
            preview_text: Vec::new(),
            typed: String::new(),
            step: BulkStep::Define,
            production,
            run: None,
            report: None,
            cancel_note: CancelNote::None,
            undo_policy: UndoPolicy::Snapshot,
            undo: UndoStatus::NotRequested,
            undo_flow: None,
            undo_folder: None,
        }
    }

    /// Records where the undo copy will be kept.
    pub fn with_undo_folder(self, undo_folder: Option<String>) -> Self {
        Self {
            undo_folder,
            ..self
        }
    }

    /// The update text only changes while the write is being defined; later steps describe
    /// what was previewed.
    pub fn with_update_text(self, update_text: String) -> Self {
        if self.step != BulkStep::Define {
            return self;
        }
        Self {
            update_check: UpdateCheck::of(&update_text),
            update_text,
            ..self
        }
    }

    /// Like the update text, the upsert flag is only editable in Define.
    pub fn with_upsert(self, upsert: bool) -> Self {
        if self.step != BulkStep::Define {
            return self;
        }
        Self { upsert, ..self }
    }

    /// Ticks or clears the all-documents box; a change drops the typed text.
    pub fn with_all_documents(self, all_documents_ticked: bool) -> Self {
        if self.locked() || self.all_documents_ticked == all_documents_ticked {
            return self;
        }
        Self {
            all_documents_ticked,
            typed: String::new(),
            ..self
        }
    }

    /// A different count is a different risk: the typed text must be given again.
    pub fn with_count(self, count: CountState) -> Self {
        if self.locked() || self.count == count {
            return self;
        }
        Self {
            count,
            typed: String::new(),
            ..self
        }
    }

    /// A different preview is a different risk: the typed text must be given again.
    pub fn with_preview(self, preview: PreviewState) -> Self {
        if self.locked() || self.preview == preview {
            return self;
        }
        Self {
            preview_text: preview_texts(&preview),
            preview,
            typed: String::new(),
            ..self
        }
    }

    /// The confirmation can only be typed in the Confirm step.
    pub fn with_typed(self, typed: String) -> Self {
        if self.step != BulkStep::Confirm {
            return self;
        }
        Self { typed, ..self }
    }

    /// One step back. Confirm keeps the preview; Preview drops it, since the definition may
    /// change. Either way the typed text goes.
    pub fn back(self) -> Self {
        match self.step {
            BulkStep::Confirm => Self {
                step: BulkStep::Preview,
                typed: String::new(),
                ..self
            },
            BulkStep::Preview => Self {
                step: BulkStep::Define,
                preview: PreviewState::Pending,
                preview_text: Vec::new(),
                typed: String::new(),
                ..self
            },
            _ => self,
        }
    }

    /// The next step (Define to Preview to Confirm), or the same flow when what the current
    /// step needs is not there yet.
    pub fn advanced(self) -> Self {
        let step = match self.step {
            BulkStep::Define if self.can_advance() => BulkStep::Preview,
            BulkStep::Preview if self.can_advance() => BulkStep::Confirm,
            other => other,
        };
        Self { step, ..self }
    }

    /// Whether [`advanced`](Self::advanced) would move to the next step: Define needs an update
    /// that is valid, Preview needs the count and the preview.
    pub fn can_advance(&self) -> bool {
        match self.step {
            BulkStep::Define => self.update_ok(),
            BulkStep::Preview => {
                self.count != CountState::Pending
                    && self.preview_ready()
                    && !self.preview_proved_invalid()
            }
            _ => false,
        }
    }

    /// Starts the run: only from Confirm and only when everything it needs is in place.
    pub fn running(self, run: BulkRunId) -> Self {
        if !self.can_run() {
            return self;
        }
        Self {
            step: BulkStep::Running,
            run: Some(run),
            ..self
        }
    }

    /// Records the answer of the run; only a running flow takes it.
    pub fn reported(self, report: Result<BulkOutcome, CoreError>) -> Self {
        if self.step != BulkStep::Running {
            return self;
        }
        Self {
            step: BulkStep::Report,
            report: Some(report),
            ..self
        }
    }

    /// Records whether the finished run can be undone; only a report takes it.
    pub fn with_undo(self, undo: UndoStatus) -> Self {
        if self.step != BulkStep::Report {
            return self;
        }
        Self { undo, ..self }
    }

    /// Whether the run ended because its undo copy could not be saved (too large, or the disk
    /// refused it). Nothing was changed; the person may run without undo.
    pub fn snapshot_refused(&self) -> bool {
        self.step == BulkStep::Report
            && matches!(
                self.report,
                Some(Err(
                    CoreError::SnapshotTooLarge | CoreError::SnapshotWriteFailed
                ))
            )
    }

    /// Back to the confirm step with undo given up, after the snapshot was refused. The typed
    /// name is asked for again, whatever the size: this is the one case where a write cannot be
    /// taken back.
    pub fn without_undo(self) -> Self {
        if !self.snapshot_refused() {
            return self;
        }
        Self {
            step: BulkStep::Confirm,
            undo_policy: UndoPolicy::None,
            undo: UndoStatus::NotRequested,
            report: None,
            run: None,
            typed: String::new(),
            cancel_note: CancelNote::None,
            ..self
        }
    }

    /// Starts the undo of this run; only a report that offers it has one.
    pub fn open_undo(self) -> Self {
        let (BulkStep::Report, UndoStatus::Offered(_), Some(run), None) =
            (self.step, self.undo, self.run, &self.undo_flow)
        else {
            return self;
        };
        let undo_flow = Some(UndoFlow::open(run, self.ns.collection.clone()));
        Self { undo_flow, ..self }
    }

    /// Stores the state of the undo flow (`None` closes it).
    pub fn with_undo_flow(self, undo_flow: Option<UndoFlow>) -> Self {
        Self { undo_flow, ..self }
    }

    /// Leaves the undo; the report of the run is shown again.
    pub fn close_undo(self) -> Self {
        Self {
            undo_flow: None,
            ..self
        }
    }

    /// Records what the last cancel request came to; only a running flow has something to
    /// cancel, and the latest answer replaces the earlier one.
    pub fn with_cancel_note(self, cancel_note: CancelNote) -> Self {
        if self.step != BulkStep::Running {
            return self;
        }
        Self {
            cancel_note,
            ..self
        }
    }

    /// An empty filter selects the whole collection, which takes an explicit tick.
    pub fn needs_all_documents_step(&self) -> bool {
        self.filter.is_empty()
    }

    /// Whether the collection name must be typed before the run.
    pub fn needs_typed(&self) -> bool {
        match self.kind {
            BulkKind::Delete => true,
            BulkKind::Update => {
                self.production
                    || self.undo_policy == UndoPolicy::None
                    || self.needs_all_documents_step()
                    || self.count_needs_typed()
                    || self.preview_needs_typed()
            }
        }
    }

    /// Whether the preview ran the update on the matching documents and it failed there: the
    /// real run would fail (and may partly apply), so neither Next nor Run is offered.
    pub fn preview_proved_invalid(&self) -> bool {
        self.preview == PreviewState::Failed(CoreError::BulkInvalidUpdate)
    }

    pub fn can_run(&self) -> bool {
        if self.step != BulkStep::Confirm || !self.preview_ready() || self.preview_proved_invalid()
        {
            return false;
        }
        let ticked = !self.needs_all_documents_step() || self.all_documents_ticked;
        let typed = !self.needs_typed() || typed_text_matches(&self.ns.collection, &self.typed);
        ticked && typed && self.count != CountState::Pending && self.update_ok()
    }

    /// All documents only for an empty filter that was ticked; the core refuses the rest.
    pub fn scope(&self) -> MatchScope {
        if self.needs_all_documents_step() && self.all_documents_ticked {
            MatchScope::AllDocuments
        } else {
            MatchScope::Filtered
        }
    }

    /// Running and finished flows no longer take edits.
    fn locked(&self) -> bool {
        matches!(self.step, BulkStep::Running | BulkStep::Report)
    }

    /// A delete has no update; an update needs text that parses and validates.
    fn update_ok(&self) -> bool {
        self.kind == BulkKind::Delete || self.update_check().is_valid()
    }

    /// The verdict on the update text: the stored one, which `open` and `with_update_text`
    /// keep in step with the text. A flow built field by field with another text (only tests do
    /// that) has a stored verdict that is not for its text, and gets a fresh one instead, so the
    /// answer is always about the text that would run.
    pub fn update_check(&self) -> Cow<'_, UpdateCheck> {
        match self.update_check.is_for(&self.update_text) {
            true => Cow::Borrowed(&self.update_check),
            false => Cow::Owned(UpdateCheck::of(&self.update_text)),
        }
    }

    /// The i18n key of what is wrong with the update text; none for blank or valid text.
    pub fn update_problem(&self) -> Option<&'static str> {
        self.update_check().problem()
    }

    /// The preview documents as shown: the ones cut when the preview was stored, or, for a flow
    /// whose `preview` was set field by field (only tests do that), cut now.
    pub fn preview_docs(&self) -> Cow<'_, [DocText]> {
        match preview_len(&self.preview) == self.preview_text.len() {
            true => Cow::Borrowed(&self.preview_text),
            false => Cow::Owned(preview_texts(&self.preview)),
        }
    }

    fn count_needs_typed(&self) -> bool {
        match self.count {
            CountState::Pending => false,
            CountState::Known(n) => n > BULK_UPDATE_TYPED_CONFIRM_THRESHOLD,
            CountState::Unavailable => true,
        }
    }

    fn preview_needs_typed(&self) -> bool {
        match &self.preview {
            PreviewState::Failed(_) => true,
            PreviewState::Update(p) => {
                matches!(
                    p.gap,
                    Some(PreviewGap::TransactionsUnavailable | PreviewGap::Conflict)
                ) || p.docs.iter().any(|d| d.after.is_none())
            }
            PreviewState::Pending | PreviewState::Sample(_) => false,
        }
    }

    fn preview_ready(&self) -> bool {
        self.preview != PreviewState::Pending
    }
}

/// The documents of a preview as shown, each cut once.
fn preview_texts(preview: &PreviewState) -> Vec<DocText> {
    match preview {
        PreviewState::Update(p) => p.docs.iter().map(DocText::of_preview).collect(),
        PreviewState::Sample(docs) => docs.iter().map(DocText::of_sample).collect(),
        PreviewState::Pending | PreviewState::Failed(_) => Vec::new(),
    }
}

/// How many documents `preview` shows, without cutting them.
fn preview_len(preview: &PreviewState) -> usize {
    match preview {
        PreviewState::Update(p) => p.docs.len(),
        PreviewState::Sample(docs) => docs.len(),
        PreviewState::Pending | PreviewState::Failed(_) => 0,
    }
}

#[cfg(test)]
#[path = "bulk_flow_tests.rs"]
mod tests;
