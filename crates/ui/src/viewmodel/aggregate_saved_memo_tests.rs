//! The "modified" mark is asked on every repaint: the answer is remembered until the draft
//! changes, and every change of the draft (or of what it is compared with) forgets it.
use super::*;
use curlybrace_core::saved_pipeline::SavedPipelines;
use uuid::Uuid;

fn opened() -> (AggregateState, SavedPipeline) {
    let conn = Uuid::new_v4();
    let draft = AggregateState::default()
        .add_stage("$match")
        .add_stage("$sort");
    let store = SavedPipelines::default()
        .saved(conn, "totals", None, draft.as_saved_stages(), 1, false)
        .unwrap();
    let entry = store.list(conn)[0].clone();
    (AggregateState::default().opened_from(&entry), entry)
}

#[test]
fn a_repaint_with_an_unchanged_draft_does_not_compare_again() {
    let (draft, _) = opened();
    for _ in 0..5 {
        assert!(!draft.is_modified());
    }
    assert_eq!(draft.saved.modified_evaluations(), 1);
}

#[test]
fn a_dialog_or_an_error_does_not_make_the_draft_be_compared_again() {
    let (draft, _) = opened();
    assert!(!draft.is_modified());
    let draft = draft
        .with_saved_dialog(Some(SavedDialog::Save {
            name: "x".into(),
            replace_prompt: false,
        }))
        .with_saved_error("k");
    assert!(!draft.is_modified());
    assert_eq!(draft.saved.modified_evaluations(), 1);
}

/// What `is_modified` must say, worked out without the memo.
fn fresh(draft: &AggregateState) -> bool {
    draft.modified_uncached()
}

/// The memo is filled, `change` is applied, and the answer is still the true one.
fn after(change: impl FnOnce(AggregateState) -> AggregateState, start: AggregateState) -> bool {
    start.is_modified();
    let next = change(start);
    let said = next.is_modified();
    assert_eq!(said, fresh(&next), "a stale answer");
    said
}

#[test]
fn every_change_of_the_draft_forgets_the_answer() {
    let (start, entry) = opened();
    assert!(after(|d| d.add_stage("$limit"), start.clone()));
    assert!(after(|d| d.remove(0), start.clone()));
    assert!(after(|d| d.toggle(0), start.clone()));
    assert!(after(|d| d.move_down(0), start.clone()));
    assert!(after(
        |d| d.edit(0, "{\"$match\": {\"a\": 1}}"),
        start.clone()
    ));
    // Back to the same text: unmodified again, and the memo says so.
    let edited = start.clone().edit(0, "{\"$match\": {\"a\": 1}}");
    let original = entry.stages[0].text.clone();
    assert!(!after(|d| d.edit(0, &original), edited));
    // Text mode: an edit that parses differently, one that does not parse, and the way back.
    let text = start.clone().to_text();
    assert!(!after(|d| d, text.clone()));
    assert!(after(|d| d.with_text("[{\"$limit\": 3}]"), text.clone()));
    assert!(after(|d| d.with_text("[{"), text.clone()));
    let edited = text.clone().with_text("[{\"$limit\": 3}]");
    assert!(after(|d| d.from_text(), edited));
    assert!(!after(|d| d.from_text(), text));
}

#[test]
fn saving_opening_and_forgetting_forget_the_answer_too() {
    let (start, entry) = opened();
    let edited = start.clone().add_stage("$limit");
    assert!(after(|d| d, edited.clone()));
    // Saved as the same entry: the baseline is now the draft.
    assert!(!after(|d| d.saved_as(entry.id, "totals"), edited.clone()));
    // Opened again: back to the entry.
    assert!(!after(|d| d.opened_from(&entry), edited.clone()));
    // The opened pipeline is gone: an unsaved draft with stages is modified.
    assert!(after(|d| d.without_opened(), start));
}
