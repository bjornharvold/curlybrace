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
