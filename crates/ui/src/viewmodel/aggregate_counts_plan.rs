//! What a "Count stages" click would send, worked out before anything is sent: for each enabled
//! stage either the bounded pipeline that counts it or the reason it is not counted. Asked on
//! every repaint (the button's state), but answering means parsing every stage, so the answer is
//! remembered on the inputs it was computed from (`InputMemo`); a repaint with nothing changed
//! parses nothing. Stage text is user data and is never logged here.
use super::aggregate::{AggregateState, Mode};
use super::aggregate_options_state::OptionsGate;
use super::aggregate_view::AggregateView;
use super::input_memo::InputMemo;
use bson::Document;
use curlybrace_core::pipeline::{COUNT_CAP, PipelineError, Stage, count_pipelines, pipeline_text};
use std::sync::Arc;

/// Why a stage gets no number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotCounted {
    /// The stage is not a stage (it does not parse).
    Invalid,
    /// An earlier enabled stage does not parse, and this one needs it.
    AfterInvalid,
    /// This stage writes, or an earlier one does; nothing that writes is ever sent.
    WriteStage,
    /// This stage or an earlier one holds a write stage in a sub-pipeline.
    NestedWrite,
}

/// One enabled stage's part of the plan.
#[derive(Clone, Debug, PartialEq)]
pub enum PlanItem {
    /// Count it with this pipeline (the stages up to it, `$limit`, `$count`).
    Send {
        index: usize,
        pipeline: Vec<Document>,
    },
    Skip {
        index: usize,
        reason: NotCounted,
    },
}

/// What the plan is for: the enabled stages, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct CountPlan {
    pub items: Vec<PlanItem>,
}

/// Why the plan is empty-handed: the Count stages button is off with this reason.
#[derive(Clone, Debug, PartialEq)]
pub enum CountsBlock {
    /// The options form has a problem: nothing is sent until it is fixed.
    Options,
    /// The preview's own reasons (no enabled stage).
    Pipeline(PipelineError),
    /// The text of text mode holds edits the stages do not have yet.
    TextNotApplied,
    /// Text mode: the counts are shown on the stage cards, which only the stage view has.
    TextMode,
    /// Every enabled stage is one that cannot be counted.
    Nothing,
}

pub type CountPlanOutcome = Result<CountPlan, CountsBlock>;

fn reason_of(error: &PipelineError, index: usize) -> NotCounted {
    match error {
        PipelineError::NotAStage { index: at } if *at < index => NotCounted::AfterInvalid,
        PipelineError::WriteStageNotLast { .. } => NotCounted::WriteStage,
        PipelineError::NestedWriteStage { .. } => NotCounted::NestedWrite,
        _ => NotCounted::Invalid,
    }
}

/// The plan for `stages` (the draft's stages with the typing applied), or why there is none.
pub fn count_plan(draft: &AggregateState, stages: &[Stage]) -> CountPlanOutcome {
    draft.options_gate().check(CountsBlock::Options)?;
    if draft.mode == Mode::Text {
        // The counts are drawn on the stage cards, which text mode does not show.
        return Err(match draft.text != pipeline_text(stages) {
            true => CountsBlock::TextNotApplied,
            false => CountsBlock::TextMode,
        });
    }
    if !stages.iter().any(|s| s.enabled) {
        return Err(CountsBlock::Pipeline(PipelineError::Empty));
    }
    let items: Vec<PlanItem> = count_pipelines(stages, COUNT_CAP)
        .into_iter()
        .enumerate()
        .filter(|(index, _)| stages[*index].enabled)
        .map(|(index, built)| match built {
            Ok(pipeline) => PlanItem::Send { index, pipeline },
            Err(error) => PlanItem::Skip {
                index,
                reason: reason_of(&error, index),
            },
        })
        .collect();
    match items.iter().any(|i| matches!(i, PlanItem::Send { .. })) {
        true => Ok(CountPlan { items }),
        false => Err(CountsBlock::Nothing),
    }
}

/// What the answer was computed from.
#[derive(Clone, Debug)]
pub struct CountsKey {
    stages: Vec<Stage>,
    mode: Mode,
    text: Option<String>,
    gate: OptionsGate,
}

impl CountsKey {
    fn is_of(&self, stages: &[Stage], mode: Mode, text: Option<&str>, gate: OptionsGate) -> bool {
        self.stages == stages
            && self.mode == mode
            && self.text.as_deref() == text
            && self.gate == gate
    }
}

/// Shared, so a hit copies a pointer rather than the pipelines.
pub type CountsMemo = InputMemo<CountsKey, Arc<CountPlanOutcome>>;

impl AggregateView {
    /// `count_plan`, remembered between repaints on the stages, mode, whether the options form
    /// is blocked and (in text mode) the buffer.
    pub fn count_plan(&self, draft: &AggregateState, stages: &[Stage]) -> Arc<CountPlanOutcome> {
        let text = (draft.mode == Mode::Text).then_some(draft.text.as_str());
        let gate = draft.options_gate();
        self.counts_memo.answer(
            |key| key.is_of(stages, draft.mode, text, gate),
            || CountsKey {
                stages: stages.to_vec(),
                mode: draft.mode,
                text: text.map(str::to_string),
                gate,
            },
            |_| Arc::new(count_plan(draft, stages)),
        )
    }

    /// How many times `count_plan` really parsed (the seam that shows a repaint does not).
    pub fn count_plan_evaluations(&self) -> u32 {
        self.counts_memo.evaluations()
    }
}

#[cfg(test)]
#[path = "aggregate_counts_plan_tests.rs"]
mod tests;
