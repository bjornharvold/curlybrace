//! Whether the Run button is shown and usable. Asked on every repaint, so the part that parses
//! the stages and builds the plan is remembered (`RunShapeMemo`) and recomputed only when the
//! stages, the mode or the text-mode buffer change; what the session around the draft says
//! (`RunEnv`) is cheap and read fresh every time.
use super::aggregate::{AggregateState, Mode};
use curlybrace_core::pipeline::{
    PipelineError, Stage, WRITE_STAGES, pipeline_text, run_pipeline, stage_name,
};
use std::cell::{Cell, RefCell};
use std::sync::Arc;

/// What the Run button needs to know about the session around the draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunEnv {
    pub connected: bool,
    pub has_collection: bool,
    pub read_only: bool,
    /// The open collection is in a database the server owns (`admin`, `local`, `config`): a
    /// stage that names no target database would write there.
    pub source_reserved: bool,
    /// Another write of the session is in flight or waits for its confirmation.
    pub other_write: bool,
}

/// Why Run is shown but cannot be used.
#[derive(Clone, Debug, PartialEq)]
pub enum RunDisabled {
    ReadOnly,
    NoConnection,
    NoCollection,
    /// A run is already in flight.
    Busy,
    /// Another write is in flight or waiting for its confirmation.
    OtherWrite,
    /// The text-mode buffer was edited and not applied to the stages yet.
    TextNotApplied,
    /// The pipeline cannot be run as written.
    Invalid(PipelineError),
}

/// Whether the Run button is shown and usable.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum RunAvailability {
    /// The last enabled stage is not `$out` or `$merge`: there is nothing to run.
    #[default]
    Hidden,
    Disabled(RunDisabled),
    Ready,
}

/// What the draft says about its run, whatever the session around it.
#[derive(Debug)]
pub(super) struct RunShape {
    ends_in_write: bool,
    /// The text-mode buffer is the stages' own text (nothing edited and left unapplied).
    text_applied: bool,
    /// The last enabled stage, where a target refusal is shown.
    last_enabled: usize,
    /// The plan: `Ok(true)` when its target names no database (the source's is used).
    plan: Result<bool, PipelineError>,
}

/// What `RunShape` was computed from.
#[derive(Debug)]
struct ShapeKey {
    stages: Vec<Stage>,
    mode: Mode,
    /// The buffer, only in text mode: stage mode does not read it.
    text: Option<String>,
}

impl ShapeKey {
    fn is_of(&self, stages: &[Stage], mode: Mode, text: Option<&str>) -> bool {
        self.stages == stages && self.mode == mode && self.text.as_deref() == text
    }
}

/// The remembered `RunShape` of a draft. It is keyed on its inputs, so no change of the draft
/// can leave a stale answer behind. Two memos are always equal: it says nothing about the draft
/// that the draft does not already say.
#[derive(Clone, Debug, Default)]
pub struct RunShapeMemo {
    cached: RefCell<Option<Arc<(ShapeKey, RunShape)>>>,
    evaluations: Cell<u32>,
}

impl PartialEq for RunShapeMemo {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for RunShapeMemo {}

impl AggregateState {
    /// The remembered shape of this draft, computed when its inputs changed.
    fn run_shape(&self) -> Arc<(ShapeKey, RunShape)> {
        let memo = &self.run_shape;
        let text = (self.mode == Mode::Text).then_some(self.text.as_str());
        if let Some(hit) = memo
            .cached
            .borrow()
            .as_ref()
            .filter(|c| c.0.is_of(&self.stages, self.mode, text))
        {
            return hit.clone();
        }
        let key = ShapeKey {
            stages: self.stages.clone(),
            mode: self.mode,
            text: text.map(str::to_string),
        };
        let fresh = Arc::new((key, self.compute_run_shape()));
        memo.evaluations.set(memo.evaluations.get().wrapping_add(1));
        *memo.cached.borrow_mut() = Some(fresh.clone());
        fresh
    }

    /// How many times the shape was really computed (the seam that shows a repaint does not).
    pub fn run_shape_evaluations(&self) -> u32 {
        self.run_shape.evaluations.get()
    }

    fn compute_run_shape(&self) -> RunShape {
        let last = self.stages.iter().rposition(|s| s.enabled);
        let ends_in_write = last
            .and_then(|i| stage_name(&self.stages[i].text))
            .is_some_and(|name| WRITE_STAGES.contains(&name.as_str()));
        let plan = match ends_in_write {
            true => run_pipeline(&self.stages).map(|plan| plan.target.db.is_none()),
            false => Ok(false),
        };
        RunShape {
            ends_in_write,
            text_applied: self.mode != Mode::Text || self.text == pipeline_text(&self.stages),
            last_enabled: last.unwrap_or(0),
            plan,
        }
    }

    /// Whether Run is shown and usable for this draft in `env`.
    pub fn can_run(&self, env: RunEnv) -> RunAvailability {
        let shape = self.run_shape();
        let shape = &shape.1;
        if !shape.ends_in_write {
            return RunAvailability::Hidden;
        }
        let refusal = match () {
            _ if self.run.is_running() => Some(RunDisabled::Busy),
            _ if env.read_only => Some(RunDisabled::ReadOnly),
            _ if !env.connected => Some(RunDisabled::NoConnection),
            _ if !env.has_collection => Some(RunDisabled::NoCollection),
            _ if env.other_write => Some(RunDisabled::OtherWrite),
            _ if !shape.text_applied => Some(RunDisabled::TextNotApplied),
            _ => shape.plan_refusal(env),
        };
        match refusal {
            Some(reason) => RunAvailability::Disabled(reason),
            None => RunAvailability::Ready,
        }
    }
}

impl RunShape {
    /// Why the pipeline cannot run: it is unrunnable as written, or its target (the source
    /// database when the stage names none) belongs to the server.
    fn plan_refusal(&self, env: RunEnv) -> Option<RunDisabled> {
        match &self.plan {
            Err(error) => Some(RunDisabled::Invalid(error.clone())),
            Ok(true) if env.source_reserved => Some(RunDisabled::Invalid(
                PipelineError::ReservedTargetDatabase {
                    index: self.last_enabled,
                },
            )),
            Ok(_) => None,
        }
    }
}
