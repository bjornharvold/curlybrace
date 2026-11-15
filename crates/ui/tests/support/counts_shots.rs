//! Per-stage counts for the Aggregate tab's screenshots and layout tests: the pipeline of
//! [`super::aggregate_shots::three_stage_draft`] counted stage by stage through the view model's
//! own sequencer, so the cards show what the real app shows.
use super::aggregate_shots::three_stage_draft;
use super::shots::browsing;
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::COUNT_CAP;
use curlybrace_core::runtime::COMMAND_TIMEOUT_MESSAGE;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::aggregate_counts_plan::count_plan;
use curlybrace_ui::viewmodel::{AggregateState, CollectionTab, ViewMode};

/// `draft` counted: every enabled stage answered in order with the next of `outcomes`.
pub fn counted(mut draft: AggregateState, outcomes: Vec<Result<u64, CoreError>>) -> AggregateState {
    let plan = count_plan(&draft, &draft.stages).expect("the fixture pipeline can be counted");
    let (mut ui, mut send) = draft.counts.clone().begun(plan, &draft.stages, &draft.keys);
    for outcome in outcomes {
        let sent = send.expect("a count is waiting for this answer");
        (ui, send) = ui.answered(sent.request, sent.stage_index, outcome);
    }
    draft.counts = ui;
    draft
}

/// The three-stage pipeline with all three counted: the `$match` hit the cap, the `$group` and
/// the `$sort` kept four documents.
pub fn counted_draft() -> AggregateState {
    counted(
        three_stage_draft(),
        vec![Ok(COUNT_CAP as u64), Ok(4), Ok(4)],
    )
}

/// The same with a failure and a timeout, the longest lines the cards can hold.
pub fn counted_with_failures() -> AggregateState {
    let failure = CoreError::Database("The aggregation failed on the server".into());
    let timeout = CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into());
    counted(three_stage_draft(), vec![Ok(1), Err(failure), Err(timeout)])
}

/// The three-stage pipeline with the first stage answered and the second one being counted.
pub fn counting_draft() -> AggregateState {
    counted(three_stage_draft(), vec![Ok(12)])
}

/// The three-stage pipeline counted, then its `$sort` edited: the numbers stay, out of date.
pub fn counted_then_edited_draft() -> AggregateState {
    counted_draft().edit(2, "{\n  \"$sort\": { \"customers\": 1 }\n}")
}

/// The Aggregate tab of a connected browser on `crm.customers` showing `draft`.
pub fn tab_of(draft: AggregateState) -> impl Fn(&mut AppState) {
    let draft = std::sync::Arc::new(draft);
    move |state| {
        browsing(ViewMode::Tree)(state);
        state.tab = CollectionTab::Aggregate;
        state.aggregate = Box::new((*draft).clone());
    }
}

/// [`counted_draft`] in the Aggregate tab.
pub fn aggregate_counts_tab(state: &mut AppState) {
    tab_of(counted_draft())(state);
}
