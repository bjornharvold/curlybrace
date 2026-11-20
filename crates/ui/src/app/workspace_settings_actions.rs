//! The "Reopen tabs at launch" switch of the Settings dialog (#159).
use super::state::AppState;

/// The switch set. Off: the saved tabs still waiting for their connection are dropped and the
/// next write (the frame this action ends) rewrites the file with no tabs, so saved query text is
/// erased now and not at the next launch. On: the open tabs are written again as they change.
pub(super) fn restore_set(state: AppState, on: bool) -> AppState {
    let mut state = state;
    state.restore.restore = on;
    if !on {
        state.restore.dormant.clear();
        state.restore.active_hint = None;
        state.restore.split_hint = None;
    }
    state
}

#[cfg(test)]
#[path = "workspace_settings_actions_tests.rs"]
mod tests;
