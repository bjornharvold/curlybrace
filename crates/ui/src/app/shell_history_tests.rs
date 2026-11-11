//! The Shell tab's history inside the app: what asks the core for it, where its answers go, the
//! recall and clear actions, and how a duplicated or deleted connection takes its history along.
use super::index_write_tests::{browsing, with_flags};
use super::reducer::{Input, update};
use super::shell_actions::ShellAction;
use super::state::AppState;
use super::*;
use crate::viewmodel::shell::ShellState;
use crate::viewmodel::shell_history::ShellHistoryEntry;
use curlybrace_core::error::CoreError;
use curlybrace_core::history::ShellRunStatus;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::secrets::MemorySecretStore;
use curlybrace_core::shell::Outcome;
use curlybrace_core::test_support::mock_objects::mock_connection;
use curlybrace_core::tools::{ToolChoice, ToolKind};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc::UnboundedReceiver;
use uuid::Uuid;

fn shell(s: AppState, action: ShellAction) -> (AppState, Vec<Command>) {
    update(s, Input::Shell(action))
}

fn event(s: AppState, ev: Event) -> (AppState, Vec<Command>) {
    update(s, Input::Event(ev))
}

fn shell_of(s: &AppState, id: Uuid) -> &ShellState {
    s.shell.get(id).expect("the session's shell")
}

fn entry(id: i64, script: &str) -> ShellHistoryEntry {
    ShellHistoryEntry {
        id,
        ran_at_secs: 1_700_000_000 + id,
        script: script.to_string(),
        status: ShellRunStatus::Succeeded,
    }
}

fn history(session: Uuid, entries: Vec<ShellHistoryEntry>) -> Event {
    Event::ShellHistory { session, entries }
}

fn asks_history(commands: &[Command], id: Uuid) -> bool {
    commands
        .iter()
        .any(|c| matches!(c, Command::ShellHistory { session } if *session == id))
}

fn clears(commands: &[Command], id: Uuid) -> usize {
    commands
        .iter()
        .filter(|c| matches!(c, Command::ClearShellHistory { session } if *session == id))
        .count()
}

/// A connected session whose Shell tab is open and whose history holds two runs.
fn with_history() -> (AppState, Uuid) {
    let (s, id) = browsing(with_flags(false, false));
    let (s, _) = shell(s, ShellAction::Open);
    let entries = vec![entry(2, "db.b.find()"), entry(1, "db.a.find()")];
    let (s, _) = event(s, history(id, entries));
    (s, id)
}

