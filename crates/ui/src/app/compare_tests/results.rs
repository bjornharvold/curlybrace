//! Result rows, paging, categories and the per-document detail.
use super::*;

#[test]
fn pair_done_requests_the_first_page_and_fresh_rows_fill_it() {
    let state = with_rows();
    assert_eq!(state.compare.results.as_ref().unwrap().rows.len(), 3);
}

#[test]
fn stale_rows_are_dropped_by_the_reducer() {
    let state = with_differences();
    let (state, _) = update(state, rows_event(Category::OnlyInSource, 0, 3));
    let (state, _) = update(state, rows_event(Category::Different, 100, 3));
    assert!(state.compare.results.unwrap().rows.is_empty());
}

#[test]
fn a_failed_rows_request_keeps_the_current_request() {
    let failed = ev(CompareEvent::Failed {
        session: 1,
        context: "compare_rows",
        error: CoreError::NotFound,
    });
    let (state, _) = update(with_differences(), failed);
    let results = state.compare.results.unwrap();
    assert_eq!(results.requested, Some((Category::Different, 0)));
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "compare_rows",
            ..
        })
    ));
}

#[test]
fn paging_category_and_select_send_the_matching_commands() {
    let (state, cmds) = act(with_rows(), CompareAction::Page(1));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Rows { offset: 100, .. })]
    ));
    let (state, cmds) = act(state, CompareAction::SelectCategory(Category::OnlyInSource));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Rows {
            category: Category::OnlyInSource,
            offset: 0,
            ..
        })]
    ));
    let (_, cmds) = act(state, CompareAction::Page(-1));
    assert!(cmds.is_empty(), "already at the first page");
}

#[test]
fn select_requests_detail_and_back_and_the_answer_fill_the_view() {
    let key = KeyTuple(vec![Bson::Int32(1)]);
    let (state, cmds) = act(with_rows(), CompareAction::Select(key.clone()));
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Detail {
            session: 1,
            pair: 1,
            ..
        })]
    ));
    assert!(state.compare.detail.as_ref().unwrap().loading);
    let answer = CompareEvent::Detail {
        session: 1,
        pair: 1,
        key,
        source: Some(bson::doc! {"a": 1}),
        target: Some(bson::doc! {"a": 2}),
        changes: vec![],
    };
    let (state, _) = update(state, ev(answer));
    assert!(!state.compare.detail.as_ref().unwrap().loading);
    let (state, _) = act(state, CompareAction::Back);
    assert!(state.compare.detail.is_none());
}

#[test]
fn a_detail_answer_for_another_key_is_dropped() {
    let (state, _) = act(
        with_rows(),
        CompareAction::Select(KeyTuple(vec![Bson::Int32(1)])),
    );
    let answer = CompareEvent::Detail {
        session: 1,
        pair: 1,
        key: KeyTuple(vec![Bson::Int32(2)]),
        source: None,
        target: None,
        changes: vec![],
    };
    let (state, _) = update(state, ev(answer));
    assert!(state.compare.detail.unwrap().loading);
}

#[test]
fn selecting_an_unlisted_key_does_nothing() {
    let key = KeyTuple(vec![Bson::Int32(99)]);
    let (state, cmds) = act(with_rows(), CompareAction::Select(key));
    assert!(cmds.is_empty() && state.compare.detail.is_none());
}

#[test]
fn rows_and_detail_after_the_session_was_closed_are_ignored() {
    let (state, _) = act(
        with_rows(),
        CompareAction::Select(KeyTuple(vec![Bson::Int32(1)])),
    );
    let (state, _) = act(state, CompareAction::CloseSession);
    assert!(state.compare.results.is_none() && state.compare.detail.is_none());
    let (state, cmds) = update(state, rows_event(Category::Different, 0, 3));
    assert!(cmds.is_empty() && state.compare.results.is_none());
    let answer = CompareEvent::Detail {
        session: 1,
        pair: 1,
        key: KeyTuple(vec![Bson::Int32(1)]),
        source: None,
        target: None,
        changes: vec![],
    };
    let (state, cmds) = update(state, ev(answer));
    assert!(cmds.is_empty() && state.compare.detail.is_none());
}

#[test]
fn a_failed_page_load_is_retried_with_the_same_request() {
    let failed = ev(CompareEvent::Failed {
        session: 1,
        context: "compare_rows",
        error: CoreError::Database("timeout".into()),
    });
    let (state, _) = update(with_differences(), failed);
    assert!(state.compare.results.as_ref().unwrap().load_failed);
    let (state, cmds) = act(state, CompareAction::RetryRows);
    assert!(matches!(
        &cmds[..],
        [Command::Compare(CompareCommand::Rows {
            session: 1,
            pair: 1,
            category: Category::Different,
            offset: 0,
            limit: 100
        })]
    ));
    assert!(!state.compare.results.as_ref().unwrap().load_failed);
    // Nothing failed any more: a second retry sends nothing.
    let (state, cmds) = act(state, CompareAction::RetryRows);
    assert!(cmds.is_empty());
    let (state, _) = update(state, rows_event(Category::Different, 0, 3));
    assert_eq!(state.compare.results.unwrap().rows.len(), 3);
}

#[test]
fn the_identical_count_is_kept_with_the_results() {
    let summary = Summary {
        equal: 17,
        different: 1,
        ..Summary::default()
    };
    let (state, _) = update(running_state(), pair_done(1, summary));
    assert_eq!(state.compare.results.unwrap().counts.identical, 17);
}
