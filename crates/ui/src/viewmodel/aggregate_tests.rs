use super::*;
use bson::doc;
use curlybrace_core::pipeline::{PREVIEW_LIMIT, stage_name};

/// A draft holding `$match`, `$sort`, `$limit` with the last one selected.
fn three() -> AggregateState {
    AggregateState::default()
        .add_stage("$match")
        .add_stage("$sort")
        .add_stage("$limit")
}

fn names(s: &AggregateState) -> Vec<String> {
    s.stages
        .iter()
        .map(|st| stage_name(&st.text).unwrap_or_default())
        .collect()
}

fn loading() -> (AggregateState, u64) {
    let (s, sent) = three().begin_preview();
    (s, sent.unwrap().0)
}

#[test]
fn default_is_an_empty_stage_list() {
    let s = AggregateState::default();
    assert!(s.stages.is_empty());
    assert_eq!(s.selected, 0);
    assert_eq!(s.mode, Mode::Stages);
    assert_eq!(s.preview, PreviewState::Idle);
    assert_eq!(s.request, 0);
}

#[test]
fn every_menu_template_parses_as_its_stage() {
    for name in [
        "$match",
        "$group",
        "$project",
        "$sort",
        "$limit",
        "$skip",
        "$unwind",
        "$lookup",
        "$addFields",
        "$count",
        "$facet",
        "$out",
        "$merge",
    ] {
        assert!(STAGE_MENU.contains(&name), "{name} missing from the menu");
    }
    for name in STAGE_MENU {
        let s = AggregateState::default().add_stage(name);
        assert_eq!(stage_name(&s.stages[0].text).as_deref(), Some(name));
        assert!(s.stages[0].enabled);
    }
}

#[test]
fn match_template_is_an_empty_filter() {
    let s = AggregateState::default().add_stage("$match");
    let v: serde_json::Value = serde_json::from_str(&s.stages[0].text).unwrap();
    assert_eq!(v, serde_json::json!({"$match": {}}));
}

#[test]
fn add_stage_appends_and_selects_it() {
    let s = three();
    assert_eq!(names(&s), ["$match", "$sort", "$limit"]);
    assert_eq!(s.selected, 2);
}

#[test]
fn add_does_not_change_the_original() {
    let before = three();
    let _ = before.clone().add_stage("$skip");
    assert_eq!(before.stages.len(), 3);
}

#[test]
fn remove_drops_the_stage_and_clamps_the_selection() {
    let s = three().remove(2);
    assert_eq!(names(&s), ["$match", "$sort"]);
    assert_eq!(s.selected, 1);
    let s = s.remove(0).remove(0);
    assert!(s.stages.is_empty());
    assert_eq!(s.selected, 0);
}

#[test]
fn remove_before_the_selection_keeps_the_same_stage_selected() {
    let s = three().select(2).remove(0);
    assert_eq!(names(&s), ["$sort", "$limit"]);
    assert_eq!(s.selected, 1);
}

#[test]
fn remove_after_the_selection_keeps_it() {
    let s = three().select(0).remove(1);
    assert_eq!(s.selected, 0);
}

#[test]
fn remove_out_of_range_changes_nothing() {
    assert_eq!(three().remove(9), three());
}

#[test]
fn move_up_swaps_and_the_selection_follows() {
    let s = three().select(1).move_up(1);
    assert_eq!(names(&s), ["$sort", "$match", "$limit"]);
    assert_eq!(s.selected, 0);
}

#[test]
fn move_up_of_the_stage_above_the_selection_keeps_the_selected_stage() {
    let s = three().select(0).move_up(1);
    assert_eq!(names(&s), ["$sort", "$match", "$limit"]);
    assert_eq!(s.selected, 1);
}

#[test]
fn move_down_swaps_and_the_selection_follows() {
    let s = three().select(0).move_down(0);
    assert_eq!(names(&s), ["$sort", "$match", "$limit"]);
    assert_eq!(s.selected, 1);
}

#[test]
fn moves_at_the_edges_change_nothing() {
    let s = three();
    assert_eq!(names(&s.clone().move_up(0)), names(&s));
    assert_eq!(names(&s.clone().move_down(2)), names(&s));
    assert_eq!(names(&s.clone().move_down(7)), names(&s));
    assert_eq!(s.clone().move_up(0).request, s.request);
}

#[test]
fn toggle_flips_enabled_and_keeps_the_selection_valid() {
    let s = three().select(1).toggle(1);
    assert!(!s.stages[1].enabled);
    assert_eq!(s.selected, 1);
    let s = s.toggle(1);
    assert!(s.stages[1].enabled);
    assert_eq!(three().toggle(5), three());
}

