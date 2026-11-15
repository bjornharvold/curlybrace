//! The Explain section of the Aggregate tab: the plan the server gave for the enabled stages up
//! to the selected one, and which of Preview | Plan | Raw the pane shows. Kept per session inside
//! the draft (so it is parked and restored with it), never persisted. Every op returns a new
//! state; nothing here talks to a server, and the explain document (user data) is never logged:
//! `Debug` of a plan names its shape, not its content.
use bson::Document;
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::{AggPlanOutcome, ExplainVerbosity, parse_aggregate_explain};
use curlybrace_core::store::Explain;
use std::fmt;

/// What the pane under the stages shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResultTab {
    /// The documents of the selected stage.
    #[default]
    Preview,
    /// The typed plan with its hints.
    Plan,
    /// The explain document as the server sent it.
    Raw,
}

/// The pipeline an explain was asked for. A plan whose fingerprint differs from the pipeline the
/// draft would send now is out of date: the draft was edited after the request.
pub type Fingerprint = Vec<Document>;

/// A plan that arrived.
#[derive(Clone, PartialEq)]
pub struct ExplainDone {
    pub plan: AggPlanOutcome,
    /// The explain document, for the Raw tab.
    pub raw: Document,
    pub verbosity: ExplainVerbosity,
    pub for_draft: Fingerprint,
}

impl fmt::Debug for ExplainDone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExplainDone")
            .field("understood", &matches!(self.plan, AggPlanOutcome::Plan(_)))
            .field("verbosity", &self.verbosity)
            .finish_non_exhaustive()
    }
}

/// The explain request's progress.
#[derive(Clone, Default, PartialEq)]
pub enum ExplainState {
    #[default]
    Idle,
    /// Request `request` is in flight; `for_draft` is what it explains.
    Loading {
        request: u64,
        verbosity: ExplainVerbosity,
        for_draft: Fingerprint,
    },
    Done(Box<ExplainDone>),
    Failed {
        reason: CoreError,
    },
}

/// Prints the variant and counts only: the pipeline under `Loading` holds the person's `$match`
/// values, and a failure's detail text may name data.
impl fmt::Debug for ExplainState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => f.write_str("Idle"),
            Self::Loading {
                request,
                verbosity,
                for_draft,
            } => f
                .debug_struct("Loading")
                .field("request", request)
                .field("verbosity", verbosity)
                .field("stages", &for_draft.len())
                .finish(),
            Self::Done(done) => f.debug_tuple("Done").field(done).finish(),
            Self::Failed { reason } => f
                .debug_struct("Failed")
                .field("reason", &reason.i18n_key())
                .finish(),
        }
    }
}

/// One session's explain: the state, the verbosity the next run uses and the tab in view.
#[derive(Clone, Debug, PartialEq)]
pub struct ExplainUi {
    pub state: ExplainState,
    pub verbosity: ExplainVerbosity,
    pub tab: ResultTab,
    /// The newest request number; an answer for any other is stale.
    pub request: u64,
}

impl Default for ExplainUi {
    fn default() -> Self {
        Self {
            state: ExplainState::Idle,
            verbosity: ExplainVerbosity::QueryPlanner,
            tab: ResultTab::Preview,
            request: 0,
        }
    }
}

impl ExplainUi {
    /// Whether a request is out and not answered yet.
    pub fn is_loading(&self) -> bool {
        matches!(self.state, ExplainState::Loading { .. })
    }

    /// An explain of `pipeline` was sent under a new request number: the pane shows the Plan tab
    /// and waits for it. Returns the number and the verbosity sent.
    pub fn begun(self, pipeline: Fingerprint) -> (Self, u64, ExplainVerbosity) {
        let request = self.request.wrapping_add(1);
        let verbosity = self.verbosity;
        let next = Self {
            state: ExplainState::Loading {
                request,
                verbosity,
                for_draft: pipeline,
            },
            tab: ResultTab::Plan,
            request,
            ..self
        };
        (next, request, verbosity)
    }

    /// The answer to `request`. Only the one awaited counts; any other (older, or after a reset)
    /// changes nothing. A failure is kept in the pane, never raised as a banner.
    pub fn arrived(self, request: u64, outcome: Result<Explain, CoreError>) -> Self {
        match self.state {
            ExplainState::Loading {
                request: awaited,
                verbosity,
                for_draft,
            } if awaited == request => {
                let state = match outcome {
                    Ok(explain) => ExplainState::Done(Box::new(ExplainDone {
                        plan: parse_aggregate_explain(&explain.document),
                        raw: explain.document,
                        verbosity,
                        for_draft,
                    })),
                    Err(reason) => ExplainState::Failed { reason },
                };
                Self { state, ..self }
            }
            state => Self { state, ..self },
        }
    }

    /// The request that was awaited gave up without an answer (its connection was replaced):
    /// the pane is idle again. Any other request changes nothing.
    pub fn given_up(self, request: u64) -> Self {
        match self.state {
            ExplainState::Loading {
                request: awaited, ..
            } if awaited == request => Self {
                state: ExplainState::Idle,
                ..self
            },
            _ => self,
        }
    }

    /// The plan no longer applies (another collection, a new connection): nothing is shown and
    /// an answer still on its way is stale. The verbosity stays; the tab goes back to Preview.
    pub fn reset(self) -> Self {
        Self {
            state: ExplainState::Idle,
            tab: ResultTab::Preview,
            request: self.request.wrapping_add(1),
            ..self
        }
    }

    pub fn with_verbosity(self, verbosity: ExplainVerbosity) -> Self {
        Self { verbosity, ..self }
    }

    pub fn with_tab(self, tab: ResultTab) -> Self {
        Self { tab, ..self }
    }

    /// The plan in view, with the pipeline it is for.
    pub fn done(&self) -> Option<&ExplainDone> {
        match &self.state {
            ExplainState::Done(done) => Some(done),
            _ => None,
        }
    }

    /// Whether the plan in view was asked for a pipeline other than `current`.
    pub fn is_out_of_date(&self, current: Option<&[Document]>) -> bool {
        self.done()
            .is_some_and(|done| current != Some(done.for_draft.as_slice()))
    }

    /// Whether `request` is the one awaited.
    pub fn awaits(&self, request: u64) -> bool {
        matches!(self.state, ExplainState::Loading { request: r, .. } if r == request)
    }
}

#[cfg(test)]
#[path = "aggregate_explain_tests.rs"]
mod tests;
