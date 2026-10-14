//! Test Connection in the reducer: a request counter, a stale-result guard and the typed secrets.
//! `tls` and `authenticated` in a report mean "configured", not "verified".
use super::state::AppState;
use crate::viewmodel::ConnectionForm;
use crate::views::connections::{ConnectionsState, TestState};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, TestReport, TestSecrets};
use curlybrace_core::secrets::SecretString;

/// Starts a test for the form. Empty password/ssh fields mean "use the stored secret" (`None`).
/// A new request supersedes a running one: only the latest `request_id` is ever applied.
pub(super) fn start(state: AppState, form: &ConnectionForm) -> (AppState, Vec<Command>) {
    let cfg = match form.to_config() {
        Ok(cfg) => cfg,
        Err(error) => {
            let conn = ConnectionsState {
                error: Some(error),
                ..state.conn
            };
            return (AppState { conn, ..state }, Vec::new());
        }
    };
    let request_id = state.conn.test_seq + 1;
    let typed = |value: &str| (!value.is_empty()).then(|| SecretString::new(value));
    let secrets = TestSecrets {
        password: typed(&form.password),
        ssh_secret: typed(&form.ssh_secret),
    };
    let conn = ConnectionsState {
        test_seq: request_id,
        test: TestState::Running { request_id },
        test_form: Some(form.clone()),
        error: None,
        ..state.conn
    };
    (
        AppState { conn, ..state },
        vec![Command::TestConnection {
            request_id,
            cfg,
            secrets,
        }],
    )
}

/// Applies a result only while the test it answers is the running one.
pub(super) fn on_event(
    state: AppState,
    request_id: u64,
    outcome: Result<TestReport, CoreError>,
) -> AppState {
    if !matches!(state.conn.test, TestState::Running { request_id: r } if r == request_id) {
        return state;
    }
    let conn = ConnectionsState {
        test: TestState::Done {
            request_id,
            outcome,
        },
        ..state.conn
    };
    AppState { conn, ..state }
}

#[cfg(test)]
mod tests {
    use super::super::reducer::{Input, update};
    use super::super::state::ConnStatus;
    use super::*;
    use crate::views::connections::{ConnectionsState, TestState};
    use curlybrace_core::runtime::Event;
    use curlybrace_core::secrets::SecretString;

    fn valid_form() -> ConnectionForm {
        ConnectionForm {
            name: "n".into(),
            uri: "mongodb://localhost:27017".into(),
            ..ConnectionForm::default()
        }
    }

    fn report() -> TestReport {
        TestReport {
            server_version: None,
            tls: false,
            authenticated: false,
        }
    }

    fn run(state: AppState, form: ConnectionForm) -> (AppState, Vec<Command>) {
        update(state, Input::TestConnection(Box::new(form)))
    }

    #[test]
    fn sends_a_command_with_typed_secrets_and_marks_running() {
        let form = ConnectionForm {
            password: "typed".into(),
            ssh_secret: "key".into(),
            ..valid_form()
        };
        let (state, cmds) = run(AppState::default(), form);
        assert!(matches!(
            state.conn.test,
            TestState::Running { request_id: 1 }
        ));
        match &cmds[..] {
            [
                Command::TestConnection {
                    request_id: 1,
                    secrets,
                    ..
                },
            ] => {
                assert_eq!(secrets.password.as_ref().map(|p| p.expose()), Some("typed"));
                assert_eq!(secrets.ssh_secret.as_ref().map(|p| p.expose()), Some("key"));
            }
            other => panic!("unexpected commands: {other:?}"),
        }
    }

    #[test]
    fn empty_secret_fields_mean_use_the_stored_ones() {
        let (_s, cmds) = run(AppState::default(), valid_form());
        assert!(matches!(
            &cmds[..],
            [Command::TestConnection { secrets, .. }]
                if secrets.password.is_none() && secrets.ssh_secret.is_none()
        ));
    }

    #[test]
    fn an_invalid_form_sets_an_error_and_sends_nothing() {
        let (state, cmds) = run(AppState::default(), ConnectionForm::default());
        assert!(cmds.is_empty());
        assert!(state.conn.error.is_some());
        assert_eq!(state.conn.test, TestState::Idle);
        assert_eq!(state.conn.test_seq, 0);
    }

