//! The reducer's connection-outcome helpers: applying a connect-family outcome at its epoch, and
//! what a failed or lost connection does to the state.
use super::shell_actions;
use super::state::{AppState, Banner};
use curlybrace_core::error::CoreError;

use super::reducer::Out;

/// Applies a connect-family outcome unless a newer Connect/Disconnect was requested since.
pub(super) fn at_epoch(state: AppState, epoch: u64, apply: impl FnOnce(AppState) -> Out) -> Out {
    if epoch < state.latest_requested_epoch {
        return (state, Vec::new());
    }
    apply(AppState {
        latest_requested_epoch: state.latest_requested_epoch.max(epoch),
        ..state
    })
}

/// The state after a session connected: its own data-failure banner is stale now; any other
/// banner (another session's included) stays.
pub(super) fn after_connected(state: AppState) -> AppState {
    let stale = state.banner_owner() == state.active
        && state.banner.as_ref().is_some_and(Banner::is_data_failure);
    if stale { state.without_banner() } else { state }
}

/// The active session's connection failed (connect) or was lost (a data command): its root
/// stays (marked failed, showing the error), its view is cleared, and the user is told through
/// the banner.
///
/// Only a background (not focused) session leaves another session's banner alone: its root shows
/// `Failed` and its error in the tree. The focused session always raises its own banner.
pub(super) fn connection_lost(
    state: AppState,
    context: &'static str,
    error: CoreError,
    background: bool,
) -> AppState {
    if background
        && state
            .banner_owner()
            .is_some_and(|owner| Some(owner) != state.active)
    {
        let code_export = code_export_without_failing(&state);
        return AppState {
            code_export,
            ..state.failed(error)
        };
    }
    let banner = Some(Banner::Failure {
        context,
        error: error.clone(),
    });
    // Captured here, while the failing session's view is the active one.
    let forget_target = (context == "connect" && error == CoreError::SshHostKeyChanged)
        .then(|| state.ssh_endpoint.clone())
        .flatten();
    let banner_session = state.active;
    let code_export = code_export_without_failing(&state);
    AppState {
        banner,
        banner_session,
        forget_target,
        code_export,
        ..state.failed(error)
    }
}

/// The Copy as code state without its dialog when that was opened on the failing session (the
/// active one while this runs, see `with_session`): its view is cleared with it. A dialog of
/// another session, the one in view when a background session fails, stays.
fn code_export_without_failing(state: &AppState) -> Box<crate::viewmodel::CodeExportUi> {
    let failing = state.session_id();
    let owned = state
        .code_export
        .dialog
        .as_ref()
        .is_some_and(|dialog| dialog.session == failing);
    match owned {
        true => Box::new((*state.code_export).clone().closed()),
        false => state.code_export.clone(),
    }
}

/// `connection_lost`, plus a cancel of the session's running shell: `mongosh` holds its own
/// connection and may outlive the blip, and once the shell is reset to idle nothing else could
/// stop it. Read while the failing session is the active one.
pub(super) fn lost_with_shell_cancelled(
    state: AppState,
    context: &'static str,
    error: CoreError,
    background: bool,
) -> Out {
    let cancel = shell_actions::cancel_running(&state, state.session_id());
    (
        connection_lost(state, context, error, background),
        cancel.into_iter().collect(),
    )
}
