//! The "Reopen tabs at launch" switch of the Settings dialog (#159).
use super::persistence::PersistError;
use super::state::{AppState, Banner};

/// The switch set. Off: the saved tabs still waiting for their connection are dropped and the
/// next write (the frame this action ends) rewrites the file with no tabs, so saved query text is
/// erased now and not at the next launch, and the backup copies of the file (they hold query text
/// too) are deleted. On: the open tabs are written again as they change. While saving is turned
/// off (a newer file, a failed backup) the switch changes nothing in the file, and says so.
pub(super) fn restore_set(state: AppState, on: bool) -> AppState {
    let mut state = state;
    state.restore.restore = on;
    let sync = &state.restore.sync;
    if sync.path.is_some() && sync.guard.check().is_err() && state.banner.is_none() {
        state.banner = Some(Banner::WorkspacePersist(PersistError::Blocked));
    }
    if !on {
        state.restore.sync.purge_backups = true;
        state.restore.dormant.clear();
        state.restore.active_hint = None;
        state.restore.split_hint = None;
    }
    state
}

#[cfg(test)]
#[path = "workspace_settings_actions_tests.rs"]
mod tests;