/// A runnable stand-in for `mongosh` in `dir` (never run by these tests).
fn fake_mongosh(dir: &Path) -> PathBuf {
    let path = dir.join("mongosh");
    std::fs::write(&path, "#!/bin/sh\necho 2.3.0\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// The history is read only while its panel is open: opening the tab with the panel closed reads
/// nothing, opening it with the panel open reads it (every time, not only the first).
#[test]
fn opening_the_shell_tab_reads_the_history_only_with_the_panel_open() {
    let (s, id) = browsing(with_flags(false, false));
    let (s, commands) = shell(s, ShellAction::Open);
    assert!(commands.is_empty(), "the panel is closed: {commands:?}");
    let (s, _) = shell(s, ShellAction::ToggleHistory);
    let (s, again) = shell(s, ShellAction::Open);
    assert_eq!(reads(&again, id), 1, "{again:?}");
    let (_, third) = shell(s, ShellAction::Open);
    assert_eq!(reads(&third, id), 1, "{third:?}");
}

#[test]
fn opening_the_panel_reads_the_history_and_closing_it_reads_nothing() {
    let (s, id) = browsing(with_flags(false, false));
    let (s, _) = shell(s, ShellAction::Open);
    let (s, opened) = shell(s, ShellAction::ToggleHistory);
    assert_eq!(reads(&opened, id), 1, "{opened:?}");
    assert!(!shell_of(&s, id).history.loaded, "nothing answered yet");
    let (s, closed) = shell(s, ShellAction::ToggleHistory);
    assert!(closed.is_empty(), "{closed:?}");
    // Opened again: read fresh, whatever was loaded before.
    let (s, _) = event(s, history(id, vec![entry(1, "db.a.find()")]));
    let (_, reopened) = shell(s, ShellAction::ToggleHistory);
    assert_eq!(reads(&reopened, id), 1, "{reopened:?}");
}

#[test]
fn nothing_is_read_while_the_panel_is_closed() {
    let (s, id) = with_history();
    assert!(!shell_of(&s, id).history.open);
    let (s, finished) = event(s, finished_event(id));
    assert!(finished.is_empty(), "{finished:?}");
    let (_, changed) = event(s, changed_event(id, true));
    assert!(changed.is_empty(), "{changed:?}");
}

fn reads(commands: &[Command], id: Uuid) -> usize {
    commands
        .iter()
        .filter(|c| matches!(c, Command::ShellHistory { session } if *session == id))
        .count()
}

fn finished_event(id: Uuid) -> Event {
    Event::ShellFinished {
        session: id,
        run_id: 1,
        outcome: Outcome::Exited(0),
    }
}

fn changed_event(id: Uuid, stored: bool) -> Event {
    Event::ShellHistoryChanged {
        session: id,
        stored,
    }
}

#[test]
fn an_open_refused_for_a_session_that_is_not_connected_asks_nothing() {
    let (s, id) = browsing(with_flags(false, false));
    let (s, _) = event(s, lost_event(id));
    let (_, commands) = shell(s, ShellAction::Open);
    assert!(commands.is_empty(), "{commands:?}");
}

#[test]
fn a_recorded_run_reads_the_history_again_while_the_panel_is_open() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let (s, id) = with_history();
    let (s, _) = shell(s, ShellAction::ToggleHistory);
    let tools = s
        .tools
        .choose_override(id, ToolKind::Mongosh, ToolChoice::Executable(exe.clone()));
    let s = AppState { tools, ..s };
    let edit = ShellAction::Edit {
        script: "db.c.find()".into(),
        selection: None,
    };
    let (s, _) = shell(s, edit);
    let (s, run) = shell(s, ShellAction::Run);
    assert!(!asks_history(&run, id), "the run itself asks nothing");
    // The record is written after the run ends: the read waits for it.
    let (s, commands) = event(s, finished_event(id));
    assert!(commands.is_empty(), "{commands:?}");
    let (s, commands) = event(s, changed_event(id, true));
    assert_eq!(reads(&commands, id), 1, "{commands:?}");
    // Any change reads once, a record too long to store included.
    let (_, commands) = event(s, changed_event(id, false));
    assert_eq!(reads(&commands, id), 1, "{commands:?}");
}

#[test]
fn the_history_answer_fills_the_list() {
    let (s, id) = with_history();
    let h = &shell_of(&s, id).history;
    assert!(h.loaded && h.available);
    assert_eq!(h.entries.len(), 2);
    let (s, _) = event(s, history(id, vec![]));
    let h = &shell_of(&s, id).history;
    assert!(h.available && h.entries.is_empty(), "empty is available");
}

#[test]
fn a_history_event_for_another_session_does_not_touch_this_one() {
    let a_cfg = with_flags(false, false);
    let b_cfg = ConnectionConfig {
        name: "other".into(),
        ..mock_connection()
    };
    let (a, b) = (a_cfg.id, b_cfg.id);
    let s = AppState::new(vec![a_cfg, b_cfg]);
    let s = connect_and_answer(s, a);
    let s = connect_and_answer(s, b);
    let (s, _) = update(s, Input::Activate(a));
    let (s, _) = shell(s, ShellAction::Open);
    let (s, _) = update(s, Input::Activate(b));
    let (s, _) = shell(s, ShellAction::Open);
    let before_a = shell_of(&s, a).clone();

    let (s, _) = event(s, history(b, vec![entry(1, "only b")]));
    let (s, commands) = event(s, history(Uuid::new_v4(), vec![entry(9, "nobody")]));

    assert!(commands.is_empty());
    assert_eq!(shell_of(&s, a), &before_a);
    assert_eq!(shell_of(&s, b).history.entries.len(), 1);
}

fn connect_and_answer(s: AppState, id: Uuid) -> AppState {
    let (s, _) = update(s, Input::Connect(id));
    let epoch = s.epoch_counter;
    let ev = Event::Connected {
        session: id,
        epoch,
        topology: None,
    };
    event(s, ev).0
}

#[test]
fn recall_puts_the_script_in_the_editor_and_runs_nothing() {
    let (s, id) = with_history();
    let (s, commands) = shell(s, ShellAction::RecallEntry(1));
    assert!(
        !commands
            .iter()
            .any(|c| matches!(c, Command::ShellRun { .. })),
        "{commands:?}"
    );
    assert!(commands.is_empty());
    assert_eq!(shell_of(&s, id).script, "db.a.find()");
}

#[test]
fn confirmed_clear_sends_the_command_once() {
    let (s, id) = with_history();
    let (s, unasked) = shell(s, ShellAction::ConfirmClear);
    assert_eq!(clears(&unasked, id), 0, "no clear was pending");

    let (s, asked) = shell(s, ShellAction::ClearHistory);
    assert!(asked.is_empty());
    assert!(shell_of(&s, id).history.confirm_clear);
    let (s, confirmed) = shell(s, ShellAction::ConfirmClear);
    assert_eq!(clears(&confirmed, id), 1);
    assert_eq!(
        shell_of(&s, id).history.entries.len(),
        2,
        "until the answer"
    );
    let (s, twice) = shell(s, ShellAction::ConfirmClear);
    assert_eq!(clears(&twice, id), 0, "a second confirm sends nothing");

    let (s, _) = event(s, history(id, vec![]));
    assert!(shell_of(&s, id).history.entries.is_empty());
}

#[test]
fn a_cancelled_clear_sends_nothing() {
    let (s, id) = with_history();
    let (s, _) = shell(s, ShellAction::ClearHistory);
    let (s, cancelled) = shell(s, ShellAction::CancelClear);
    assert!(cancelled.is_empty());
    let (_, confirmed) = shell(s, ShellAction::ConfirmClear);
    assert_eq!(clears(&confirmed, id), 0);
}

#[test]
fn toggling_the_panel_resets_a_pending_clear_confirmation() {
    let (s, id) = with_history();
    let (s, toggled) = shell(s, ShellAction::ToggleHistory);
    assert_eq!(reads(&toggled, id), 1, "opening reads: {toggled:?}");
    assert!(shell_of(&s, id).history.open);
    let (s, _) = shell(s, ShellAction::ClearHistory);
    let (s, _) = shell(s, ShellAction::ToggleHistory);
    let (s, _) = shell(s, ShellAction::ToggleHistory);
    let h = &shell_of(&s, id).history;
    assert!(h.open && !h.confirm_clear);
    let (_, confirmed) = shell(s, ShellAction::ConfirmClear);
    assert_eq!(clears(&confirmed, id), 0, "the stale confirm is gone");
}

fn read_failed(id: Uuid) -> Event {
    Event::Failed {
        session: id,
        context: "shell_history",
        error: CoreError::Unsupported("history".into()),
    }
}

#[test]
fn a_failed_first_read_marks_the_history_unavailable_without_a_banner() {
    let (s, id) = browsing(with_flags(false, false));
    let (s, _) = shell(s, ShellAction::Open);
    let (s, _) = shell(s, ShellAction::ToggleHistory);
    let (s, commands) = event(s, read_failed(id));
    assert!(commands.is_empty());
    assert!(s.banner.is_none(), "{:?}", s.banner);
    let h = &shell_of(&s, id).history;
    assert!(h.loaded && !h.available);
    assert!(h.entries.is_empty());
    assert!(!h.clear_failed && !h.refresh_failed);
}

/// A refresh that fails says nothing about the list already shown: it stays, and the panel says
/// it could not be refreshed, until the next read is sent.
#[test]
fn a_failed_refresh_keeps_the_list_and_says_so_without_a_banner() {
    let (s, id) = with_history();
    let (s, _) = shell(s, ShellAction::ToggleHistory);
    let (s, commands) = event(s, read_failed(id));
    assert!(commands.is_empty());
    assert!(s.banner.is_none(), "{:?}", s.banner);
    let h = &shell_of(&s, id).history;
    assert!(h.loaded && h.available);
    assert_eq!(h.entries.len(), 2);
    assert!(h.refresh_failed);

    let (s, commands) = event(s, changed_event(id, true));
    assert_eq!(reads(&commands, id), 1);
    let h = &shell_of(&s, id).history;
    assert!(!h.refresh_failed, "a new read hides the line");
    assert_eq!(h.entries.len(), 2);
}

/// A clear that failed says nothing about whether the history can be read: the list and its
/// availability stay, the confirm step is over, and the panel says the clear failed.
#[test]
fn a_failed_clear_keeps_the_list_and_says_so_without_a_banner() {
    let (s, id) = with_history();
    let (s, _) = shell(s, ShellAction::ClearHistory);
    let (s, confirmed) = shell(s, ShellAction::ConfirmClear);
    assert_eq!(clears(&confirmed, id), 1);
    let (s, commands) = event(s, clear_failed(id));
    assert!(commands.is_empty(), "{commands:?}");
    assert!(s.banner.is_none(), "{:?}", s.banner);
    let h = &shell_of(&s, id).history;
    assert!(h.loaded && h.available);
    assert_eq!(h.entries.len(), 2);
    assert!(!h.confirm_clear && h.clear_failed);

    // Trying again hides the line; so does a list that arrives.
    let (s, _) = shell(s, ShellAction::ClearHistory);
    assert!(!shell_of(&s, id).history.clear_failed);
    let (s, _) = shell(s, ShellAction::ConfirmClear);
    let (s, _) = event(s, clear_failed(id));
    assert!(shell_of(&s, id).history.clear_failed);
    let (s, _) = event(s, history(id, vec![entry(1, "db.a.find()")]));
    assert!(!shell_of(&s, id).history.clear_failed);
}

fn clear_failed(id: Uuid) -> Event {
    Event::Failed {
        session: id,
        context: "clear_shell_history",
        error: CoreError::Database("disk I/O error".into()),
    }
}

/// History off: the core answers a read with `Failed(shell_history)`, never with an empty list,
/// so the panel says "unavailable" rather than "no scripts yet". A later list makes it usable.
#[test]
fn history_off_reads_as_unavailable_until_a_list_arrives() {
    let (s, id) = browsing(with_flags(false, false));
    let (s, _) = shell(s, ShellAction::Open);
    let off = Event::Failed {
        session: id,
        context: "shell_history",
        error: CoreError::NotFound,
    };
    let (s, commands) = event(s, off);
    assert!(commands.is_empty(), "{commands:?}");
    assert!(s.banner.is_none(), "{:?}", s.banner);
    let h = &shell_of(&s, id).history;
    assert!(h.loaded && !h.available);

    let (s, _) = event(s, history(id, vec![entry(1, "db.a.find()")]));
    let h = &shell_of(&s, id).history;
    assert!(h.loaded && h.available);
    assert_eq!(h.entries.len(), 1);
}

#[test]
fn a_reconnect_keeps_the_history_list() {
    let (s, id) = with_history();
    let s = connect_and_answer(event(s, lost_event(id)).0, id);
    let h = &shell_of(&s, id).history;
    assert!(h.loaded && h.available);
    assert_eq!(
        h.entries.len(),
        2,
        "the connection's runs are still its runs"
    );
    let (s, _) = shell(s, ShellAction::ToggleHistory);
    let (_, reopened) = shell(s, ShellAction::Open);
    assert!(
        asks_history(&reopened, id),
        "and it is read again on open, with the panel open"
    );
}

fn lost_event(id: Uuid) -> Event {
    Event::Failed {
        session: id,
        context: "find",
        error: CoreError::Unreachable,
    }
}

// ---- The running app: duplicate and delete ----

fn app_with(
    dir: &tempfile::TempDir,
    connections: Vec<ConnectionConfig>,
) -> (CurlybraceApp, UnboundedReceiver<Command>) {
    let (tx, commands) = tokio::sync::mpsc::unbounded_channel();
    let (_events, rx) = tokio::sync::mpsc::unbounded_channel();
    let app = CurlybraceApp::new(
        RuntimeHandle { tx, rx },
        I18n::english(),
        connections,
        Arc::new(MemorySecretStore::default()),
    )
    .with_config_path(Ok(dir.path().join("connections.json")));
    (app, commands)
}

fn drain(commands: &mut UnboundedReceiver<Command>) -> Vec<Command> {
    std::iter::from_fn(|| commands.try_recv().ok()).collect()
}

#[test]
fn a_saved_duplicate_copies_the_history_and_an_unsaved_one_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = mock_connection();
    let source = cfg.id;
    let (mut app, mut commands) = app_with(&dir, vec![cfg]);

    app.dispatch_connection(ConnectionAction::Duplicate(source));
    let copy = app
        .state()
        .conn
        .connections
        .iter()
        .find(|c| c.id != source)
        .expect("the copy")
        .id;
    let sent = drain(&mut commands);
    assert!(
        matches!(
            sent.as_slice(),
            [Command::CopyShellHistory { from, to }] if *from == source && *to == copy
        ),
        "one copy, and no read of the new connection's history: {sent:?}"
    );

    app.state_mut().persist = persistence::PersistGuard::blocked();
    app.dispatch_connection(ConnectionAction::Duplicate(source));
    assert_eq!(app.state().conn.connections.len(), 2, "refused");
    let sent = drain(&mut commands);
    assert!(sent.is_empty(), "{sent:?}");
}

#[test]
fn a_delete_purges_the_history_through_purge_history() {
    let dir = tempfile::tempdir().unwrap();
    let (kept, gone) = (
        mock_connection(),
        ConnectionConfig {
            name: "gone".into(),
            ..mock_connection()
        },
    );
    let (kept_id, gone_id) = (kept.id, gone.id);
    let (mut app, mut commands) = app_with(&dir, vec![kept, gone]);

    app.state_mut().persist = persistence::PersistGuard::blocked();
    app.dispatch_connection(ConnectionAction::Delete(gone_id));
    assert!(app.state().connection(gone_id).is_some(), "refused");
    let sent = drain(&mut commands);
    assert!(
        sent.is_empty(),
        "an unsaved delete purges nothing: {sent:?}"
    );

    app.state_mut().persist = persistence::PersistGuard::default();
    app.dispatch_connection(ConnectionAction::Delete(gone_id));
    let sent = drain(&mut commands);
    assert!(
        matches!(
            sent.as_slice(),
            [Command::PurgeHistory { connection }] if *connection == gone_id
        ),
        "{sent:?}"
    );
    assert!(app.state().connection(kept_id).is_some());
}
