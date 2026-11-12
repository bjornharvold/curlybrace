//! The Shell tab of the open sessions: editing, running and cancelling a script, and the events
//! of its runs. Pure: each handler returns the next state plus the runtime commands to send.
//!
//! A run is guarded in this order, and every refusal sends nothing: the session must be
//! connected, its connection must not be read-only (the core refuses it too), `mongosh` must
//! resolve, and must not be older than the server when both versions are known. The text then
//! passes the syntax check and, with the guard on, the index-miss check (`shell_guard`), whose
//! answer comes back later. A production connection then asks for confirmation on every run,
//! through the production write warning.
use super::reducer::Out;
use super::shell_preflight::usable;
use super::state::{AppState, ConnStatus, Workspace};
use super::write_guard::{ParkedWrite, WriteKind};
use crate::viewmodel::shell::{ShellNotice, ShellState, ShellStatus};
use crate::viewmodel::shell_result::OutputMode;
use curlybrace_core::runtime::{Command, Event};
use std::ops::Range;
use std::path::PathBuf;
use uuid::Uuid;

/// What the Shell tab asks for. Every action applies to the active session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellAction {
    Edit {
        script: String,
        selection: Option<Range<usize>>,
    },
    Run,
    /// Run without the syntax check, after it stopped a Run. Every other guard of Run applies;
    /// after an index notice the index check is skipped too (it runs exactly what was checked).
    RunAnyway,
    /// Select the bytes of the syntax notice's problem in the editor. Never runs anything.
    GoToProblem,
    /// Stop the run, or the index check before it; also drops an index notice.
    Cancel,
    Format,
    /// Show the Shell tab of the active session (only while it is connected).
    Open,
    /// Open or close the history panel; either way a pending clear is dropped.
    ToggleHistory,
    /// Put the script of the history entry with this id in the editor. Never runs it.
    RecallEntry(i64),
    /// Ask to clear the connection's history (the first step: confirmation is pending).
    ClearHistory,
    ConfirmClear,
    CancelClear,
    /// Show the output as text, or the run's last value as a table or a tree. Kept across runs.
    SetOutputMode(OutputMode),
    /// Open or close this path of the tree's document with this index.
    TreeToggle(usize, Vec<String>),
    TreeExpandAll,
    TreeCollapseAll,
}

pub(super) fn on_action(state: AppState, action: ShellAction) -> Out {
    let Some(id) = state.active else {
        return unchanged(state);
    };
    match action {
        ShellAction::Edit { script, selection } => {
            let shell = state.shell_or_new(id).edited(script, selection);
            unchanged(state.with_shell(id, shell))
        }
        ShellAction::Format => {
            let shell = state.shell_or_new(id).formatted();
            unchanged(state.with_shell(id, shell))
        }
        ShellAction::Open => open(state, id),
        ShellAction::Run => run(state, id, Check::Syntax),
        ShellAction::RunAnyway => run(state, id, Check::Skip),
        ShellAction::GoToProblem => updated(state, id, ShellState::problem_selected),
        ShellAction::Cancel => cancel(state, id),
        ShellAction::ToggleHistory => toggle_history(state, id),
        // Only the buffer changes, also mid-run: nothing is sent.
        ShellAction::RecallEntry(entry) => {
            updated(state, id, |shell| shell.history_entry_recalled(entry))
        }
        ShellAction::ClearHistory => updated(state, id, ShellState::clear_requested),
        ShellAction::CancelClear => updated(state, id, ShellState::clear_cancelled),
        ShellAction::ConfirmClear => confirm_clear(state, id),
        // How the result is shown: only the view changes, also mid-run; nothing is sent.
        ShellAction::SetOutputMode(mode) => updated(state, id, |shell| shell.mode_set(mode)),
        ShellAction::TreeToggle(doc_index, path) => {
            updated(state, id, |shell| shell.tree_toggled(doc_index, path))
        }
        ShellAction::TreeExpandAll => updated(state, id, |shell| shell.tree_all_set(true)),
        ShellAction::TreeCollapseAll => updated(state, id, |shell| shell.tree_all_set(false)),
    }
}

/// The shell of `id` changed by `change`; nothing is sent.
fn updated(state: AppState, id: Uuid, change: impl FnOnce(&ShellState) -> ShellState) -> Out {
    let shell = change(&state.shell_or_new(id));
    unchanged(state.with_shell(id, shell))
}

/// Opens or closes the panel; opening it reads the history fresh (nothing is read while it is
/// closed, so what it showed before may be stale).
fn toggle_history(state: AppState, id: Uuid) -> Out {
    let (shell, commands) = read_if_open(&state.shell_or_new(id).history_toggled(), id);
    (state.with_shell(id, shell), commands)
}

