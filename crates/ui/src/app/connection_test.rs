//! Test Connection in the reducer: a request counter, a stale-result guard and the typed secrets.
//! `tls` and `authenticated` in a report mean "configured", not "verified".
use super::state::AppState;
use crate::viewmodel::ConnectionForm;
use crate::views::connections::{ConnectionsState, TestState};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, PasswordChoice, TestReport, TestSecrets};
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
    let password = match typed(&form.password) {
        Some(secret) => PasswordChoice::Typed(secret),
        None if form.clear_password => PasswordChoice::Without,
        None => PasswordChoice::Stored,
    };
    let ssh_secret = match typed(&form.ssh_secret) {
        Some(secret) => PasswordChoice::Typed(secret),
        None if form.clear_ssh_secret => PasswordChoice::Without,
        None => PasswordChoice::Stored,
    };
    let secrets = TestSecrets {
        password,
        ssh_secret,
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
    // The tunnel is the first thing to check when a server behind one is unreachable.
    let through_ssh = state.conn.test_form.as_ref().is_some_and(|f| f.ssh_enabled);
    let outcome = match outcome {
        Err(CoreError::Unreachable) if through_ssh => Err(CoreError::UnreachableSsh),
        Err(CoreError::TlsHandshake) if through_ssh => Err(CoreError::TlsHandshakeSsh),
        other => other,
    };
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
    use crate::views::connections::TestState;
    use curlybrace_core::runtime::Event;

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

    fn finished_with(form: ConnectionForm, error: CoreError) -> TestState {
        let (state, _) = run(AppState::default(), form);
        let (state, _) = update(
            state,
            Input::Event(Event::TestConnection {
                request_id: 1,
                outcome: Err(error),
            }),
        );
        state.conn.test
    }

    #[test]
    fn an_unreachable_test_through_a_tunnel_names_the_tunnel() {
        let tunnelled = ConnectionForm {
            ssh_enabled: true,
            ssh_host: "bastion".into(),
            ssh_user: "ops".into(),
            ..valid_form()
        };
        assert!(matches!(
            finished_with(tunnelled, CoreError::Unreachable),
            TestState::Done {
                outcome: Err(CoreError::UnreachableSsh),
                ..
            }
        ));
        assert!(matches!(
            finished_with(valid_form(), CoreError::Unreachable),
            TestState::Done {
                outcome: Err(CoreError::Unreachable),
                ..
            }
        ));
    }

    #[test]
    fn a_tls_handshake_test_gets_the_tls_text_and_names_the_tunnel_through_ssh() {
        let tunnelled = ConnectionForm {
            ssh_enabled: true,
            ssh_host: "bastion".into(),
            ssh_user: "ops".into(),
            ..valid_form()
        };
        assert!(matches!(
            finished_with(tunnelled, CoreError::TlsHandshake),
            TestState::Done {
                outcome: Err(CoreError::TlsHandshakeSsh),
                ..
            }
        ));
        assert!(matches!(
            finished_with(valid_form(), CoreError::TlsHandshake),
            TestState::Done {
                outcome: Err(CoreError::TlsHandshake),
                ..
            }
        ));
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
                assert_eq!(
                    secrets.password,
                    PasswordChoice::Typed(SecretString::new("typed"))
                );
                assert_eq!(
                    secrets.ssh_secret,
                    PasswordChoice::Typed(SecretString::new("key"))
                );
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
                if secrets.password == PasswordChoice::Stored
                    && secrets.ssh_secret == PasswordChoice::Stored
        ));
    }

    #[test]
    fn the_clear_password_toggle_sends_an_explicit_no_password_intent() {
        let form = ConnectionForm {
            clear_password: true,
            ..valid_form()
        };
        let (_s, cmds) = run(AppState::default(), form);
        assert!(matches!(
            &cmds[..],
            [Command::TestConnection { secrets, .. }] if secrets.password == PasswordChoice::Without
        ));
    }

    #[test]
    fn the_clear_ssh_secret_toggle_sends_an_explicit_no_secret_intent() {
        let form = ConnectionForm {
            clear_ssh_secret: true,
            ..valid_form()
        };
        let (_s, cmds) = run(AppState::default(), form);
        assert!(matches!(
            &cmds[..],
            [Command::TestConnection { secrets, .. }] if secrets.ssh_secret == PasswordChoice::Without
        ));
    }

    #[test]
    fn a_typed_ssh_secret_wins_over_the_clear_toggle() {
        let form = ConnectionForm {
            clear_ssh_secret: true,
            ssh_secret: "new".into(),
            ..valid_form()
        };
        let (_s, cmds) = run(AppState::default(), form);
        assert!(matches!(
            &cmds[..],
            [Command::TestConnection { secrets, .. }]
                if secrets.ssh_secret == PasswordChoice::Typed(SecretString::new("new"))
        ));
    }

    #[test]
    fn a_typed_password_wins_over_the_clear_toggle() {
        let form = ConnectionForm {
            clear_password: true,
            password: "new".into(),
            ..valid_form()
        };
        let (_s, cmds) = run(AppState::default(), form);
        assert!(matches!(
            &cmds[..],
            [Command::TestConnection { secrets, .. }]
                if secrets.password == PasswordChoice::Typed(SecretString::new("new"))
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
}
