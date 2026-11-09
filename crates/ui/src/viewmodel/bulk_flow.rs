//! View-model of the bulk update / delete flow: the steps from defining the write to its
//! report, and the rules for when a typed confirmation and the all-documents tick are
//! required. Pure and egui-free; the updaters return new values.
use super::typed_confirmation::typed_text_matches;
use bson::Document;
use curlybrace_core::bulk::{
    BulkOutcome, BulkRunId, MatchScope, PreviewGap, UpdatePreview, parse_update, validate_update,
};
use curlybrace_core::error::CoreError;
use curlybrace_core::store::Namespace;

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
    /// The preview errored.
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BulkFlow {
    pub kind: BulkKind,
    pub ns: Namespace,
    pub filter: Document,
    pub update_text: String,
    pub upsert: bool,
    pub all_documents_ticked: bool,
    pub count: CountState,
    pub preview: PreviewState,
    pub typed: String,
    pub step: BulkStep,
    pub production: bool,
    pub run: Option<BulkRunId>,
    /// The answer of the run, once it came: the outcome, or why it failed.
    pub report: Option<Result<BulkOutcome, CoreError>>,
    /// The runtime could not cancel the run; it carries on and its report still comes.
    pub cancel_unavailable: bool,
}

impl BulkFlow {
    pub fn open(kind: BulkKind, ns: Namespace, filter: Document, production: bool) -> Self {
        Self {
            kind,
            ns,
            filter,
            update_text: String::new(),
            upsert: false,
            all_documents_ticked: false,
            count: CountState::Pending,
            preview: PreviewState::Pending,
            typed: String::new(),
            step: BulkStep::Define,
            production,
            run: None,
            report: None,
            cancel_unavailable: false,
        }
    }

    /// The update text only changes while the write is being defined; later steps describe
    /// what was previewed.
    pub fn with_update_text(self, update_text: String) -> Self {
        if self.step != BulkStep::Define {
            return self;
        }
        Self {
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
            BulkStep::Define if self.update_ok() => BulkStep::Preview,
            BulkStep::Preview if self.count != CountState::Pending && self.preview_ready() => {
                BulkStep::Confirm
            }
            other => other,
        };
        Self { step, ..self }
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

    /// Records that cancelling was not possible; only a running flow has something to cancel.
    pub fn with_cancel_unavailable(self) -> Self {
        if self.step != BulkStep::Running {
            return self;
        }
        Self {
            cancel_unavailable: true,
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
                    || self.needs_all_documents_step()
                    || self.count_needs_typed()
                    || self.preview_needs_typed()
            }
        }
    }

    pub fn can_run(&self) -> bool {
        if self.step != BulkStep::Confirm || !self.preview_ready() {
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
        self.kind == BulkKind::Delete
            || parse_update(&self.update_text)
                .and_then(|spec| validate_update(&spec))
                .is_ok()
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
            PreviewState::Failed => true,
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

#[cfg(test)]
#[path = "bulk_flow_tests.rs"]
mod tests;
