use super::*;
use curlybrace_core::saved_pipeline::SavedPipelines;
use uuid::Uuid;

fn two() -> AggregateState {
    AggregateState::default()
        .add_stage("$match")
        .add_stage("$sort")
}

/// The draft saved under "totals" and opened from that entry, so it is unmodified.
fn opened() -> (AggregateState, SavedPipeline) {
    let conn = Uuid::new_v4();
    let store = SavedPipelines::default()
        .saved(conn, "totals", None, two().as_saved_stages(), 1, false)
        .unwrap();
    let entry = store.list(conn)[0].clone();
    (AggregateState::default().opened_from(&entry), entry)
}

#[test]
fn an_empty_draft_is_not_modified() {
    assert!(!AggregateState::default().is_modified());
}

#[test]
fn a_draft_with_stages_that_was_never_saved_or_opened_is_modified() {
    assert!(two().is_modified());
}

#[test]
fn opening_a_pipeline_leaves_it_unmodified_with_its_name() {
    let (draft, entry) = opened();
    assert!(!draft.is_modified());
    assert_eq!(draft.saved.opened, Some((entry.id, "totals".to_string())));
    assert_eq!(draft.as_saved_stages(), entry.stages);
}

#[test]
fn edit_toggle_move_add_and_remove_mark_it_modified() {
    let (draft, _) = opened();
    assert!(
        draft
            .clone()
            .edit(0, r#"{"$match": {"a": 1}}"#)
            .is_modified()
    );
    assert!(draft.clone().toggle(1).is_modified());
    assert!(draft.clone().move_down(0).is_modified());
    assert!(draft.clone().add_stage("$limit").is_modified());
    assert!(draft.remove(1).is_modified());
}

#[test]
fn undoing_a_change_clears_the_mark() {
    let (draft, _) = opened();
    let back = draft.clone().toggle(1).toggle(1);
    assert!(!back.is_modified());
    let moved = draft.move_down(0).move_up(1);
    assert!(!moved.is_modified());
}

#[test]
fn a_stages_text_stages_round_trip_of_unchanged_text_is_not_modified() {
    let (draft, _) = opened();
    let round = draft.to_text().from_text();
    assert!(!round.is_modified());
}

#[test]
fn unsynced_text_edits_count_as_modified() {
    let (draft, _) = opened();
    let edited = draft.to_text().with_text(r#"[{"$match": {"b": 2}}]"#);
    assert!(edited.is_modified());
    let broken = edited.with_text("[{");
    assert!(broken.is_modified());
}

#[test]
fn opening_replaces_the_draft_with_unique_keys_and_an_idle_preview() {
    let (before, entry) = opened();
    let (loaded, _) = before.begin_preview();
    let loaded = loaded.add_stage("$limit");
    let old_keys = loaded.keys.clone();
    let next = loaded.opened_from(&entry);
    assert_eq!(next.stages.len(), 2);
    assert_eq!(next.keys.len(), 2);
    assert!(next.keys.iter().all(|k| !old_keys.contains(k)));
    assert_ne!(next.keys[0], next.keys[1]);
    assert_eq!(next.preview, PreviewState::Idle);
    assert_eq!(next.mode, Mode::Stages);
    assert_eq!(next.selected, 1);
    assert!(next.queued.is_none());
}

#[test]
fn opening_makes_an_answer_still_in_flight_stale_and_keeps_the_slot_rules() {
    let (draft, entry) = opened();
    let (draft, plan) = draft.begin_preview();
    let (request, pipeline) = plan.unwrap();
    let (draft, sent) = draft.offered(
        request,
        curlybrace_core::store::Namespace::new("a", "b"),
        pipeline,
    );
    assert!(sent.is_some());
    let next = draft.opened_from(&entry);
    assert_eq!(next.in_flight, Some(request));
    assert_ne!(next.request, request);
    let answered = next.arrived(request, Ok(Vec::new()));
    assert_eq!(answered.preview, PreviewState::Idle);
    let (freed, resend) = answered.released(request, None);
    assert_eq!(freed.in_flight, None);
    assert!(resend.is_none());
}

#[test]
fn opening_from_text_mode_returns_to_stages() {
    let (draft, entry) = opened();
    let next = draft.to_text().opened_from(&entry);
    assert_eq!(next.mode, Mode::Stages);
    assert!(next.text_error.is_none());
    assert!(!next.is_modified());
}

#[test]
fn dialogs_open_and_close_without_touching_the_draft() {
    let (draft, _) = opened();
    let id = Uuid::new_v4();
    for dialog in [
        SavedDialog::Save {
            name: "x".into(),
            replace_prompt: false,
        },
        SavedDialog::ConfirmOpen { id },
        SavedDialog::Rename {
            id,
            name: "y".into(),
        },
        SavedDialog::ConfirmDelete { id },
    ] {
        let shown = draft.clone().with_saved_dialog(Some(dialog));
        assert!(shown.saved.dialog.is_some());
        assert_eq!(shown.stages, draft.stages);
        assert_eq!(shown.keys, draft.keys);
        assert_eq!(shown.request, draft.request);
        let closed = shown.with_saved_dialog(None);
        assert_eq!(closed, draft);
    }
}

#[test]
fn a_new_dialog_or_a_close_clears_the_inline_error() {
    let s = SavedUi::default()
        .with_error("aggregate.saved.error.name_taken")
        .with_dialog(None);
    assert_eq!(s.error, None);
    let s = SavedUi::default().with_error("k");
    assert_eq!(s.error, Some("k"));
}

#[test]
fn saving_marks_the_draft_opened_and_unmodified() {
    let draft = two();
    let id = Uuid::new_v4();
    let saved = draft.saved_as(id, "totals");
    assert!(!saved.is_modified());
    assert_eq!(saved.saved.opened, Some((id, "totals".to_string())));
    assert!(
        saved
            .clone()
            .edit(0, "{\"$match\":{\"z\":1}}")
            .is_modified()
    );
}

#[test]
fn forgetting_the_opened_pipeline_keeps_the_draft() {
    let (draft, _) = opened();
    let forgotten = draft.clone().without_opened();
    assert_eq!(forgotten.stages, draft.stages);
    assert_eq!(forgotten.saved.opened, None);
    assert_eq!(forgotten.saved.baseline, None);
    assert!(forgotten.is_modified());
}

#[test]
fn the_saved_state_survives_a_connection_reset() {
    let (draft, _) = opened();
    let draft = draft.with_saved_dialog(Some(SavedDialog::ConfirmOpen { id: Uuid::new_v4() }));
    let reset = draft.clone().on_connection_reset();
    assert_eq!(reset.saved, draft.saved);
}
