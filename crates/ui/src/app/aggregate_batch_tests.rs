//! The Aggregate tab's mode round trip and the actions of one frame applied as a batch: a frame
//! sends at most one preview, the one of the request that is current after it.
use super::AggregateAction as A;
use crate::app::reducer::{Input, update};
use crate::app::state::{AppState, ConnStatus};
use crate::viewmodel::aggregate::Mode;
use curlybrace_core::runtime::{Command, EMBEDDED_SESSION};
use curlybrace_core::store::Namespace;

/// Connected embedded session with `app.people` selected.
fn connected() -> AppState {
    let mut s = AppState::new(vec![]);
    s.open = vec![EMBEDDED_SESSION];
    s.active = Some(EMBEDDED_SESSION);
    s.status = ConnStatus::Connected;
    s.sidebar = s
        .sidebar
        .with_databases(vec!["app".into()])
        .with_collections("app", vec!["people".into()])
        .toggled("app")
        .selecting(Namespace::new("app", "people"));
    s
}

fn act(s: AppState, action: A) -> (AppState, Vec<Command>) {
    update(s, Input::Aggregate(action))
}

/// `connected()` with `$match`, `$sort` and `$limit` stages.
fn with_three_stages() -> AppState {
    ["$match", "$sort", "$limit"]
        .into_iter()
        .fold(connected(), |s, name| act(s, A::AddStage(name.into())).0)
}

/// The request numbers of the preview commands in `commands`.
fn preview_requests(commands: &[Command]) -> Vec<u64> {
    commands
        .iter()
        .filter_map(|c| match c {
            Command::AggregatePreview { request, .. } => Some(*request),
            _ => None,
        })
        .collect()
}

#[test]
fn leaving_unchanged_text_keeps_the_stages_and_sends_nothing() {
    let (s, _) = act(with_three_stages(), A::Toggle(0));
    let before = s.aggregate.clone();
    let (s, _) = act(s, A::SetMode(Mode::Text));
    let (s, c) = act(s, A::SetMode(Mode::Stages));
    assert!(c.is_empty(), "{c:?}");
    assert_eq!(s.aggregate.stages, before.stages);
    assert_eq!(s.aggregate.selected, before.selected);
    assert_eq!(s.aggregate.mode, Mode::Stages);
}

#[test]
fn a_frame_of_several_changes_sends_one_preview_of_the_current_request() {
    let batch = A::Batch(vec![
        A::Edit(0, r#"{ "$match": { "a": 1 } }"#.into()),
        A::MoveUp(2),
        A::Select(0),
    ]);
    let (s, c) = act(with_three_stages(), batch);
    assert_eq!(preview_requests(&c), [s.aggregate.request]);
    assert_eq!(c.len(), 1);
    assert!(
        s.aggregate.stages[0].text.contains("\"a\""),
        "every action applied"
    );
}

#[test]
fn a_frame_ending_in_a_refused_pipeline_sends_nothing() {
    let batch = A::Batch(vec![
        A::Edit(0, r#"{ "$match": { "a": 1 } }"#.into()),
        A::AddStage("$out".into()),
        A::AddStage("$match".into()),
    ]);
    let (_, c) = act(with_three_stages(), batch);
    assert!(c.is_empty(), "the earlier previews are stale: {c:?}");
}

#[test]
fn one_action_is_not_wrapped_and_none_is_nothing() {
    assert_eq!(A::batched(Vec::new()), None);
    assert_eq!(A::batched(vec![A::Refresh]), Some(A::Refresh));
    assert_eq!(
        A::batched(vec![A::Refresh, A::Select(1)]),
        Some(A::Batch(vec![A::Refresh, A::Select(1)]))
    );
}
