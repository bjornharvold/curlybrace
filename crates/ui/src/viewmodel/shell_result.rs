//! The structured result of a shell run: the last value `mongosh` returned, which output mode the
//! person chose (remembered per session, never switched for them) and whether that mode can show
//! the value. Pure and egui-free; every updater returns a new value. No user-facing strings: the
//! view turns a [`Reason`] into a translated line.
//!
//! The mode lives on `ShellState` (it outlives runs); everything about one run's value lives in
//! [`ResultView`] (it is replaced by every new run).
use super::shell::ShellState;
use super::shell_result_tree::TreeExpansion;
pub use curlybrace_core::shell::ShellResult;
use std::sync::Arc;

/// How the Shell tab shows a run: its text output, or its last value as a table or a tree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputMode {
    #[default]
    Text,
    Table,
    Tree,
}

impl OutputMode {
    pub const ALL: [OutputMode; 3] = [OutputMode::Text, OutputMode::Table, OutputMode::Tree];
}

/// Where the value of the run `ResultView::run_id` stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResultState {
    /// The run is in flight: its value may still come.
    Pending,
    /// The value arrived (`ResultView::result` holds it).
    Ready,
    /// No value: nothing ran yet, the run ended without one, or it was cleared.
    #[default]
    NoResult,
}

/// The value of one run. A new run replaces it whole, so the Table and Tree never show the
/// previous run's documents under the new run's text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResultView {
    pub run_id: u64,
    pub result: Option<Arc<ShellResult>>,
    pub state: ResultState,
    /// With `NoResult`: the run did not end cleanly (an error, a cancel, a refusal, a lost
    /// connection), so the missing value is not the script's doing.
    pub run_failed: bool,
    /// What the Tree mode shows expanded; empty for every new value.
    pub tree: TreeExpansion,
}

/// Whether a mode can show the current value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Available,
    Unavailable(Reason),
}

/// Why a mode cannot show the current value. The view gives each its own explanation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The run is still going.
    Pending,
    /// The run ended cleanly without a value (or with `undefined`).
    NoValue,
    /// The value is not documents: a scalar, or (for the Table) an array of non-documents.
    NotDocuments,
    /// The value was over the payload cap; this many characters (UTF-16 units).
    TooBig(u64),
    /// A value of a kind that cannot be lifted out.
    Unsupported,
    /// Masking the connection password broke the value, so it is hidden.
    Withheld,
    /// The run failed, was cancelled or refused, or its connection was lost.
    RunFailed,
}

impl ResultView {
    /// The empty value a run starts with.
    pub(super) fn pending(run_id: u64) -> Self {
        Self {
            run_id,
            state: ResultState::Pending,
            ..Self::default()
        }
    }

    /// The run ended: a value that arrived stays, a missing one is `NoResult`.
    pub(super) fn ended(&self, clean: bool) -> Self {
        match self.state {
            ResultState::Pending => Self {
                state: ResultState::NoResult,
                run_failed: !clean,
                ..self.clone()
            },
            ResultState::Ready | ResultState::NoResult => self.clone(),
        }
    }
}

impl ShellState {
    /// The value of the run `run_id`; ignored unless that run is the one in flight.
    pub fn result_arrived(&self, run_id: u64, result: ShellResult) -> Self {
        let awaited = self.is_current(run_id) && self.result.run_id == run_id;
        if !awaited {
            return self.clone();
        }
        Self {
            result: ResultView {
                run_id,
                result: Some(Arc::new(result)),
                state: ResultState::Ready,
                run_failed: false,
                tree: TreeExpansion::default(),
            },
            ..self.clone()
        }
    }

    pub fn mode_set(&self, mode: OutputMode) -> Self {
        Self {
            mode,
            ..self.clone()
        }
    }

    /// The connection was lost or reopened. A run in flight ends unclean, as if it failed (its
    /// value, if it already arrived, stays); an idle shell keeps its value, as its text output
    /// stays too. The mode stays.
    pub fn result_interrupted(&self) -> Self {
        match self.is_running() {
            true => Self {
                result: self.result.ended(false),
                ..self.clone()
            },
            false => self.clone(),
        }
    }

    /// Whether `mode` can show the current value. Text always can.
    pub fn availability(&self, mode: OutputMode) -> Availability {
        if mode == OutputMode::Text {
            return Availability::Available;
        }
        match (self.result.state, self.result.result.as_deref()) {
            (ResultState::Pending, _) => Availability::Unavailable(Reason::Pending),
            (ResultState::Ready, Some(result)) => availability_of(result, mode),
            _ if self.result.run_failed => Availability::Unavailable(Reason::RunFailed),
            _ => Availability::Unavailable(Reason::NoValue),
        }
    }

    /// The value when `mode` can show it.
    pub(super) fn shown_in(&self, mode: OutputMode) -> Option<&ShellResult> {
        match self.availability(mode) {
            Availability::Available => self.result.result.as_deref(),
            Availability::Unavailable(_) => None,
        }
    }
}

/// Table: documents and a single object (one row). Tree: those and an array of other values.
fn availability_of(result: &ShellResult, mode: OutputMode) -> Availability {
    let unavailable = Availability::Unavailable;
    match result {
        ShellResult::Documents { .. } | ShellResult::Object { .. } => Availability::Available,
        ShellResult::Values { .. } if mode == OutputMode::Tree => Availability::Available,
        ShellResult::Values { .. } => unavailable(Reason::NotDocuments),
        ShellResult::Scalar {
            undefined: true, ..
        } => unavailable(Reason::NoValue),
        ShellResult::Scalar { .. } => unavailable(Reason::NotDocuments),
        ShellResult::TooBig { chars } => unavailable(Reason::TooBig(*chars)),
        ShellResult::Unsupported { .. } => unavailable(Reason::Unsupported),
        ShellResult::Withheld => unavailable(Reason::Withheld),
    }
}

#[cfg(test)]
#[path = "shell_result_tests.rs"]
mod tests;
