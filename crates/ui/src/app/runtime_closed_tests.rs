//! A runtime whose channels close ends the sessions, compare state and any connection test, and
//! raises its failure banner once, whichever side notices first (a failed command send, or a
//! closed event channel in `pump`).
use super::*;
use crate::viewmodel::compare::SessionPhase;
use curlybrace_core::secrets::MemorySecretStore;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

fn app() -> (
    CurlybraceApp,
    UnboundedSender<Event>,
    UnboundedReceiver<curlybrace_core::runtime::Command>,
) {
    let (tx, commands) = tokio::sync::mpsc::unbounded_channel();
    let (events, rx) = tokio::sync::mpsc::unbounded_channel();
    let app = CurlybraceApp::new(
        RuntimeHandle { tx, rx },
        I18n::english(),
        Vec::new(),
        Arc::new(MemorySecretStore::default()),
    );
    (app, events, commands)
}

fn runtime_banner(app: &CurlybraceApp) -> bool {
    matches!(
        app.state().banner,
        Some(Banner::Failure {
            context: "runtime",
            ..
        })
    )
}

fn disconnect() -> Vec<curlybrace_core::runtime::Command> {
    vec![curlybrace_core::runtime::Command::Disconnect {
        session: uuid::Uuid::new_v4(),
    }]
}

#[test]
fn a_closed_event_channel_runs_the_full_runtime_stop() {
    let (mut app, events, _commands) = app();
    let id = uuid::Uuid::new_v4();
    {
        let s = app.state_mut();
        s.open = vec![id];
        s.active = Some(id);
        s.status = ConnStatus::Connecting;
        s.compare.session = Some(SessionPhase::Ready(7));
        s.conn.test = TestState::Running { request_id: 3 };
    }
    drop(events);
    app.pump();
    let s = app.state();
    assert!(s.open.is_empty());
    assert_eq!(s.active, None);
    assert_eq!(s.status, ConnStatus::Disconnected);
    assert_eq!(s.compare.session, None);
    assert_eq!(s.conn.test, TestState::Idle);
    assert!(runtime_banner(&app));
}

#[test]
fn the_runtime_banner_is_raised_once_across_frames() {
    let (mut app, events, _commands) = app();
    drop(events);
    app.pump();
    assert!(runtime_banner(&app));
    // The user dismisses it; later frames must not bring it back.
    app.state_mut().banner = None;
    app.pump();
    app.pump();
    assert_eq!(app.state().banner, None);
}

#[test]
fn a_failed_send_after_the_channel_closed_raises_no_second_banner() {
    let (mut app, events, commands) = app();
    drop(events);
    drop(commands);
    app.pump();
    app.state_mut().banner = None;
    app.send(disconnect());
    assert_eq!(app.state().banner, None);
}

#[test]
fn a_closed_channel_after_a_failed_send_raises_no_second_banner() {
    let (mut app, events, commands) = app();
    drop(commands);
    app.send(disconnect());
    assert!(runtime_banner(&app));
    app.state_mut().banner = None;
    drop(events);
    app.pump();
    assert_eq!(app.state().banner, None);
}

#[test]
fn a_connect_attempted_after_the_runtime_is_gone_does_not_stay_connecting() {
    let (mut app, events, commands) = app();
    drop(events);
    drop(commands);
    app.pump();
    app.state_mut().status = ConnStatus::Connecting;
    app.state_mut().conn.test = TestState::Running { request_id: 9 };
    app.send(disconnect());
    assert_eq!(app.state().status, ConnStatus::Disconnected);
    assert_eq!(app.state().conn.test, TestState::Idle);
}

#[test]
fn an_open_channel_is_left_untouched() {
    let (mut app, _events, _commands) = app();
    let id = uuid::Uuid::new_v4();
    app.state_mut().open = vec![id];
    app.state_mut().status = ConnStatus::Connecting;
    app.state_mut().conn.test = TestState::Running { request_id: 3 };
    app.pump();
    assert_eq!(app.state().open, vec![id]);
    assert_eq!(app.state().status, ConnStatus::Connecting);
    assert_eq!(app.state().conn.test, TestState::Running { request_id: 3 });
    assert_eq!(app.state().banner, None);
}

#[test]
fn events_queued_before_the_channel_closed_are_still_applied() {
    let (mut app, events, _commands) = app();
    app.state_mut().conn.test = TestState::Running { request_id: 3 };
    events
        .send(Event::TestConnection {
            request_id: 3,
            outcome: Err(CoreError::NotFound),
        })
        .unwrap();
    drop(events);
    app.pump();
    // The answer that was already queued wins over the closed channel behind it.
    assert!(matches!(
        app.state().conn.test,
        TestState::Done { request_id: 3, .. }
    ));
}

#[test]
fn a_failed_send_still_ends_the_test_and_raises_the_runtime_banner() {
    let (mut app, _events, commands) = app();
    drop(commands);
    app.state_mut().conn.test = TestState::Running { request_id: 3 };
    app.send(disconnect());
    assert_eq!(app.state().conn.test, TestState::Idle);
    assert!(runtime_banner(&app));
}
