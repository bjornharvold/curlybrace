use super::*;
use crate::viewmodel::aggregate::{Preview, PreviewError, PreviewState};
use crate::viewmodel::aggregate_counts_plan::count_plan;
use crate::viewmodel::aggregate_flight::Queued;
use crate::viewmodel::aggregate_options::{OptionField, Tri};
use crate::viewmodel::aggregate_run::{CountState, RunDialog};
use bson::doc;
use curlybrace_core::pipeline::{OptionsError, PipelineOptions, ReadPreferenceMode, RunTarget};
use curlybrace_core::store::Namespace;

fn draft() -> AggregateState {
    AggregateState::default()
        .add_stage("$match")
        .add_stage("$sort")
}

fn with_comment(draft: AggregateState, text: &str) -> AggregateState {
    draft.options_edited(OptionsEdit::Comment(text.into()))
}

fn good_preview() -> Preview {
    Preview {
        docs: vec![doc! { "a": 1 }],
        not_run: Vec::new(),
    }
}

#[test]
fn the_default_draft_has_no_options_and_a_closed_section() {
    let d = AggregateState::default();
    assert_eq!(d.options, PipelineOptions::default());
    assert!(!d.options_ui.open);
    assert!(!d.options_blocked());
}

#[test]
fn a_valid_change_applies_through_changed() {
    let dialog = RunDialog {
        target: RunTarget {
            db: None,
            collection: "dest".into(),
        },
        stage: "$out".into(),
        source: Namespace::new("app", "people"),
        typed: String::new(),
        count: CountState::Loading,
    };
    let d = draft();
    let plan = count_plan(&d, &d.stages).expect("plan");
    let (counts, _) = d.counts.clone().begun(plan, &d.stages, &d.keys);
    let run = d.run.clone().opened(dialog);
    let d = AggregateState {
        run,
        ..d.with_counts(counts)
    };
    assert!(d.counts.is_running());
    let before = d.request;
    let _ = d.is_modified();
    assert_eq!(d.saved.modified_evaluations(), 1);
    let edited = d.options_edited(OptionsEdit::ReadPreference(Some(
        ReadPreferenceMode::Secondary,
    )));
    let (next, applied) = edited.options_applied();
    assert_eq!(applied, Applied::Changed);
    assert_eq!(next.request, before.wrapping_add(1));
    assert_eq!(
        next.options.read_preference,
        Some(ReadPreferenceMode::Secondary)
    );
    assert!(next.run.dialog.is_none(), "the run dialog closes");
    assert!(!next.counts.is_running(), "the sequence ends");
    assert_eq!(
        next.saved.modified_evaluations(),
        0,
        "the modified memo was forgotten (its counter starts over)"
    );
    let _ = next.is_modified();
    assert_eq!(next.saved.modified_evaluations(), 1);
}

#[test]
fn applying_the_same_options_changes_nothing() {
    let d = draft();
    let before = d.request;
    let (next, applied) = d.options_applied();
    assert_eq!(applied, Applied::Unchanged);
    assert_eq!(next.request, before);
}

#[test]
fn an_invalid_form_keeps_the_last_applied_options() {
    let d = draft().options_edited(OptionsEdit::ReadPreference(Some(
        ReadPreferenceMode::Nearest,
    )));
    let (d, _) = d.options_applied();
    let d = with_comment(d, "   ");
    assert!(d.options_blocked());
    let before = d.request;
    let (next, applied) = d.options_applied();
    assert_eq!(applied, Applied::Blocked);
    assert_eq!(next.request, before);
    assert_eq!(
        next.options.read_preference,
        Some(ReadPreferenceMode::Nearest),
        "the last good options stay"
    );
    assert_eq!(
        next.options_ui.form.problem(OptionField::Comment),
        Some(OptionsError::CommentEmpty)
    );
}

#[test]
fn preview_blocked_keeps_the_last_good_preview_and_drops_the_queued_one() {
    let d = AggregateState {
        preview: PreviewState::Shown(good_preview()),
        in_flight: Some(3),
        queued: Some(Queued {
            request: 4,
            ns: Namespace::new("app", "people"),
            pipeline: vec![doc! { "$match": {} }],
        }),
        ..draft()
    };
    let blocked = d.preview_blocked();
    assert_eq!(blocked.queued, None);
    assert_eq!(blocked.in_flight, Some(3), "the server may still answer it");
    match blocked.preview {
        PreviewState::Failed {
            error: PreviewError::Options,
            last: Some(last),
        } => assert_eq!(last, good_preview()),
        other => panic!("expected the options block, got {other:?}"),
    }
}

#[test]
fn fixing_the_form_after_a_block_is_unblocked_even_with_equal_options() {
    let d = with_comment(draft(), "  ");
    let (d, applied) = d.options_applied();
    assert_eq!(applied, Applied::Blocked);
    let d = d.preview_blocked();
    let fixed = d.options_edited(OptionsEdit::Comment(String::new()));
    let (next, applied) = fixed.options_applied();
    assert_eq!(applied, Applied::Unblocked);
    assert_eq!(next.options, PipelineOptions::default());
    // Once the pane is previewing again nothing is unblocked any more.
    let (previewing, _) = next.begin_preview();
    let (_, applied) = previewing.options_applied();
    assert_eq!(applied, Applied::Unchanged);
}

#[test]
fn reset_empties_the_form_and_applies() {
    let d = draft().options_edited(OptionsEdit::AllowDiskUse(Tri::On));
    let (d, _) = d.options_applied();
    assert_eq!(d.options.set_count(), 1);
    let (reset, applied) = d.options_reset();
    assert_eq!(applied, Applied::Changed);
    assert!(reset.options.is_empty());
    assert_eq!(reset.options_ui.form, OptionsForm::default());
    let (_, again) = reset.options_reset();
    assert_eq!(again, Applied::Unchanged);
}

#[test]
fn reset_keeps_the_section_open() {
    let d = draft().options_toggled();
    let (reset, _) = d.options_reset();
    assert!(reset.options_ui.open);
}

#[test]
fn toggling_opens_and_closes_without_touching_the_request() {
    let d = draft();
    let before = d.request;
    let open = d.options_toggled();
    assert!(open.options_ui.open);
    assert_eq!(open.request, before);
    assert!(!open.options_toggled().options_ui.open);
}

#[test]
fn an_options_edit_alone_does_not_touch_the_request() {
    let d = draft();
    let before = d.request;
    let typed = with_comment(d, "hello");
    assert_eq!(typed.request, before, "typing sends nothing by itself");
    assert!(typed.options.is_empty(), "nothing is applied yet");
}
