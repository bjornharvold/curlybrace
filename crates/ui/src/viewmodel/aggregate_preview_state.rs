//! The preview pane's state: the documents of the previewed stage, why there are none, and the
//! last good preview kept through a reload or a failure.
use bson::Document;
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::PipelineError;

/// The documents after the previewed stage, and the trailing write stages that were not run.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub docs: Vec<Document>,
    pub not_run: Vec<String>,
}

/// Why a preview is not shown: the pipeline was refused before sending, the options form has a
/// problem, or the server failed.
#[derive(Clone, Debug, PartialEq)]
pub enum PreviewError {
    Pipeline(PipelineError),
    /// The options form is invalid: nothing is sent until it is fixed.
    Options,
    Core(CoreError),
}

/// The preview pane. `last` is the latest good preview, kept through a reload or a failure.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum PreviewState {
    #[default]
    Idle,
    /// Request `AggregateState::request` is in flight.
    Loading {
        not_run: Vec<String>,
        last: Option<Preview>,
    },
    Shown(Preview),
    Failed {
        error: PreviewError,
        last: Option<Preview>,
    },
}

impl PreviewState {
    /// The latest good preview this state shows or keeps.
    pub fn into_last(self) -> Option<Preview> {
        match self {
            Self::Idle => None,
            Self::Shown(p) => Some(p),
            Self::Loading { last, .. } | Self::Failed { last, .. } => last,
        }
    }
}
