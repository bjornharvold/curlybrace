//! The Aggregate draft across collection and connection changes: a preview of the collection
//! that was open before is stale, and a lost connection or a reconnect keeps the draft.
use crate::app::AggregateAction as A;
use crate::app::reducer::{Input, Out, update};
use crate::app::state::{AppState, ConnStatus};
use crate::viewmodel::SidebarAction;
use crate::viewmodel::aggregate::PreviewState;
use bson::doc;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::Namespace;
use uuid::Uuid;

fn ev(state: AppState, event: Event) -> Out {
    update(state, Input::Event(event))
}

fn people() -> Namespace {
    Namespace::new("app", "people")
}

/// One saved connection, connected, with `app.people` open.
fn browsing() -> (AppState, Uuid) {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let (s, _) = update(AppState::new(vec![cfg]), Input::Connect(id));
    let epoch = s.epoch_counter;
    let connected = Event::Connected {
        session: id,
        epoch,
        topology: None,
    };
    let s = ev(s, connected).0;
    (
        update(s, Input::Sidebar(SidebarAction::Open(people()))).0,
        id,
    )
}

/// The request number of the one preview command in `commands`.
fn request_of(commands: &[Command]) -> u64 {
    match commands {
        [Command::AggregatePreview { request, .. }] => *request,
        other => panic!("expected one AggregatePreview, got {other:?}"),
    }
}

/// `browsing()` with a `$match` stage whose preview is in flight.
fn previewing() -> (AppState, Uuid, u64) {
    let (s, id) = browsing();
    let (s, c) = update(s, Input::Aggregate(A::AddStage("$match".into())));
    let request = request_of(&c);
    (s, id, request)
}

fn answered(s: AppState, id: Uuid, request: u64) -> AppState {
    let event = Event::AggregatePreviewed {
        session: id,
        request,
        outcome: Ok(vec![doc! { "a": 1 }]),
    };
    ev(s, event).0
}

#[test]
fn opening_another_collection_makes_a_preview_in_flight_stale() {
    let (s, id, request) = previewing();
    let other = Namespace::new("app", "orders");
    let (s, _) = update(s, Input::Sidebar(SidebarAction::Open(other)));
    assert_eq!(s.aggregate.preview, PreviewState::Idle);
    assert_eq!(s.aggregate.stages.len(), 1, "the draft stays");
    let s = answered(s, id, request);
    assert_eq!(
        s.aggregate.preview,
        PreviewState::Idle,
        "the late answer is dropped"
    );
}

#[test]
fn opening_another_collection_drops_a_shown_preview_so_it_runs_again() {
    let (s, id, request) = previewing();
    let s = answered(s, id, request);
    assert!(matches!(s.aggregate.preview, PreviewState::Shown(_)));
    let (s, _) = update(s, Input::Sidebar(SidebarAction::Open(people())));
    assert_eq!(s.aggregate.preview, PreviewState::Idle);
    assert!(s.aggregate.wants_auto_refresh(s.aggregate_view.auto_sent));
    let (s, c) = update(s, Input::Aggregate(A::Refresh));
    request_of(&c);
    assert!(matches!(s.aggregate.preview, PreviewState::Loading { .. }));
}

#[test]
fn a_lost_connection_keeps_the_draft_and_makes_its_preview_stale() {
    let (s, id, request) = previewing();
    let lost = Event::Failed {
        session: id,
        context: "find",
        error: CoreError::Unreachable,
    };
    let (s, _) = ev(s, lost);
    assert_eq!(s.status, ConnStatus::Failed);
    assert_eq!(s.aggregate.stages.len(), 1, "the draft survives the reset");
    assert_eq!(s.aggregate.preview, PreviewState::Idle);
    assert!(s.aggregate.request != request);
}

#[test]
fn a_reconnect_keeps_the_draft_of_the_failed_session() {
    let (s, id, _) = previewing();
    let (s, _) = update(s, Input::Aggregate(A::AddStage("$sort".into())));
    let lost = Event::Failed {
        session: id,
        context: "find",
        error: CoreError::Unreachable,
    };
    let (s, _) = ev(s, lost);
    let (s, _) = update(s, Input::Connect(id));
    assert_eq!(s.status, ConnStatus::Connecting);
    assert_eq!(s.aggregate.stages.len(), 2);
    assert_eq!(s.aggregate.selected, 1);
    assert_eq!(s.aggregate.preview, PreviewState::Idle);
}

fn preview_failed(id: Uuid, request: u64, error: CoreError) -> Event {
    Event::RequestFailed {
        session: id,
        context: "aggregate_preview",
        request,
        error,
    }
}

#[test]
fn a_preview_that_finds_the_server_gone_fails_the_session_like_any_read() {
    for error in [CoreError::Unreachable, CoreError::TlsHandshake] {
        let (s, id, request) = previewing();
        let (s, _) = ev(s, preview_failed(id, request, error));
        assert_eq!(s.status, ConnStatus::Failed);
        assert_eq!(s.aggregate.stages.len(), 1, "the draft survives the reset");
    }
}

#[test]
fn an_ordinary_preview_failure_stays_inline() {
    let (s, id, request) = previewing();
    let (s, _) = ev(
        s,
        preview_failed(id, request, CoreError::Database("x".into())),
    );
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(s.banner.is_none());
    assert!(matches!(s.aggregate.preview, PreviewState::Failed { .. }));
}

#[test]
fn a_loss_of_the_preview_in_flight_fails_the_session_though_a_newer_one_waits() {
    let (s, id, request) = previewing();
    let (s, c) = update(s, Input::Aggregate(A::AddStage("$sort".into())));
    assert!(
        c.is_empty(),
        "the newer one waits: nobody else would answer"
    );
    let (s, c) = ev(s, preview_failed(id, request, CoreError::Unreachable));
    assert_eq!(s.status, ConnStatus::Failed);
    assert!(c.is_empty(), "nothing is sent on the lost connection");
    assert_eq!(s.aggregate.in_flight, None);
    assert_eq!(s.aggregate.queued, None);
}

#[test]
fn an_answer_that_is_not_the_one_in_flight_changes_nothing() {
    let (s, id, request) = previewing();
    let (s, c) = ev(s, preview_failed(id, request + 7, CoreError::Unreachable));
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(c.is_empty());
    assert_eq!(s.aggregate.in_flight, Some(request));
}
