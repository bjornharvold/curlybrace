//! The guards a Shell run passes before its text is looked at: the session is connected, its
//! connection is not read-only (the core refuses it too), and its `mongosh` resolves and is not
//! older than the server when both versions are known. Pure: reads the state, changes nothing.
use super::state::{AppState, ConnStatus};
use super::tools_state::{ProbeScope, ProbeStatus};
use crate::viewmodel::shell::{ShellNotice, ToolProblem};
use curlybrace_core::runtime::ShellRefusal;
use curlybrace_core::tools::{Compatibility, ToolKind, compatibility, resolve};
use std::path::PathBuf;
use uuid::Uuid;

/// The guards a run passes before its text is looked at: connected, not read-only, and a
/// usable `mongosh`, which is returned; otherwise the notice that says why not.
pub(super) fn usable(state: &AppState, id: Uuid) -> Result<PathBuf, ShellNotice> {
    if state.status != ConnStatus::Connected {
        return Err(ShellNotice::Refused(ShellRefusal::NotConnected));
    }
    if state.session_read_only(id) {
        return Err(ShellNotice::Refused(ShellRefusal::ReadOnly));
    }
    mongosh(state, id).map_err(ShellNotice::ToolProblem)
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
