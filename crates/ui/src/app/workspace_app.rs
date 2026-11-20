//! The workspace file inside the running app: loading it at launch and writing it as the tabs
//! change (#159). The file rules are in `workspace_file`, the reopening of tabs in
//! `workspace_restore`.
#![allow(dead_code)] // wired in by the next commits
use super::persistence::{self, PersistGuard};
use super::state::AppState;
use super::tabs::TabId;
use super::workspace_file::{self, WorkspaceFile, snapshot};
use super::workspace_restore::RestoreState;
use super::{ConfigPath, CurlybraceApp};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;
use uuid::Uuid;

/// Where and when `workspace.json` is written.
#[derive(Debug, Default)]
pub struct WorkspaceSync {
    /// `None` until a path is set: nothing is ever written then (a new app never touches the
    /// real settings folder).
    pub path: Option<PathBuf>,
    pub guard: PersistGuard,
    /// What the file holds, as far as this window knows.
    pub written: Option<WorkspaceFile>,
    pub writes: u64,
    pub(super) pending: Option<(WorkspaceFile, Instant)>,
    pub(super) failed: Option<WorkspaceFile>,
    pub(super) marker: Option<(Option<TabId>, u64, u64)>,
    pub(super) streak: bool,
    pub(super) announced: bool,
}

impl AppState {
    /// Loads the saved tabs from `path`. They wait, grouped by connection, until their
    /// connection connects; nothing is sent. A file that cannot be used is backed up and no tabs
    /// are reopened, with a notice. An unknown settings folder means no path: nothing is read or
    /// written.
    pub fn with_workspace_path(self, path: ConfigPath) -> Self {
        let Ok(path) = path else {
            return self;
        };
        let known: BTreeSet<Uuid> = self.conn.connections.iter().map(|c| c.id).collect();
        let started = workspace_file::load(&path, &persistence::back_up, &known);
        let sync = WorkspaceSync {
            path: Some(path),
            guard: started.guard,
            ..WorkspaceSync::default()
        };
        let state = Self {
            restore: Box::new(RestoreState::of_file(&started.file, sync)),
            ..self
        };
        // A file that was fine stays as it is until something changes; one that was repaired is
        // written clean by the first save.
        let state = match started.recovery.is_none() && !started.needs_rewrite {
            true => {
                let written = snapshot(&state);
                let mut state = state;
                state.restore.sync.written = Some(written);
                state
            }
            false => state,
        };
        match started.recovery {
            Some(recovery) => state.with_recovery_notice(recovery),
            None => state,
        }
    }
}

impl CurlybraceApp {
    /// Loads the saved tabs from `path` (see [`AppState::with_workspace_path`]).
    pub fn with_workspace_path(mut self, path: ConfigPath) -> Self {
        self.state = std::mem::take(&mut self.state).with_workspace_path(path);
        self
    }
}
