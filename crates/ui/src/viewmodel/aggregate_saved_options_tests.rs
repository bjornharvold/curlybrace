//! Saved pipelines and the draft's options: opening restores them, "modified" compares them with
//! the stages, and a form with a problem counts as a change.
use super::*;
use crate::viewmodel::aggregate_options::{OptionsEdit, OptionsForm, Tri};
use curlybrace_core::pipeline::{HintOption, PipelineOptions, ReadPreferenceMode};
use curlybrace_core::saved_pipeline::{SavedDraft, SavedPipelines};
use uuid::Uuid;

fn two() -> AggregateState {
    AggregateState::default()
        .add_stage("$match")
        .add_stage("$sort")
}

fn options() -> PipelineOptions {
    PipelineOptions {
        hint: Some(HintOption::Name("status_1".into())),
        read_preference: Some(ReadPreferenceMode::Secondary),
        ..PipelineOptions::default()
    }
}

/// The entry saved for `draft` with `options`.
fn entry_of(draft: &AggregateState, options: PipelineOptions) -> SavedPipeline {
    let conn = Uuid::new_v4();
    let store = SavedPipelines::default()
        .saved(
            conn,
            "totals",
            None,
            SavedDraft {
                stages: draft.as_saved_stages(),
                options,
            },
            1,
            false,
        )
        .unwrap();
    store.list(conn)[0].clone()
}

fn applied(draft: AggregateState, edit: OptionsEdit) -> AggregateState {
    draft.options_edited(edit).options_applied().0
}

#[test]
fn opening_restores_the_options_and_the_form() {
    let entry = entry_of(&two(), options());
    let opened = AggregateState::default()
        .options_toggled()
        .opened_from(&entry);
    assert_eq!(opened.options, options());
    assert_eq!(
        opened.options_ui.form,
        OptionsForm::from_options(&options())
    );
    assert!(
        opened.options_ui.open,
        "the section stays as the person left it"
    );
    assert!(!opened.is_modified());
}

#[test]
fn opening_a_pipeline_saved_before_options_clears_them() {
    let entry = entry_of(&two(), PipelineOptions::default());
    let with = applied(two(), OptionsEdit::AllowDiskUse(Tri::On));
    assert_eq!(with.options.set_count(), 1);
    let opened = with.opened_from(&entry);
    assert!(opened.options.is_empty());
    assert_eq!(opened.options_ui.form, OptionsForm::default());
}

#[test]
fn an_options_change_marks_the_draft_modified_and_back_unmarks_it() {
    let entry = entry_of(&two(), PipelineOptions::default());
    let opened = AggregateState::default().opened_from(&entry);
    assert!(!opened.is_modified());
    let changed = applied(opened, OptionsEdit::AllowDiskUse(Tri::On));
    assert!(changed.is_modified());
    let back = applied(changed, OptionsEdit::AllowDiskUse(Tri::Default));
    assert!(!back.is_modified());
}

#[test]
fn a_blocked_form_counts_as_modified() {
    let entry = entry_of(&two(), PipelineOptions::default());
    let opened = AggregateState::default().opened_from(&entry);
    let blocked = opened.options_edited(OptionsEdit::Comment("   ".into()));
    assert!(blocked.is_modified());
    let fixed = blocked.options_edited(OptionsEdit::Comment(String::new()));
    assert!(!fixed.is_modified());
}

#[test]
fn an_unsaved_draft_with_only_options_is_modified() {
    let draft = applied(
        AggregateState::default(),
        OptionsEdit::AllowDiskUse(Tri::On),
    );
    assert!(draft.stages.is_empty());
    assert!(draft.is_modified());
    assert!(!AggregateState::default().is_modified());
}

#[test]
fn saved_as_takes_the_options_as_the_baseline() {
    let draft = applied(two(), OptionsEdit::AllowDiskUse(Tri::On));
    assert!(draft.is_modified());
    let saved = draft.saved_as(Uuid::new_v4(), "totals");
    assert_eq!(saved.saved.baseline_options, saved.options);
    assert!(!saved.is_modified());
    assert_eq!(saved.as_saved_draft().options, saved.options);
}

#[test]
fn the_modified_memo_is_forgotten_by_an_options_edit() {
    let entry = entry_of(&two(), PipelineOptions::default());
    let opened = AggregateState::default().opened_from(&entry);
    assert!(!opened.is_modified());
    assert_eq!(opened.saved.modified_evaluations(), 1);
    let edited = opened.options_edited(OptionsEdit::Comment("   ".into()));
    assert!(edited.is_modified(), "answered again, not remembered");
    assert_eq!(edited.saved.modified_evaluations(), 1);
}

#[test]
fn deleting_the_opened_pipeline_leaves_an_unsaved_draft_with_its_options() {
    let entry = entry_of(&two(), options());
    let opened = AggregateState::default().opened_from(&entry);
    let orphan = opened.without_opened();
    assert_eq!(orphan.options, options());
    assert!(orphan.is_modified(), "nobody saved it any more");
}