#[test]
fn edit_replaces_the_text() {
    let s = three().edit(0, r#"{"$match": {"a": 1}}"#);
    assert_eq!(s.stages[0].text, r#"{"$match": {"a": 1}}"#);
    assert_eq!(three().edit(9, "x"), three());
}

#[test]
fn select_clamps_to_the_last_stage() {
    assert_eq!(three().select(0).selected, 0);
    assert_eq!(three().select(99).selected, 2);
    assert_eq!(AggregateState::default().select(3).selected, 0);
}

#[test]
fn text_round_trips_through_text_mode() {
    let s = three().edit(0, r#"{"$match":{"a":1}}"#).toggle(1);
    let t = s.clone().to_text();
    assert_eq!(t.mode, Mode::Text);
    assert!(t.text.starts_with('['));
    let back = t.from_text();
    assert_eq!(back.mode, Mode::Stages);
    assert_eq!(back.text_error, None);
    assert_eq!(names(&back), names(&s));
    assert_eq!(stage_name(&back.stages[0].text).as_deref(), Some("$match"));
    assert_eq!(back.selected, 2);
}

#[test]
fn unchanged_text_returns_to_the_same_stages_disabled_ones_included() {
    let s = three().edit(0, r#"{"$match":{"a":1}}"#).toggle(1).select(0);
    let back = s.clone().to_text().from_text();
    assert_eq!(back.mode, Mode::Stages);
    assert_eq!(back.stages, s.stages, "texts and enabled flags kept");
    assert_eq!(back.selected, 0, "the selection is kept");
    assert_eq!(
        back.request, s.request,
        "nothing changed, nothing to preview"
    );
}

#[test]
fn edited_text_turns_every_stage_on() {
    let t = three().toggle(1).to_text();
    let text = t.text.replace("10", "5");
    let back = t.with_text(&text).from_text();
    assert!(back.stages.iter().all(|st| st.enabled));
    assert_eq!(back.selected, 2);
}

#[test]
fn edited_text_replaces_the_stages() {
    let s = three().to_text().with_text(r#"[{"$skip": 1}]"#).from_text();
    assert_eq!(names(&s), ["$skip"]);
    assert_eq!(s.selected, 0);
}

#[test]
fn invalid_text_blocks_leaving_text_mode_with_line_and_column() {
    let t = three().to_text().with_text("[\n  {\"$match\": }\n]");
    let s = t.from_text();
    assert_eq!(s.mode, Mode::Text);
    assert_eq!(names(&s), ["$match", "$sort", "$limit"]);
    match s.text_error {
        Some(PipelineError::Syntax { line, column, .. }) => {
            assert_eq!(line, 2);
            assert!(column > 1);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(s.text, "[\n  {\"$match\": }\n]");
}

#[test]
fn text_that_is_not_an_array_blocks_too() {
    let s = three().to_text().with_text("{}").from_text();
    assert_eq!(s.mode, Mode::Text);
    assert_eq!(s.text_error, Some(PipelineError::NotAnArray));
}

#[test]
fn begin_preview_sends_a_numbered_plan_for_the_selected_stage() {
    let s = three().edit(1, r#"{"$sort": {"a": 1}}"#).select(1);
    let before = s.request;
    let (s, sent) = s.begin_preview();
    let (request, pipeline) = sent.unwrap();
    assert_eq!(request, before + 1);
    assert_eq!(s.request, request);
    assert_eq!(
        pipeline,
        vec![
            doc! {"$match": {}},
            doc! {"$sort": {"a": 1}},
            doc! {"$limit": PREVIEW_LIMIT}
        ]
    );
    assert!(matches!(s.preview, PreviewState::Loading { .. }));
}

#[test]
fn begin_preview_of_an_invalid_pipeline_fails_without_a_request() {
    let (s, sent) = AggregateState::default()
        .add_stage("$match")
        .edit(0, "{ bad")
        .begin_preview();
    assert!(sent.is_none());
    assert_eq!(
        s.preview,
        PreviewState::Failed {
            error: PreviewError::Pipeline(PipelineError::NotAStage { index: 0 }),
            last: None
        }
    );
}

#[test]
fn an_empty_or_all_disabled_pipeline_resets_the_preview_to_idle_without_old_documents() {
    let (s, request) = loading();
    let shown = s.arrived(request, Ok(vec![doc! {"a": 1}]));
    for emptied in [
        shown.clone().remove(2).remove(1).remove(0),
        shown.clone().toggle(0).toggle(1).toggle(2),
    ] {
        let (s, sent) = emptied.begin_preview();
        assert!(sent.is_none());
        assert_eq!(s.preview, PreviewState::Idle);
    }
}

#[test]
fn an_idle_pane_with_nothing_to_run_does_not_ask_for_a_refresh() {
    let s = three().toggle(0).toggle(1).toggle(2);
    assert!(!s.wants_auto_refresh(None));
    let (s, _) = s.begin_preview();
    assert!(!s.wants_auto_refresh(None));
    let s = s.toggle(1);
    assert!(
        s.wants_auto_refresh(None),
        "an enabled stage previews again"
    );
    assert!(!AggregateState::default().wants_auto_refresh(None));
}

#[test]
fn a_failed_plan_keeps_the_last_good_preview_and_drops_in_flight_answers() {
    let (s, request) = loading();
    let s = s.arrived(request, Ok(vec![doc! {"a": 1}]));
    let (s, _) = s.edit(0, "{ bad").begin_preview();
    match &s.preview {
        PreviewState::Failed {
            error: PreviewError::Pipeline(PipelineError::NotAStage { index: 0 }),
            last: Some(last),
        } => assert_eq!(last.docs, vec![doc! {"a": 1}]),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_trailing_write_stage_is_reported_not_run() {
    let (s, sent) = three().add_stage("$out").begin_preview();
    let (request, pipeline) = sent.unwrap();
    assert!(!pipeline.iter().any(|d| d.contains_key("$out")));
    let s = s.arrived(request, Ok(vec![]));
    assert_eq!(
        s.preview,
        PreviewState::Shown(Preview {
            docs: vec![],
            not_run: vec!["$out".to_string()]
        })
    );
}

#[test]
fn arrived_shows_the_documents() {
    let (s, request) = loading();
    let s = s.arrived(request, Ok(vec![doc! {"n": 1}]));
    assert_eq!(
        s.preview,
        PreviewState::Shown(Preview {
            docs: vec![doc! {"n": 1}],
            not_run: vec![]
        })
    );
}

#[test]
fn a_stale_answer_is_ignored() {
    let (s, first) = loading();
    let (s, second) = s.begin_preview();
    let second = second.unwrap().0;
    assert!(second > first);
    let s = s.arrived(first, Ok(vec![doc! {"old": 1}]));
    assert!(matches!(s.preview, PreviewState::Loading { .. }));
    let s = s.arrived(second, Ok(vec![doc! {"new": 1}]));
    assert!(matches!(&s.preview, PreviewState::Shown(p) if p.docs == vec![doc! {"new": 1}]));
}

#[test]
fn an_answer_after_a_pipeline_change_is_stale() {
    type Change = fn(AggregateState) -> AggregateState;
    let changes: [Change; 6] = [
        |s| s.edit(0, r#"{"$match": {"b": 2}}"#),
        |s| s.toggle(0),
        |s| s.move_down(0),
        |s| s.remove(0),
        |s| s.add_stage("$skip"),
        |s| s.select(0),
    ];
    for change in changes {
        let (s, request) = loading();
        let s = change(s).arrived(request, Ok(vec![doc! {"old": 1}]));
        assert!(
            !matches!(s.preview, PreviewState::Shown(_)),
            "{:?}",
            s.preview
        );
    }
}

#[test]
fn an_answer_already_shown_is_not_applied_twice() {
    let (s, request) = loading();
    let s = s.arrived(request, Ok(vec![doc! {"n": 1}]));
    let again = s.clone().arrived(request, Ok(vec![doc! {"n": 2}]));
    assert_eq!(again, s);
}

#[test]
fn an_error_keeps_the_last_good_preview() {
    let (s, request) = loading();
    let s = s.arrived(request, Ok(vec![doc! {"n": 1}]));
    let (s, sent) = s.begin_preview();
    let s = s.arrived(sent.unwrap().0, Err(CoreError::Unreachable));
    match &s.preview {
        PreviewState::Failed {
            error: PreviewError::Core(CoreError::Unreachable),
            last: Some(last),
        } => assert_eq!(last.docs, vec![doc! {"n": 1}]),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_error_with_nothing_shown_before_has_no_last() {
    let (s, request) = loading();
    let s = s.arrived(request, Err(CoreError::Unreachable));
    assert!(matches!(s.preview, PreviewState::Failed { last: None, .. }));
}

#[test]
fn begin_preview_does_not_change_the_stages() {
    let s = three();
    let (next, _) = s.clone().begin_preview();
    assert_eq!(next.stages, s.stages);
    assert_eq!(next.selected, s.selected);
}

#[test]
fn a_collection_change_makes_a_preview_stale_and_idle() {
    let (s, request) = loading();
    let moved = s.on_collection_changed();
    assert_eq!(moved.preview, PreviewState::Idle);
    assert!(moved.request != request);
    let late = moved.clone().arrived(request, Ok(vec![doc! { "a": 1 }]));
    assert_eq!(late, moved, "the answer for the old collection is stale");
    assert_eq!(
        names(&late),
        ["$match", "$sort", "$limit"],
        "the draft stays"
    );
}

#[test]
fn a_collection_change_drops_a_shown_preview() {
    let (s, request) = loading();
    let shown = s.arrived(request, Ok(vec![doc! { "a": 1 }]));
    assert!(matches!(shown.preview, PreviewState::Shown(_)));
    assert_eq!(shown.on_collection_changed().preview, PreviewState::Idle);
}

#[test]
fn an_automatic_refresh_is_wanted_once_per_request_for_an_idle_non_empty_draft() {
    let idle = three();
    assert!(idle.wants_auto_refresh(None));
    assert!(!idle.wants_auto_refresh(Some(idle.request)));
    assert!(idle.wants_auto_refresh(Some(idle.request.wrapping_sub(1))));
    assert!(!AggregateState::default().wants_auto_refresh(None), "empty");
    let (loading, _) = loading();
    assert!(!loading.wants_auto_refresh(None), "in flight");
    let (failed, sent) = AggregateState::default()
        .add_stage("$match")
        .edit(0, "{ \"$match\": ")
        .begin_preview();
    assert!(sent.is_none());
    assert!(!failed.wants_auto_refresh(None), "failed stays until asked");
}
