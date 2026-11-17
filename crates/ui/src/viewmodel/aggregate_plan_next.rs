//! What the Plan tab offers after a plan arrived: try an index (set the hint and explain again),
//! clear the hint, and the suggested index with its command. Pure: the view draws it and the
//! reducer acts on the actions it names. The suggestion comes from the pipeline and the options
//! the plan was made with (`ExplainDone`), never from a draft edited since.
use super::aggregate::AggregateState;
use super::aggregate_options::keys_text;
use curlybrace_core::explain::{
    AggHint, AggPlan, AggPlanOutcome, SuggestedIndex, create_index_command, suggest_for_pipeline,
};
use curlybrace_core::pipeline_options::HintOption;
use curlybrace_core::store::Namespace;

/// The follow-ups under a plan.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanNext {
    /// Show Try an index (and Clear hint when one is set): the plan reads the whole collection or
    /// sorts in memory, or a hint is set (so it can be cleared).
    pub try_index: bool,
    /// The hint set now: its name, or its key pattern as text.
    pub current_hint: Option<String>,
    /// Why the controls that send are off: `aggregate.explain.disabled.running` while an explain
    /// is out, `aggregate.options.blocked` while the options form has a problem.
    pub blocked: Option<&'static str>,
    pub suggested: Option<SuggestedIndex>,
    /// `create_index_command` of the suggestion, with the collation the plan was made with.
    pub command: Option<String>,
}

impl PlanNext {
    fn off() -> Self {
        Self {
            try_index: false,
            current_hint: None,
            blocked: None,
            suggested: None,
            command: None,
        }
    }
}

/// The plan reads the whole collection or sorts in memory: an index could help.
pub fn plan_wants_an_index(plan: &AggPlan) -> bool {
    plan.hints
        .iter()
        .any(|hint| matches!(hint, AggHint::CollectionScan | AggHint::InMemorySort))
}

/// The follow-ups for `draft`'s Plan tab; all off unless it holds a plan (or a hint is set while
/// the next plan is awaited).
pub fn plan_next(draft: &AggregateState, ns: Option<&Namespace>) -> PlanNext {
    let current_hint = draft.options.hint.as_ref().map(|hint| match hint {
        HintOption::Name(name) => name.clone(),
        HintOption::Keys(keys) => keys_text(keys),
    });
    let blocked = if draft.explain.is_loading() {
        Some("aggregate.explain.disabled.running")
    } else if draft.options_blocked() {
        Some("aggregate.options.blocked")
    } else {
        None
    };
    let Some(done) = draft.explain.done() else {
        // No plan (an explain is out, or it failed): nothing is offered, but a hint that is set
        // stays reachable, so a hint that made the explain fail can be cleared.
        return PlanNext {
            try_index: current_hint.is_some(),
            current_hint,
            blocked,
            ..PlanNext::off()
        };
    };
    let wants = matches!(&*done.plan, AggPlanOutcome::Plan(plan) if plan_wants_an_index(plan));
    let suggested = wants
        .then(|| suggest_for_pipeline(&done.for_draft))
        .flatten();
    let command = match (&suggested, ns) {
        (Some(index), Some(ns)) => Some(create_index_command(
            index,
            ns,
            done.options.collation.as_ref(),
        )),
        _ => None,
    };
    PlanNext {
        try_index: wants || current_hint.is_some(),
        current_hint,
        blocked,
        suggested,
        command,
    }
}

#[cfg(test)]
#[path = "aggregate_plan_next_tests.rs"]
mod tests;
