//! The Shell tab's guard when its check answers while the person looks at another session. A
//! production run is confirmed only from the session it belongs to, so a background answer that
//! would need the confirmation parks nothing: the shell says the run was not started.
use super::shell_guard_tests::{
    QUERY, answered, clean, event, fake_mongosh, production, runs, shell, shell_of, the_check,
};
use crate::app::reducer::{Input, update};
use crate::app::shell_actions::ShellAction;
use crate::app::state::AppState;
use crate::viewmodel::shell::{ShellNotice, ShellStatus};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Event;
use curlybrace_core::test_support::mock_objects::mock_connection;
use curlybrace_core::tools::{ToolChoice, ToolKind};
use std::path::Path;
use uuid::Uuid;

fn connected(s: AppState, id: Uuid) -> AppState {
    let (s, _) = update(s, Input::Connect(id));
    let epoch = s.epoch_counter;
    let ev = Event::Connected {
        session: id,
        epoch,
        topology: None,
    };
    event(s, ev).0
}

/// `cfg` and another plain session open and connected, the guard on; `cfg`'s shell ran `QUERY`
/// and waits for its check (request 1) while the other session is the active one.
fn checking_in_background(cfg: ConnectionConfig, exe: &Path) -> (AppState, Uuid, Uuid) {
    let other = ConnectionConfig {
        name: "other".into(),
        ..mock_connection()
    };
    let (a, b) = (cfg.id, other.id);
    let s = connected(connected(AppState::new(vec![cfg, other]), a), b);
    let tools = s.tools.choose_override(
        a,
        ToolKind::Mongosh,
        ToolChoice::Executable(exe.to_path_buf()),
    );
    let mut s = AppState { tools, ..s };
    s.explain.guard = true;
    let (s, _) = update(s, Input::Activate(a));
    let (s, _) = shell(
        s,
        ShellAction::Edit {
            script: QUERY.into(),
            selection: None,
        },
    );
    let (s, commands) = shell(s, ShellAction::Run);
    let (session, request, _, _) = the_check(&commands);
    assert_eq!((session, request), (a, 1));
    let (s, _) = update(s, Input::Activate(b));
    assert_eq!(s.active, Some(b));
    (s, a, b)
}

fn assert_not_started_in_background(s: &AppState, a: Uuid, b: Uuid) {
    assert!(s.write_warning.is_none(), "{:?}", s.write_warning);
    assert_eq!(s.active, Some(b));
    let sh = shell_of(s, a);
    assert_eq!(sh.status, ShellStatus::Idle);
    assert_eq!(sh.notice, Some(ShellNotice::GuardNotStartedBackground));
    assert!(sh.can_run());
}

#[test]
fn a_background_production_clean_answer_parks_nothing_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a, b) = checking_in_background(production(), &fake_mongosh(dir.path()));

    let (s, commands) = answered(s, a, 1, Ok(clean()));

    assert!(commands.is_empty(), "{commands:?}");
    assert_not_started_in_background(&s, a, b);
}

#[test]
fn a_background_production_failed_check_parks_nothing_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a, b) = checking_in_background(production(), &fake_mongosh(dir.path()));

    let (s, commands) = answered(s, a, 1, Err(CoreError::Database("timed out".into())));

    assert!(commands.is_empty(), "{commands:?}");
    assert_not_started_in_background(&s, a, b);
    assert!(s.banner.is_none());
}

#[test]
fn a_background_production_check_that_could_not_start_parks_nothing_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a, b) = checking_in_background(production(), &fake_mongosh(dir.path()));
    let ev = Event::RequestFailed {
        session: a,
        context: "guard_check",
        request: 1,
        error: CoreError::Unreachable,
    };

    let (s, commands) = event(s, ev);

    assert!(commands.is_empty(), "{commands:?}");
    assert_not_started_in_background(&s, a, b);
    assert!(s.banner.is_none());
}

#[test]
fn a_background_non_production_clean_answer_still_runs() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig {
        name: "plain".into(),
        ..mock_connection()
    };
    let (s, a, b) = checking_in_background(cfg, &fake_mongosh(dir.path()));

    let (s, commands) = answered(s, a, 1, Ok(clean()));

    assert_eq!(runs(&commands), vec![(1, QUERY.to_string())]);
    assert!(s.write_warning.is_none());
    assert_eq!(s.active, Some(b));
    assert_eq!(shell_of(&s, a).status, ShellStatus::Running { run_id: 1 });
}

#[test]
fn an_active_production_clean_answer_still_parks_the_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let (s, a, _) = checking_in_background(production(), &fake_mongosh(dir.path()));
    let (s, _) = update(s, Input::Activate(a));

    let (s, commands) = answered(s, a, 1, Ok(clean()));

    assert!(commands.is_empty(), "{commands:?}");
    let parked = s.write_warning.as_ref().expect("the confirmation");
    assert_eq!(parked.command.session(), Some(a));
    assert_eq!(shell_of(&s, a).notice, None);
}
