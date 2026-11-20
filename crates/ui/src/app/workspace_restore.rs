//! Reopening the saved tabs when their connection connects, without running anything (#159).
//!
//! At launch the saved tabs wait in `RestoreState::dormant`, grouped by connection id: nothing is
//! sent and nothing shows in the strip. When a connection connects, its saved tabs become
//! `Dormant` tabs holding their namespace, text, sub-tab and view mode. A dormant tab sends
//! nothing when shown; its Run is the only way its query goes out.
#![allow(dead_code)] // wired in by the next commits
use super::state::AppState;
use super::workspace_app::WorkspaceSync;
use super::workspace_file::SavedTab;
use std::collections::BTreeMap;
use uuid::Uuid;

/// What the session restore keeps in the state.
#[derive(Debug)]
pub struct RestoreState {
    /// Whether the open tabs are saved and reopened (the "Reopen tabs at launch" switch).
    pub restore: bool,
    /// The saved tabs of connections that have not connected yet.
    pub dormant: BTreeMap<Uuid, Vec<SavedTab>>,
    /// The saved active tab when its connection has not connected yet: the connection and the
    /// position among that connection's saved tabs.
    pub active_hint: Option<(Uuid, usize)>,
    /// Where and when the file is written.
    pub sync: WorkspaceSync,
}

impl Default for RestoreState {
    fn default() -> Self {
        Self {
            restore: true,
            dormant: BTreeMap::new(),
            active_hint: None,
            sync: WorkspaceSync::default(),
        }
    }
}

/// `session` has connected: its saved tabs become dormant tabs.
pub(super) fn materialise(_state: &mut AppState, _session: Uuid, _background: bool) {}
