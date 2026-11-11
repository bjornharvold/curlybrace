//! The Shell tab of the open sessions: editing, running and cancelling a script, and the events
//! of its runs. Pure: each handler returns the next state plus the runtime commands to send.
//!
//! A run is guarded in this order, and every refusal sends nothing: the session must be
//! connected, its connection must not be read-only (the core refuses it too), `mongosh` must
//! resolve, and must not be older than the server when both versions are known. A production
//! connection then asks for confirmation on every run, through the production write warning.
use super::reducer::Out;
use super::state::{AppState, ConnStatus, Workspace};
use super::tools_state::{ProbeScope, ProbeStatus};
use super::write_guard::{ParkedWrite, WriteKind};
use crate::viewmodel::shell::{ShellNotice, ShellState, ShellStatus, ToolProblem};
use curlybrace_core::runtime::{Command, Event, ShellRefusal};
use curlybrace_core::tools::{Compatibility, ToolKind, compatibility, resolve};
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
        ShellAction::Run => run(state, id),
        ShellAction::Cancel => cancel(state, id),
        ShellAction::ToggleHistory => updated(state, id, ShellState::history_toggled),
        // Only the buffer changes, also mid-run: nothing is sent.
        ShellAction::RecallEntry(entry) => {
            updated(state, id, |shell| shell.history_entry_recalled(entry))
        }
        ShellAction::ClearHistory => updated(state, id, ShellState::clear_requested),
        ShellAction::CancelClear => updated(state, id, ShellState::clear_cancelled),
        ShellAction::ConfirmClear => confirm_clear(state, id),
    }
}

/// The shell of `id` changed by `change`; nothing is sent.
fn updated(state: AppState, id: Uuid, change: impl FnOnce(&ShellState) -> ShellState) -> Out {
    let shell = change(&state.shell_or_new(id));
    unchanged(state.with_shell(id, shell))
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
/// `session`. A failed read means history is unavailable; a failed clear keeps the list (it can
/// still be read) and the panel says the clear did not happen. No banner either way: history is
/// optional, and its store failing says nothing about the connection.
pub(super) fn history_failed(state: AppState, session: Uuid, context: &str) -> Out {
    let Some(shell) = state.shell.get(session) else {
        return unchanged(state);
    };
    let next = match context {
        "clear_shell_history" => shell.history_clear_failed(),
        _ => shell.history_unavailable(),
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
        // The core recorded the run before sending this, so the history read sees it.
        Event::ShellFinished {
            run_id, outcome, ..
        } => {
            let next = shell.finished(run_id, outcome);
            let read = Command::ShellHistory { session };
            return (state.with_shell(session, next), vec![read]);
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
    if shell.is_running() {
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
    let shell = state.shell_or_new(id);
    let next = AppState {
        workspace: Workspace::Shell,
        ..state.with_shell(id, shell)
    };
    // Every time the tab is shown (this is the only way into it): the stored list may have
    // changed since it was last read, a history copied into a duplicated connection included.
    (next, vec![Command::ShellHistory { session: id }])
}

fn run(state: AppState, id: Uuid) -> Out {
    let shell = state.shell_or_new(id);
    if shell.is_running() || state.write_warning.is_some() {
        return unchanged(state);
    }
    if state.status != ConnStatus::Connected {
        return notice(state, id, ShellNotice::Refused(ShellRefusal::NotConnected));
    }
    if state.session_read_only(id) {
        return notice(state, id, ShellNotice::Refused(ShellRefusal::ReadOnly));
    }
    let exe = match mongosh(&state, id) {
        Ok(exe) => exe,
        Err(problem) => return notice(state, id, ShellNotice::ToolProblem(problem)),
    };
    let (started, request) = shell.run_requested();
    // An empty script: the view-model's notice, and no warning about a run that cannot happen.
    let Some(request) = request else {
        return unchanged(state.with_shell(id, started));
    };
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

fn cancel(state: AppState, id: Uuid) -> Out {
    let commands = cancel_running(&state, id).into_iter().collect();
    (state, commands)
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

/// The `mongosh` the session's connection uses, or why it cannot be used.
fn mongosh(state: &AppState, id: Uuid) -> Result<PathBuf, ToolProblem> {
    let choice = state.tools.settings.effective(ToolKind::Mongosh, Some(id));
    let exe = resolve(
        ToolKind::Mongosh,
        &choice,
        state.tool_search_path.as_deref(),
    )
    .map_err(ToolProblem::Missing)?;
    match too_old(state, id) {
        Some(problem) => Err(problem),
        None => Ok(exe),
    }
}

/// `TooOld` when the version check of the session's `mongosh` has answered and the server version
/// is known; anything unknown is not flagged (and no check is run from here).
fn too_old(state: &AppState, id: Uuid) -> Option<ToolProblem> {
    let server = state.shell.server_version(id)?;
    let status = state
        .tools
        .shown_status(ProbeScope::Connection(id), ToolKind::Mongosh);
    let Some(ProbeStatus::Done(Ok(tool))) = status else {
        return None;
    };
    match compatibility(ToolKind::Mongosh, *tool, Some(server)) {
        Compatibility::TooOld => Some(ToolProblem::TooOld {
            tool: *tool,
            server,
        }),
        Compatibility::Ok | Compatibility::Unknown => None,
    }
}

fn notice(state: AppState, id: Uuid, notice: ShellNotice) -> Out {
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
    fn shell_or_new(&self, id: Uuid) -> ShellState {
        self.shell.get_or_new(id)
    }

    fn with_shell(self, id: Uuid, shell: ShellState) -> AppState {
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