/// The history read, only while `shell`'s panel is open (a closed panel never reads), with the
/// shell that sent it.
fn read_if_open(shell: &ShellState, id: Uuid) -> (ShellState, Vec<Command>) {
    match shell.history.open {
        true => (
            shell.history_read_requested(),
            vec![Command::ShellHistory { session: id }],
        ),
        false => (shell.clone(), Vec::new()),
    }
}

/// Sends the clear only when one was pending. The list stays until the core answers with the
/// (now empty) history.
fn confirm_clear(state: AppState, id: Uuid) -> Out {
    let (shell, send) = state.shell_or_new(id).clear_confirmed();
    let commands = match send {
        true => vec![Command::ClearShellHistory { session: id }],
        false => Vec::new(),
    };
    (state.with_shell(id, shell), commands)
}

/// A failed read (`shell_history`) or clear (`clear_shell_history`) of the history of
/// `session`. A failed first read means history is unavailable; a failed refresh of a list
/// already shown keeps it and the panel says it could not be refreshed; a failed clear keeps the
/// list (it can still be read) and the panel says the clear did not happen. No banner in any
/// case: history is optional, and its store failing says nothing about the connection.
pub(super) fn history_failed(state: AppState, session: Uuid, context: &str) -> Out {
    let Some(shell) = state.shell.get(session) else {
        return unchanged(state);
    };
    let next = match context {
        "clear_shell_history" => shell.history_clear_failed(),
        _ => shell.history_read_failed(),
    };
    unchanged(state.with_shell(session, next))
}

/// A shell event of the open session `session`; the view-model ignores the ones of a run it no
/// longer shows, and a session without a shell has nothing to update.
pub(super) fn on_event(state: AppState, session: Uuid, event: Event) -> Out {
    let Some(shell) = state.shell.get(session) else {
        return unchanged(state);
    };
    let next = match event {
        Event::ShellOutput {
            run_id,
            stream,
            text,
            ..
        } => shell.output_arrived(run_id, stream, &text),
        // Moved, not cloned: the view-model wraps it in an `Arc` once.
        Event::ShellResult { run_id, result, .. } => shell.result_arrived(run_id, result),
        Event::ShellFinished {
            run_id, outcome, ..
        } => shell.finished(run_id, outcome),
        // The record of a run is written after its `ShellFinished`; the read is queued behind it.
        // Only an open panel reads; a closed one reads when it is opened, but still notes a
        // script that was too long to remember.
        Event::ShellHistoryChanged { stored, .. } => {
            let (next, commands) = read_if_open(&shell.history_changed(stored), session);
            return (state.with_shell(session, next), commands);
        }
        Event::ShellRefused { run_id, reason, .. } => shell.refused(run_id, reason),
        Event::ShellHistory { entries, .. } => shell.history_loaded(entries),
        _ => return unchanged(state),
    };
    unchanged(state.with_shell(session, next))
}

/// The user confirmed a parked run on a production connection. It is sent only if the opening it
/// was aimed at is still the connected active session and the shell is still idle; the script
/// sent is the one parked when Run was clicked.
pub(super) fn confirmed(state: AppState, parked: ParkedWrite) -> Out {
    let unchanged_connection = super::write_guard::connection_unchanged(&state, &parked);
    let Command::ShellRun {
        session,
        exe,
        script,
        ..
    } = parked.command
    else {
        return unchanged(state);
    };
    if !unchanged_connection {
        let banner = Some(super::state::Banner::WriteNotSentConnectionChanged);
        return unchanged(AppState { banner, ..state });
    }
    let shell = state.shell_or_new(session);
    if shell.is_busy() {
        return unchanged(state);
    }
    // The script as it was when Run was clicked runs, whatever the editor holds now.
    let (shell, request) = shell.started(&script);
    let command = Command::ShellRun {
        session,
        run_id: request.run_id,
        exe,
        script: request.script,
    };
    (state.with_shell(session, shell), vec![command])
}

fn open(state: AppState, id: Uuid) -> Out {
    if state.status != ConnStatus::Connected {
        return unchanged(state);
    }
    // Every time the tab is shown with its panel open (this is the only way into it): the
    // stored list may have changed since it was last read, a copied history included.
    let (shell, commands) = read_if_open(&state.shell_or_new(id), id);
    let next = AppState {
        workspace: Workspace::Shell,
        ..state.with_shell(id, shell)
    };
    (next, commands)
}

/// Whether a Run checks the text's syntax first: Run does, Run anyway does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Check {
    Syntax,
    Skip,
}

