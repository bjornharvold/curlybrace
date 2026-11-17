//! What an options change does to the answers already on screen: counts and plan keep their
//! numbers but say they are out of date, an answer to an older request is stale, and the
//! remembered answers of the repaint ask again exactly when the form blocks or unblocks.
use super::*;
use crate::viewmodel::aggregate_counts::CountsUi;
use crate::viewmodel::aggregate_counts_plan::{CountsBlock, count_plan};
use crate::viewmodel::aggregate_explain::ExplainUi;
use crate::viewmodel::aggregate_explain_block::explain_request;
use crate::viewmodel::aggregate_view::AggregateView;
use crate::viewmodel::code_export::{CodeBlock, aggregate_stages};
use bson::doc;
use curlybrace_core::pipeline::{PipelineOptions, ReadPreferenceMode};
use curlybrace_core::store::Explain;

fn draft() -> AggregateState {
    AggregateState::default()
        .add_stage("$match")
        .add_stage("$sort")
}

fn secondary(d: AggregateState) -> AggregateState {
    let d = d.options_edited(OptionsEdit::ReadPreference(Some(
        ReadPreferenceMode::Secondary,
    )));
    d.options_applied().0
}

fn blocked(d: AggregateState) -> AggregateState {
    d.options_edited(OptionsEdit::Comment("   ".into()))
}

/// Counts of both stages of `d`, finished.
fn counted(d: &AggregateState) -> CountsUi {
    let plan = count_plan(d, &d.stages).expect("plan");
    let (ui, first) = CountsUi::default().begun(plan, &d.stages, &d.keys, &d.options);
    let first = first.expect("sent");
    let (ui, second) = ui.answered(first.request, 0, Ok(10));
    let second = second.expect("second");
    ui.answered(second.request, 1, Ok(4)).0
}

#[test]
fn an_options_change_marks_the_counts_out_of_date_and_keeps_their_numbers() {
    let d = draft();
    let counts = counted(&d);
    assert!(!counts.is_out_of_date(&d.stages, &d.options));
    let changed = secondary(d.with_counts(counts));
    assert!(
        changed
            .counts
            .is_out_of_date(&changed.stages, &changed.options)
    );
    assert!(
        changed.counts.result_of(changed.keys[0]).is_some(),
        "the numbers stay"
    );
    assert!(changed.counts.result_of(changed.keys[1]).is_some());
}

#[test]
fn an_options_change_marks_the_plan_out_of_date() {
    let d = draft();
    let pipeline = explain_request(&d, &d.stages).expect("request");
    let (ui, request, _) = ExplainUi::default().begun(pipeline.clone(), &d.options);
    let explain = Explain {
        document: doc! { "queryPlanner": {} },
        plan_only: true,
    };
    let ui = ui.arrived(request, Ok(explain));
    assert!(ui.done().is_some());
    assert!(!ui.is_out_of_date(Some(&pipeline), &d.options));
    let changed = secondary(d);
    assert!(ui.is_out_of_date(Some(&pipeline), &changed.options));
    assert!(ui.done().is_some(), "its numbers stay");
}

#[test]
fn the_same_options_again_are_not_out_of_date() {
    let d = draft();
    let counts = counted(&d);
    let (again, applied) = d.with_counts(counts).options_applied();
    assert_eq!(applied, Applied::Unchanged);
    assert!(!again.counts.is_out_of_date(&again.stages, &again.options));
    let (equal, _) = secondary(again).options_applied();
    assert_eq!(equal.options.set_count(), 1);
}

#[test]
fn an_answer_to_a_preview_sent_with_older_options_is_stale() {
    let (d, sent) = draft().begin_preview();
    let (request, _) = sent.expect("a preview");
    let changed = secondary(d);
    assert_ne!(changed.request, request);
    let after = changed.arrived(request, Ok(vec![doc! { "a": 1 }]));
    assert!(
        matches!(after.preview, PreviewState::Loading { .. }),
        "the old answer is dropped"
    );
}

#[test]
fn the_memos_answer_again_when_the_form_blocks_or_unblocks() {
    let view = AggregateView::default();
    let d = draft();
    let plan = view.count_plan(&d, &d.stages);
    assert!(plan.is_ok());
    view.count_plan(&d, &d.stages);
    view.explain_request(&d, &d.stages);
    view.explain_request(&d, &d.stages);
    view.code_block(&d, &d.stages);
    view.code_block(&d, &d.stages);
    assert_eq!(view.count_plan_evaluations(), 1, "a repaint parses nothing");
    assert_eq!(view.explain_evaluations(), 1);
    assert_eq!(view.code_block_evaluations(), 1);

    let stuck = blocked(d);
    let plan = view.count_plan(&stuck, &stuck.stages);
    assert!(matches!(&*plan, Err(CountsBlock::Options)));
    let request = view.explain_request(&stuck, &stuck.stages);
    assert_eq!(*request, Err(CodeBlock::Options));
    assert_eq!(
        view.code_block(&stuck, &stuck.stages),
        Some(CodeBlock::Options)
    );
    view.count_plan(&stuck, &stuck.stages);
    view.explain_request(&stuck, &stuck.stages);
    view.code_block(&stuck, &stuck.stages);
    assert_eq!(
        view.count_plan_evaluations(),
        2,
        "once more, not per repaint"
    );
    assert_eq!(view.explain_evaluations(), 2);
    assert_eq!(view.code_block_evaluations(), 2);

    let fixed = stuck.options_edited(OptionsEdit::Comment(String::new()));
    assert!(view.count_plan(&fixed, &fixed.stages).is_ok());
    assert!(view.explain_request(&fixed, &fixed.stages).is_ok());
    assert_eq!(view.code_block(&fixed, &fixed.stages), None);
    assert_eq!(view.count_plan_evaluations(), 3);
    assert_eq!(view.explain_evaluations(), 3);
    assert_eq!(view.code_block_evaluations(), 3);
}

#[test]
fn copy_is_blocked_by_an_invalid_form_and_free_again_when_fixed() {
    let stuck = blocked(draft());
    assert_eq!(aggregate_stages(&stuck), Err(CodeBlock::Options));
    let fixed = stuck.options_edited(OptionsEdit::Comment(String::new()));
    assert!(aggregate_stages(&fixed).is_ok());
    assert!(fixed.options.is_empty());
    assert_eq!(PipelineOptions::default(), fixed.options);
}
