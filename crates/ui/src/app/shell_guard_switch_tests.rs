//! The Shell tab's guard when the world changes during a check: another session's write warning
//! is up when the answer comes, or the guard switch is turned off. A run the check let go is
//! never dropped silently, and "off" means the run behaves as without a guard.
use super::shell_guard_tests::{
    QUERY, answered, clean, fake_mongosh, missed, plain, production, ready, runs, scan, shell,
    shell_of, the_check,
};
use crate::app::reducer::{Input, update};
use crate::app::shell_actions::ShellAction;
use crate::app::state::AppState;
use crate::app::write_guard::{ParkedWrite, WriteKind};
use crate::viewmodel::FindAction;
use crate::viewmodel::shell::{ShellNotice, ShellStatus};
use curlybrace_core::runtime::Command;
use std::path::PathBuf;
use uuid::Uuid;

/// A write warning raised for another session while the check ran.
fn other_warning() -> Option<ParkedWrite> {
    Some(ParkedWrite {
        command: Command::ShellRun {
            session: Uuid::new_v4(),
            run_id: 99,
            exe: PathBuf::from("/other/mongosh"),
            script: "other()".into(),
        },
        pending: None,
        kind: WriteKind::ShellRun,
        epoch: 0,
    })
}

fn parked_script(s: &AppState) -> Option<String> {
    match s.write_warning.as_ref().map(|w| &w.command) {
        Some(Command::ShellRun { script, .. }) => Some(script.clone()),
        _ => None,
    }
}

#[test]
fn a_non_production_run_goes_while_another_sessions_warning_is_up() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()), QUERY);
    let (s, _) = shell(s, ShellAction::Run);
    let s = AppState {
        write_warning: other_warning(),
        ..s
    };

    let (s, commands) = answered(s, id, 1, Ok(clean()));

    assert_eq!(runs(&commands), vec![(1, QUERY.to_string())]);
    assert_eq!(shell_of(&s, id).status, ShellStatus::Running { run_id: 1 });
    assert_eq!(parked_script(&s).as_deref(), Some("other()"));
}

#[test]
fn a_production_run_that_finds_the_warning_taken_says_so_and_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(production(), &fake_mongosh(dir.path()), QUERY);
    let (s, _) = shell(s, ShellAction::Run);
    let s = AppState {
        write_warning: other_warning(),
        ..s
    };

    let (s, commands) = answered(
        s,
        id,
        1,
        Err(curlybrace_core::error::CoreError::Unreachable),
    );

    assert!(commands.is_empty(), "{commands:?}");
    let sh = shell_of(&s, id);
    assert_eq!(sh.status, ShellStatus::Idle);
    assert_eq!(sh.notice, Some(ShellNotice::GuardNotStarted));
    assert_eq!(parked_script(&s).as_deref(), Some("other()"));
}

#[test]
fn findings_after_the_guard_was_turned_off_still_run() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()), QUERY);
    let (s, commands) = shell(s, ShellAction::Run);
    let (_, request, _, _) = the_check(&commands);
    let (s, _) = update(s, Input::Find(FindAction::SetGuard(false)));
    assert_eq!(shell_of(&s, id).status, ShellStatus::Checking { request });

    let (s, commands) = answered(s, id, request, Ok(scan("test", 0)));

    assert_eq!(runs(&commands), vec![(1, QUERY.to_string())]);
    assert_eq!(shell_of(&s, id).notice, None);
}

#[test]
fn turning_the_guard_off_closes_an_index_notice_and_the_next_run_goes() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = missed(plain(), &fake_mongosh(dir.path()));

    let (s, commands) = update(s, Input::Find(FindAction::SetGuard(false)));

    assert!(commands.is_empty());
    assert_eq!(shell_of(&s, id).notice, None);
    let (_, commands) = shell(s, ShellAction::Run);
    assert_eq!(runs(&commands), vec![(1, QUERY.to_string())]);
}
