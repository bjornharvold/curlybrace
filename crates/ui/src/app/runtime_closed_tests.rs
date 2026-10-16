//! A runtime whose channels close must not leave a connection test running forever, whichever
//! side notices first (a failed command send, or a closed event channel in `pump`).
use super::*;
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

#[test]
fn a_closed_event_channel_ends_a_running_test_without_a_banner() {
    let (mut app, events, _commands) = app();
    app.state_mut().conn.test = TestState::Running { request_id: 3 };
    drop(events);
    app.pump();
    assert_eq!(app.state().conn.test, TestState::Idle);
    assert_eq!(app.state().banner, None);
}

#[test]
fn a_closed_event_channel_is_harmless_on_every_later_frame() {
    let (mut app, events, _commands) = app();
    drop(events);
    app.pump();
    app.state_mut().conn.test = TestState::Running { request_id: 4 };
    app.pump();
    app.pump();
    assert_eq!(app.state().conn.test, TestState::Idle);
    assert_eq!(app.state().banner, None);
}

#[test]
fn a_closed_event_channel_leaves_other_test_states_alone() {
    for test in [
        TestState::Idle,
        TestState::Done {
            request_id: 2,
            outcome: Err(CoreError::NotFound),
        },
    ] {
        let (mut app, events, _commands) = app();
        app.state_mut().conn.test = test.clone();
        drop(events);
        app.pump();
        assert_eq!(app.state().conn.test, test);
        assert_eq!(app.state().banner, None);
    }
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
    app.send(vec![curlybrace_core::runtime::Command::Disconnect {
        session: uuid::Uuid::new_v4(),
    }]);
    assert_eq!(app.state().conn.test, TestState::Idle);
    assert!(matches!(
        app.state().banner,
        Some(Banner::Failure {
            context: "runtime",
            ..
        })
    ));
}
