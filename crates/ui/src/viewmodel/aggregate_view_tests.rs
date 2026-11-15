use super::*;
use crate::I18n;
use crate::viewmodel::aggregate::AggregateState;
use bson::doc;
use curlybrace_core::error::CoreError;

#[test]
fn a_typed_edit_waits_for_the_pause_then_is_taken_once() {
    let (view, displaced) = AggregateView::default().edited(1, "{}".into(), 10.0);
    assert_eq!(displaced, None);
    assert_eq!(view.shown_text(1, "old"), "{}");
    assert_eq!(
        view.shown_text(0, "old"),
        "old",
        "another stage shows its own text"
    );
    let (view, early) = view.take_due(10.0 + EDIT_PAUSE_SECS / 2.0, false);
    assert_eq!(early, None);
    assert!(view.due_in(10.0).is_some());
    let (view, due) = view.take_due(10.0 + EDIT_PAUSE_SECS, false);
    assert_eq!(due, Some((1, "{}".to_string())));
    assert_eq!(view.pending, None);
    assert_eq!(view.take_due(99.0, true).1, None, "nothing left to take");
}

#[test]
fn another_action_takes_the_waiting_edit_at_once() {
    let (view, _) = AggregateView::default().edited(0, "x".into(), 5.0);
    assert_eq!(view.take_due(5.0, true).1, Some((0, "x".to_string())));
}

#[test]
fn typing_in_another_stage_hands_the_waiting_edit_back() {
    let (view, _) = AggregateView::default().edited(0, "a".into(), 1.0);
    let (view, displaced) = view.edited(1, "b".into(), 1.1);
    assert_eq!(displaced, Some((0, "a".to_string())));
    assert_eq!(view.shown_text(1, "old"), "b");
    let (view, displaced) = view.edited(1, "bc".into(), 1.2);
    assert_eq!(displaced, None, "more typing in the same stage replaces it");
    assert_eq!(view.take_due(9.0, false).1, Some((1, "bc".to_string())));
}

#[test]
fn a_carried_view_keeps_the_typing_and_the_mode_only() {
    let view = AggregateView {
        mode: ViewMode::Json,
        auto_sent: Some(3),
        ..AggregateView::default()
    }
    .edited(0, "x".into(), 1.0)
    .0;
    let carried = view.carried();
    assert_eq!(carried.mode, ViewMode::Json);
    assert_eq!(carried.auto_sent, None);
    assert_eq!(carried.pending.map(|p| p.text), Some("x".to_string()));
}

#[test]
fn the_page_is_rebuilt_only_when_the_documents_change() {
    let mut cache = PageCache::default();
    let a = vec![doc! { "a": 1 }];
    let first = cache.of(&a).1;
    assert_eq!(cache.of(&a).1, first);
    let (page, key) = cache.of(&[doc! { "a": 2 }]);
    assert_eq!(page.docs, vec![doc! { "a": 2 }]);
    assert!(key != first);
    assert!(!page.has_more && page.skip == 0);
}

#[test]
fn the_pane_shows_the_current_or_the_last_good_preview() {
    let preview = Preview {
        docs: vec![doc! { "a": 1 }],
        not_run: Vec::new(),
    };
    assert_eq!(shown_preview(&PreviewState::Idle), None);
    assert_eq!(
        shown_preview(&PreviewState::Shown(preview.clone())),
        Some(&preview)
    );
    let failed = PreviewState::Failed {
        error: crate::viewmodel::aggregate::PreviewError::Core(CoreError::Unreachable),
        last: Some(preview.clone()),
    };
    assert_eq!(shown_preview(&failed), Some(&preview));
    let loading = PreviewState::Loading {
        not_run: Vec::new(),
        last: None,
    };
    assert_eq!(shown_preview(&loading), None);
}

