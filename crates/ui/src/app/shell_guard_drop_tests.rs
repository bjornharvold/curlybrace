//! The Shell tab's guard check going away: Cancel, a text change, a lost connection or a closed
//! session return the shell to idle and its late answer changes nothing; a stale answer is
//! ignored; and the query bar's check and the shell's each get their own answer.
use super::shell_guard_tests::{
    QUERY, answered, checking, clean, edit, event, fake_mongosh, missed, plain, ready, runs, scan,
    shell, shell_of, the_check,
};
use crate::app::GuardState;
use crate::app::reducer::{Input, update};
use crate::app::shell_actions::ShellAction;
use crate::app::state::AppState;
use crate::viewmodel::FindAction;
use crate::viewmodel::shell::{ShellNotice, ShellStatus};
use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::Query;
use std::path::Path;
use uuid::Uuid;

/// The shell of `id` waiting for guard request 1, after a Run of `script`.
fn waiting_on(exe: &Path, script: &str) -> (AppState, Uuid) {
    let (s, id) = ready(plain(), exe, script);
    let (s, commands) = shell(s, ShellAction::Run);
    assert_eq!(the_check(&commands).1, 1);
    (s, id)
}

fn waiting(exe: &Path) -> (AppState, Uuid) {
    waiting_on(exe, QUERY)
}

/// Every kind of late answer to request 1, each given to a fresh `make()`, leaves its shell as it
/// was and sends nothing.
fn late_answers_change_nothing(make: impl Fn() -> (AppState, Uuid)) {
    for outcome in [
        Ok(clean()),
        Ok(scan("test", 0)),
        Err(CoreError::Unreachable),
    ] {
        let (s, id) = make();
        let before = shell_of(&s, id).clone();
        let (after, commands) = answered(s, id, 1, outcome);
        assert!(commands.is_empty(), "{commands:?}");
        assert_eq!(shell_of(&after, id), &before);
    }
}

#[test]
fn a_stale_answer_is_ignored_and_the_current_one_still_counts() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = waiting(&fake_mongosh(dir.path()));

    let (s, commands) = answered(s, id, 7, Ok(clean()));

    assert!(commands.is_empty());
    assert_eq!(shell_of(&s, id).status, checking(1));
    let (_, commands) = answered(s, id, 1, Ok(clean()));
    assert_eq!(runs(&commands), vec![(1, QUERY.to_string())]);
}

#[test]
fn a_second_run_while_checking_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = waiting(&fake_mongosh(dir.path()));

    let (s, commands) = shell(s, ShellAction::Run);
    assert!(commands.is_empty());
    let (s, commands) = shell(s, ShellAction::RunAnyway);
    assert!(commands.is_empty());

    assert_eq!(shell_of(&s, id).status, checking(1));
    assert_eq!(s.guard_request, 1);
}

#[test]
fn cancel_while_checking_returns_to_idle_and_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let cancelled = || {
        let (s, id) = waiting(&exe);
        let (s, commands) = shell(s, ShellAction::Cancel);
        assert!(
            commands.is_empty(),
            "no ShellCancel for a check: {commands:?}"
        );
        (s, id)
    };

    let (s, id) = cancelled();

    assert_eq!(shell_of(&s, id).status, ShellStatus::Idle);
    late_answers_change_nothing(cancelled);
}

#[test]
fn a_text_change_while_checking_returns_to_idle_but_a_caret_move_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let (s, id) = waiting(&exe);
    let moved = edit(s, QUERY, Some(0..3));
    assert_eq!(shell_of(&moved, id).status, checking(1));
    let typed = || {
        let (s, id) = waiting(&exe);
        (edit(s, "db.people.find({ age: 2 })", None), id)
    };

    let (s, id) = typed();

    assert_eq!(shell_of(&s, id).status, ShellStatus::Idle);
    late_answers_change_nothing(typed);
}

const UNFORMATTED: &str = "if (x) {\ndb.people.find({ age: 1 })\n}";

#[test]
fn formatting_while_checking_returns_to_idle() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let formatted = || {
        let (s, id) = waiting_on(&exe, UNFORMATTED);
        (shell(s, ShellAction::Format).0, id)
    };

    let (s, id) = formatted();

    assert_ne!(shell_of(&s, id).script, UNFORMATTED);
    assert_eq!(shell_of(&s, id).status, ShellStatus::Idle);
    late_answers_change_nothing(formatted);
}

#[test]
fn editing_drops_the_index_notice_and_moving_the_caret_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let is_miss =
        |s: &AppState, id| matches!(shell_of(s, id).notice, Some(ShellNotice::IndexMiss { .. }));

    let (s, id) = missed(plain(), &exe);
    assert!(is_miss(&edit(s, QUERY, Some(0..2)), id));
    let (s, id) = missed(plain(), &exe);
    assert_eq!(shell_of(&edit(s, "db.x.find()", None), id).notice, None);
}

#[test]
fn cancel_drops_the_index_notice() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = missed(plain(), &fake_mongosh(dir.path()));

    let (s, commands) = shell(s, ShellAction::Cancel);

    assert!(commands.is_empty());
    assert_eq!(shell_of(&s, id).notice, None);
    assert_eq!(shell_of(&s, id).status, ShellStatus::Idle);
}

#[test]
fn a_lost_connection_while_checking_resets_the_shell_and_cancels_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let lost = || {
        let (s, id) = waiting(&exe);
        let ev = Event::Failed {
            session: id,
            context: "find",
            error: CoreError::Unreachable,
        };
        let (s, commands) = event(s, ev);
        let cancels = commands
            .iter()
            .any(|c| matches!(c, Command::ShellCancel { .. }));
        assert!(!cancels, "{commands:?}");
        (s, id)
    };

    let (s, id) = lost();

    assert_eq!(shell_of(&s, id).status, ShellStatus::Idle);
    late_answers_change_nothing(lost);
}

#[test]
fn closing_the_session_while_checking_sends_only_the_disconnect() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = waiting(&fake_mongosh(dir.path()));

    let (s, commands) = update(s, Input::Disconnect(id));

    assert!(matches!(commands.as_slice(), [Command::Disconnect { .. }]));
    assert!(s.shell.get(id).is_none());
}

#[test]
fn the_query_bar_and_the_shell_each_get_their_own_answer() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = waiting(&fake_mongosh(dir.path()));
    let query = Query {
        filter: doc! { "age": 3 },
        ..Query::default()
    };
    let (s, commands) = update(s, Input::Find(FindAction::Run(query.clone())));
    assert!(matches!(
        commands.as_slice(),
        [Command::GuardCheck { request: 2, .. }]
    ));
    assert_eq!(s.guard_request, 2);

    let (s, commands) = answered(s, id, 1, Ok(scan("test", 0)));
    assert!(commands.is_empty());
    assert!(matches!(
        shell_of(&s, id).notice,
        Some(ShellNotice::IndexMiss { .. })
    ));
    assert_eq!(*s.guard, GuardState::Checking { request: 2, query });

    let (s, commands) = answered(s, id, 2, Ok(clean()));
    assert!(matches!(commands.first(), Some(Command::Find { .. })));
    assert_eq!(*s.guard, GuardState::None);
    assert!(matches!(
        shell_of(&s, id).notice,
        Some(ShellNotice::IndexMiss { .. })
    ));
}