/// Run and Run anyway share every guard; only the syntax check differs, and Run anyway on an
/// index notice skips the index check too. A run parked for the production confirmation is
/// never checked again: `confirmed` sends the parked text.
fn run(state: AppState, id: Uuid, check: Check) -> Out {
    let shell = state.shell_or_new(id);
    if shell.is_busy() || state.write_warning.is_some() {
        return unchanged(state);
    }
    let exe = match usable(&state, id) {
        Ok(exe) => exe,
        Err(refusal) => return notice(state, id, refusal),
    };
    let index_checked =
        check == Check::Syntax || !matches!(shell.notice, Some(ShellNotice::IndexMiss { .. }));
    // An empty or broken script: the view-model's notice, and no warning about a run that
    // does not happen.
    let range = match shell.run_target(check == Check::Syntax) {
        Ok(range) => range,
        Err(refused) => return unchanged(state.with_shell(id, *refused)),
    };
    if !index_checked {
        return send(state, id, &shell, exe, range);
    }
    match super::shell_guard::check_first(state, id, &shell, range.clone()) {
        super::shell_guard::Guarded::Checking(out) => out,
        super::shell_guard::Guarded::Clear(state) => send(state, id, &shell, exe, range),
    }
}

/// Runs `range` of `shell`'s script (`None`: all of it) with `exe`, after every check passed:
/// at once, or on a production connection parked in the write warning.
pub(super) fn send(
    state: AppState,
    id: Uuid,
    shell: &ShellState,
    exe: PathBuf,
    range: Option<Range<usize>>,
) -> Out {
    let (started, request) = shell.started(shell.text_of(&range));
    let command = Command::ShellRun {
        session: id,
        run_id: request.run_id,
        exe,
        script: request.script,
    };
    if state.session_production(id) {
        // The run id in the parked command is provisional: `confirmed` hands out the real one.
        // Nothing is marked running before then.
        let write_warning = Some(ParkedWrite {
            command,
            pending: None,
            kind: WriteKind::ShellRun,
            epoch: state.latest_requested_epoch,
        });
        return unchanged(AppState {
            write_warning,
            ..state
        });
    }
    (state.with_shell(id, started), vec![command])
}

/// Stops the running `mongosh`, or forgets the index check before a run (nothing was sent to
/// stop: its late answer is ignored); an index notice goes too.
fn cancel(state: AppState, id: Uuid) -> Out {
    let commands = cancel_running(&state, id).into_iter().collect();
    let shell = state.shell.get(id).map(ShellState::check_cancelled);
    let next = match shell {
        Some(shell) => state.with_shell(id, shell),
        None => state,
    };
    (next, commands)
}

/// The cancel of the run the shell of `id` shows as running, if any. Sent by the user's Cancel
/// and, under one rule, wherever a running shell is reset or dropped (connection lost, reconnect,
/// close), so a `mongosh` the tab no longer tracks is never left running.
pub(super) fn cancel_running(state: &AppState, id: Uuid) -> Option<Command> {
    match state.shell.get(id).map(|s| s.status) {
        Some(ShellStatus::Running { run_id }) => Some(Command::ShellCancel {
            session: id,
            run_id,
        }),
        _ => None,
    }
}

/// A Run refused before it started: only the notice changes. Nothing ran, so the previous run's
/// output and value stay, as for an empty script.
pub(super) fn notice(state: AppState, id: Uuid, notice: ShellNotice) -> Out {
    let shell = ShellState {
        notice: Some(notice),
        ..state.shell_or_new(id)
    };
    unchanged(state.with_shell(id, shell))
}

fn unchanged(state: AppState) -> Out {
    (state, Vec::new())
}

impl AppState {
    pub(super) fn shell_or_new(&self, id: Uuid) -> ShellState {
        self.shell.get_or_new(id)
    }

    pub(super) fn with_shell(self, id: Uuid, shell: ShellState) -> AppState {
        let tabs = Box::new((*self.shell).with(id, shell));
        AppState {
            shell: tabs,
            ..self
        }
    }

    /// See `ShellTabs::reset`: the connection of `id` was lost or is being reopened.
    pub(super) fn shell_reset_for(self, id: Uuid) -> AppState {
        let tabs = Box::new((*self.shell).reset(id));
        AppState {
            shell: tabs,
            ..self
        }
    }

    /// See `ShellTabs::dropped`: the session `id` was closed.
    pub(super) fn shell_dropped_for(self, id: Uuid) -> AppState {
        let tabs = Box::new((*self.shell).dropped(id));
        AppState {
            shell: tabs,
            ..self
        }
    }
}

// Declared here, not in `app/mod.rs` (kept from growing), so its path is
// `app::shell_actions::shell_result_app_tests`: run with `cargo test shell_result`.
#[cfg(test)]
#[path = "shell_result_app_tests.rs"]
mod shell_result_app_tests;

// Declared here for the same reason: run with `cargo test shell_check`.
#[cfg(test)]
#[path = "shell_check_app_tests.rs"]
mod shell_check_app_tests;