    #[test]
    fn only_the_latest_request_result_is_applied() {
        let (s, _) = run(AppState::default(), valid_form());
        let (s, _) = run(s, valid_form()); // request 2 supersedes request 1
        let ev = |id| {
            Input::Event(Event::TestConnection {
                request_id: id,
                outcome: Ok(report()),
            })
        };
        let (s, _) = update(s, ev(1));
        assert!(
            matches!(s.conn.test, TestState::Running { request_id: 2 }),
            "stale result ignored"
        );
        let (s, _) = update(s, ev(2));
        assert!(matches!(s.conn.test, TestState::Done { request_id: 2, .. }));
    }

    #[test]
    fn a_result_after_the_test_was_reset_is_ignored() {
        let (mut s, _) = run(AppState::default(), valid_form());
        s.conn.reset_tools();
        let s = on_event(s, 1, Ok(report()));
        assert_eq!(s.conn.test, TestState::Idle);
    }

    #[test]
    fn a_result_is_shown_even_when_not_connected_to_any_server() {
        let (s, _) = run(AppState::default(), valid_form());
        assert_eq!(s.status, ConnStatus::Disconnected);
        let (s, _) = update(
            s,
            Input::Event(Event::TestConnection {
                request_id: 1,
                outcome: Err(CoreError::AuthFailed),
            }),
        );
        assert!(matches!(
            s.conn.test,
            TestState::Done {
                outcome: Err(CoreError::AuthFailed),
                ..
            }
        ));
    }

    #[test]
    fn editing_the_form_after_a_test_clears_the_result() {
        let mut conn = ConnectionsState {
            form: valid_form(),
            ..ConnectionsState::default()
        };
        conn.test_form = Some(conn.form.clone());
        conn.test = TestState::Done {
            request_id: 1,
            outcome: Ok(report()),
        };
        conn.invalidate_stale_test();
        assert!(
            matches!(conn.test, TestState::Done { .. }),
            "unchanged form keeps it"
        );
        conn.form.uri = "mongodb://other".into();
        conn.invalidate_stale_test();
        assert_eq!(conn.test, TestState::Idle);
    }

    #[test]
    fn editing_the_form_clears_a_built_uri_but_not_a_running_test() {
        let mut conn = ConnectionsState {
            form: valid_form(),
            ..ConnectionsState::default()
        };
        conn.built_for = Some(conn.form.clone());
        conn.built_uri = Some(SecretString::new("x"));
        conn.built_with_password = true;
        conn.test = TestState::Running { request_id: 1 };
        conn.invalidate_stale_test();
        assert!(conn.built_uri.is_some());
        conn.form.name = "changed".into();
        conn.invalidate_stale_test();
        assert!(conn.built_uri.is_none() && conn.built_for.is_none());
        assert!(!conn.built_with_password);
        assert_eq!(conn.test, TestState::Running { request_id: 1 });
    }

    #[test]
    fn apply_uri_input_fills_the_form_and_clears_the_buffer() {
        let mut conn = ConnectionsState {
            uri_input: "mongodb://bob:pw@h/db".into(),
            error: Some(CoreError::AuthFailed),
            ..ConnectionsState::default()
        };
        conn.apply_uri_input();
        assert!(conn.uri_input.is_empty());
        assert_eq!(conn.form.uri, "mongodb://h/db");
        assert_eq!(conn.form.password, "pw");
        assert!(conn.error.is_none());
    }

    #[test]
    fn apply_uri_input_failure_clears_the_buffer_and_does_not_echo_it() {
        let mut conn = ConnectionsState {
            uri_input: "notmongo://bob:secretpw@h".into(),
            ..ConnectionsState::default()
        };
        let before = conn.form.clone();
        conn.apply_uri_input();
        assert!(conn.uri_input.is_empty());
        assert_eq!(conn.form, before);
        let error = conn.error.clone().expect("error");
        assert!(!format!("{error:?} {error}").contains("secretpw"));
        assert!(!format!("{conn:?}").contains("secretpw"));
    }
}