#[test]
fn a_stage_status_names_the_stage_or_says_what_is_wrong() {
    assert_eq!(
        stage_status(r#"{ "$match": {} }"#),
        StageStatus::Valid("$match".into())
    );
    assert_eq!(stage_status(r#"{ "a": 1 }"#), StageStatus::NotAStage);
    assert_eq!(
        stage_status(r#"{ "$a": 1, "$b": 2 }"#),
        StageStatus::NotAStage
    );
    assert_eq!(stage_status("[]"), StageStatus::NotAStage);
    match stage_status("{\n  \"$match\": \n}") {
        StageStatus::Syntax {
            line,
            column,
            message,
        } => {
            assert_eq!(line, 3);
            assert!(column >= 1);
            assert!(!message.contains(" at line "), "{message}");
        }
        other => panic!("expected a syntax error, got {other:?}"),
    }
}

#[test]
fn a_deeply_nested_stage_is_an_error_not_a_panic() {
    let deep = format!("{}{}", "[".repeat(5_000), "]".repeat(5_000));
    let text = format!("{{ \"$match\": {deep} }}");
    assert!(matches!(stage_status(&text), StageStatus::Syntax { .. }));
}

#[test]
fn every_pipeline_error_has_a_filled_message() {
    let i18n = I18n::english();
    let errors = [
        PipelineError::Syntax {
            line: 2,
            column: 7,
            message: "expected value at line 2 column 7".into(),
        },
        PipelineError::NotAnArray,
        PipelineError::NotAStage { index: 1 },
        PipelineError::WriteStageNotLast {
            index: 0,
            name: "$out".into(),
        },
        PipelineError::NestedWriteStage {
            index: 2,
            name: "$merge".into(),
        },
        PipelineError::Empty,
    ];
    for error in errors {
        let (key, params) = pipeline_error_message(&error);
        let params: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let text = i18n.t_with(key, &params);
        assert!(
            !text.starts_with('⟦') && !text.contains('{'),
            "{key}: {text}"
        );
    }
    let (_, params) = pipeline_error_message(&PipelineError::NotAStage { index: 1 });
    assert_eq!(
        params,
        vec![("n", "2".to_string())],
        "positions are 1-based"
    );
    let syntax = PipelineError::Syntax {
        line: 2,
        column: 7,
        message: "expected value at line 2 column 7".into(),
    };
    let (key, params) = pipeline_error_message(&syntax);
    let params: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
    assert_eq!(
        i18n.t_with(key, &params),
        "line 2, column 7: expected value"
    );
}

#[test]
fn an_idle_draft_asks_once_and_a_view_remembers_it() {
    let draft = AggregateState::default().add_stage("$match");
    let view = AggregateView::default();
    assert!(draft.wants_auto_refresh(view.auto_sent));
    let view = AggregateView {
        auto_sent: Some(draft.request),
        ..view
    };
    assert!(!draft.wants_auto_refresh(view.auto_sent));
}

#[test]
fn a_timed_out_preview_has_its_own_message_other_errors_keep_the_generic_one() {
    use curlybrace_core::runtime::COMMAND_TIMEOUT_MESSAGE;
    let timeout = CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into());
    let key = preview_error_key(&timeout).expect("timeout key");
    assert_eq!(key, "aggregate.error.timeout");
    let text = I18n::english().t(key);
    assert!(text.contains("took too long") && text.contains("last result"));
    assert_eq!(preview_error_key(&CoreError::Database("boom".into())), None);
    assert_eq!(preview_error_key(&CoreError::Unreachable), None);
}

#[test]
fn a_status_is_computed_again_only_when_its_text_changes() {
    let mut cache = StatusCache::default();
    let a = r#"{"$match": {}}"#;
    assert_eq!(cache.of(1, a), StageStatus::Valid("$match".into()));
    assert_eq!(cache.of(1, a), StageStatus::Valid("$match".into()));
    assert_eq!(
        cache.computed(),
        1,
        "the second frame is served from the cache"
    );
    assert_eq!(cache.of(2, a), StageStatus::Valid("$match".into()));
    assert_eq!(cache.computed(), 2, "another stage has its own entry");
    assert_eq!(cache.of(1, "{ bad"), stage_status("{ bad"));
    assert_eq!(cache.computed(), 3, "changed text is parsed again");
    cache.keeping(&[2]);
    assert_eq!(cache.of(1, a), StageStatus::Valid("$match".into()));
    assert_eq!(cache.computed(), 4, "a removed stage's entry is gone");
}

#[test]
fn the_status_shown_follows_what_is_being_typed() {
    let (mut view, _) = AggregateView::default().edited(0, "{ bad".into(), 1.0);
    assert_eq!(
        view.status_of(0, 7, r#"{"$match": {}}"#),
        stage_status("{ bad")
    );
    assert_eq!(
        view.status_of(1, 8, r#"{"$match": {}}"#),
        StageStatus::Valid("$match".into())
    );
}

#[test]
fn the_typed_stages_borrow_the_draft_unless_an_edit_waits() {
    let draft = AggregateState::default()
        .add_stage("$match")
        .edit(0, r#"{"$match": {}}"#);
    let idle = AggregateView::default();
    assert!(matches!(idle.typed_stages(&draft), Cow::Borrowed(_)));

    let (waiting, _) = AggregateView::default().edited(0, "{ bad".into(), 1.0);
    let typed = waiting.typed_stages(&draft);
    assert_eq!(typed[0].text, "{ bad");
    assert_eq!(
        draft.stages[0].text, r#"{"$match": {}}"#,
        "the draft is untouched"
    );

    let (same, _) = AggregateView::default().edited(0, draft.stages[0].text.clone(), 1.0);
    assert!(matches!(same.typed_stages(&draft), Cow::Borrowed(_)));
    let (beyond, _) = AggregateView::default().edited(9, "x".into(), 1.0);
    assert!(matches!(beyond.typed_stages(&draft), Cow::Borrowed(_)));
}
