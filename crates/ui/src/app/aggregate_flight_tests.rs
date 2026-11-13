//! At most one preview in flight per session: a newer request waits (the newest wins) and goes out
//! when the one in flight is answered, whatever the answer; a collection change drops the wait.
use super::AggregateAction as A;
use crate::app::reducer::{Input, update};
use crate::app::state::{AppState, ConnStatus};
use bson::Document;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, EMBEDDED_SESSION, Event};
use curlybrace_core::store::Namespace;

fn people() -> Namespace {
    Namespace::new("app", "people")
}

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
        .selecting(people());
    s
}

fn act(s: AppState, action: A) -> (AppState, Vec<Command>) {
    update(s, Input::Aggregate(action))
}

fn requests(commands: &[Command]) -> Vec<(u64, Vec<Document>)> {
    commands
        .iter()
        .filter_map(|c| match c {
            Command::AggregatePreview {
                request, pipeline, ..
            } => Some((*request, pipeline.clone())),
            _ => None,
        })
        .collect()
}

fn answer(s: AppState, request: u64, outcome: Result<Vec<Document>, CoreError>) -> Out {
    let event = Event::AggregatePreviewed {
        session: EMBEDDED_SESSION,
        request,
        outcome,
    };
    update(s, Input::Event(event))
}

type Out = (AppState, Vec<Command>);

/// Three stages, every preview answered, so nothing is in flight.
fn settled() -> AppState {
    let mut s = ["$match", "$sort", "$limit"]
        .into_iter()
        .fold(connected(), |s, name| act(s, A::AddStage(name.into())).0);
    while let Some(request) = s.aggregate.in_flight {
        s = answer(s, request, Ok(vec![])).0;
    }
    s
}

#[test]
fn three_rapid_selections_keep_one_preview_in_flight_and_send_only_the_last_after_the_answer() {
    let (s, first) = act(settled(), A::Select(0));
    let first = requests(&first);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].1.len(), 2, "stage 0 plus the limit");

    let (s, middle) = act(s, A::Select(1));
    assert!(middle.is_empty(), "the second waits");
    let (s, last) = act(s, A::Select(2));
    assert!(last.is_empty(), "the third replaces the second");
    assert_eq!(s.aggregate.in_flight, Some(first[0].0));
    assert!(matches!(
        s.aggregate.preview,
        crate::viewmodel::aggregate::PreviewState::Loading { .. }
    ));

    let (s, sent) = answer(s, first[0].0, Ok(vec![]));
    let sent = requests(&sent);
    assert_eq!(sent.len(), 1, "only the newest goes out");
    assert_eq!(sent[0].0, s.aggregate.request);
    assert_eq!(sent[0].1.len(), 4, "all three stages plus the limit");
    assert_eq!(s.aggregate.in_flight, Some(sent[0].0));
    assert_eq!(s.aggregate.queued, None);

    let (s, none) = answer(s, sent[0].0, Ok(vec![]));
    assert!(none.is_empty());
    assert_eq!(s.aggregate.in_flight, None);
}

#[test]
fn a_failed_or_superseded_answer_also_frees_the_slot_for_the_waiting_preview() {
    let failures = [
        CoreError::Database("boom".into()),
        CoreError::Superseded,
        CoreError::Database("timeout".into()),
    ];
    for error in failures {
        let (s, first) = act(settled(), A::Select(0));
        let first = requests(&first)[0].0;
        let (s, _) = act(s, A::Select(1));
        let event = Event::RequestFailed {
            session: EMBEDDED_SESSION,
            context: "aggregate_preview",
            request: first,
            error: error.clone(),
        };
        let (s, sent) = update(s, Input::Event(event));
        assert_eq!(requests(&sent).len(), 1, "{error:?}");
        assert_eq!(
            s.aggregate.in_flight,
            Some(s.aggregate.request),
            "{error:?}"
        );
    }
}

#[test]
fn a_collection_change_drops_the_waiting_preview_but_the_old_answer_still_frees_the_slot() {
    let (s, first) = act(settled(), A::Select(0));
    let first = requests(&first)[0].0;
    let (s, _) = act(s, A::Select(1));
    assert!(s.aggregate.queued.is_some());
    let (s, _) = update(
        s,
        Input::Sidebar(crate::views::sidebar::SidebarAction::Open(Namespace::new(
            "app", "orders",
        ))),
    );
    assert_eq!(s.aggregate.queued, None);
    let (s, sent) = answer(s, first, Ok(vec![]));
    assert!(requests(&sent).is_empty(), "nothing for the old collection");
    assert_eq!(s.aggregate.in_flight, None);
    // The new collection's preview now runs.
    let (s, c) = act(s, A::Refresh);
    assert_eq!(requests(&c).len(), 1);
    assert!(s.aggregate.in_flight.is_some());
}

#[test]
fn a_frame_of_changes_never_leaves_the_slot_taken_by_an_unsent_preview() {
    let batch = A::Batch(vec![A::Select(0), A::Select(1), A::Select(2)]);
    let (s, c) = act(settled(), batch);
    let sent = requests(&c);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, s.aggregate.request);
    assert_eq!(s.aggregate.in_flight, Some(sent[0].0));
    assert_eq!(s.aggregate.queued, None);
}

#[test]
fn a_frame_that_ends_refused_sends_nothing_and_frees_the_slot() {
    let batch = A::Batch(vec![A::Select(0), A::Edit(0, "{ bad".into())]);
    let (s, c) = act(settled(), batch);
    assert!(requests(&c).is_empty());
    assert_eq!(s.aggregate.in_flight, None);
}

fn failed_preview(s: AppState, request: u64, error: CoreError) -> Out {
    let event = Event::RequestFailed {
        session: EMBEDDED_SESSION,
        request,
        context: "aggregate_preview",
        error,
    };
    update(s, Input::Event(event))
}

/// Request 1 in flight with request 2 queued behind it (the pane shows Loading for 2).
fn one_in_flight_one_queued() -> (AppState, u64) {
    let (s, first) = act(settled(), A::Select(0));
    let first = requests(&first)[0].0;
    let (s, none) = act(s, A::Select(1));
    assert!(none.is_empty());
    (s, first)
}

#[test]
fn a_lost_server_drops_the_queued_preview_and_leaves_loading_while_an_editor_is_open() {
    use crate::viewmodel::aggregate::PreviewState;
    for error in [CoreError::Unreachable, CoreError::TlsHandshake] {
        let (mut s, first) = one_in_flight_one_queued();
        s.editor = Some(crate::viewmodel::editor::EditorState::for_new());
        let (s, sent) = failed_preview(s, first, error);
        assert!(
            requests(&sent).is_empty(),
            "nothing is sent on a dead connection"
        );
        assert_eq!(s.aggregate.in_flight, None);
        assert_eq!(s.aggregate.queued, None);
        assert!(
            !matches!(s.aggregate.preview, PreviewState::Loading { .. }),
            "the pane must not stay on Loading with nothing in flight"
        );
        assert!(
            !s.aggregate.wants_auto_refresh(None),
            "a failed pane is not refreshed on its own against a dead connection"
        );
    }
}

#[test]
fn without_an_editor_a_lost_server_still_fails_the_session_and_resets_the_pane() {
    let (s, first) = one_in_flight_one_queued();
    let (s, sent) = failed_preview(s, first, CoreError::Unreachable);
    assert!(requests(&sent).is_empty());
    assert_ne!(s.status, ConnStatus::Connected);
    assert_eq!(s.aggregate.in_flight, None);
}
