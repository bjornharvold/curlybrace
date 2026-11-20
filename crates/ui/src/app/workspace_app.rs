//! The workspace file inside the running app: loading it at launch and writing it as the tabs
//! change (#159). The file rules are in `workspace_file`, the reopening of tabs in
//! `workspace_restore`.
use super::persistence::{self, PersistError, PersistGuard};
use super::state::{AppState, Banner};
use super::tabs::TabId;
use super::workspace_file::{self, WorkspaceFile, snapshot};
use super::workspace_restore::RestoreState;
use super::{ConfigPath, CurlybraceApp};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};
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
    /// The error that began the current streak of failed writes.
    pub(super) streak: Option<PersistError>,
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

    /// Called at the end of every frame: writes the tabs when they changed. A change of
    /// structure (a tab opened, closed or moved, a namespace, sub-tab, view mode, split, the
    /// setting) and a Run are written at once; typed text one second after the last change.
    /// Never when saving is blocked or there is no path.
    pub fn save_workspace_if_changed(&mut self, ctx: &eframe::egui::Context, now: Instant) {
        if let Some(wait) = self.state.workspace_step(now) {
            ctx.request_repaint_after(wait);
        }
    }

    /// Writes the tabs as they are, without waiting (the app is closing).
    pub fn save_workspace_now(&mut self) {
        self.state.workspace_flush();
    }
}

/// How long typed text rests before it is written.
const TEXT_DEBOUNCE: Duration = Duration::from_secs(1);

impl AppState {
    /// One look at the tabs against the file. Returns how long to wait before looking again when
    /// typed text is waiting to be written.
    pub(super) fn workspace_step(&mut self, now: Instant) -> Option<Duration> {
        self.workspace_failure_announced();
        if !self.workspace_may_write() {
            return None;
        }
        let snap = snapshot(self);
        let marker = (
            self.active_tab(),
            self.issued_request,
            self.aggregate.request,
        );
        let sync = &mut self.restore.sync;
        let ran = sync.marker.replace(marker).is_some_and(|m| m != marker);
        if sync.written.as_ref() == Some(&snap) {
            sync.pending = None;
            return None;
        }
        // The same content that just failed is not tried again until something changes.
        if sync.failed.as_ref() == Some(&snap) {
            return None;
        }
        let structural = sync.written.as_ref().is_none_or(|w| !w.same_shape(&snap));
        if structural || ran {
            self.workspace_written(snap);
            return None;
        }
        let since = match sync.pending.take() {
            Some((pending, at)) if pending == snap => at,
            _ => now,
        };
        let waited = now.saturating_duration_since(since);
        if waited >= TEXT_DEBOUNCE {
            self.workspace_written(snap);
            return None;
        }
        self.restore.sync.pending = Some((snap, since));
        Some(TEXT_DEBOUNCE - waited)
    }

    /// The tabs written now, whatever is waiting or failed before.
    pub(super) fn workspace_flush(&mut self) {
        if !self.workspace_may_write() {
            return;
        }
        let snap = snapshot(self);
        if self.restore.sync.written.as_ref() != Some(&snap) {
            self.workspace_written(snap);
        }
    }

    fn workspace_may_write(&self) -> bool {
        let sync = &self.restore.sync;
        sync.path.is_some() && sync.guard.check().is_ok()
    }

    /// Writes `snap`. A failure is logged (the error kind only, never a path or text) and raises
    /// one banner per streak of failures.
    fn workspace_written(&mut self, snap: WorkspaceFile) {
        let sync = &mut self.restore.sync;
        let Some(path) = sync.path.clone() else {
            return;
        };
        sync.pending = None;
        match workspace_file::save(&path, &sync.guard, &snap) {
            Ok(()) => {
                sync.written = Some(snap);
                sync.writes += 1;
                sync.failed = None;
                sync.streak = None;
                sync.announced = false;
            }
            Err(e) => {
                log::warn!("could not save the open tabs: {e:?}");
                sync.failed = Some(snap);
                if sync.streak.is_none() {
                    sync.streak = Some(e);
                    sync.announced = false;
                }
                self.workspace_failure_announced();
            }
        }
    }

    /// The banner of a failure streak, once, as soon as the banner slot is free.
    fn workspace_failure_announced(&mut self) {
        let sync = &mut self.restore.sync;
        let Some(error) = sync.streak.clone().filter(|_| !sync.announced) else {
            return;
        };
        if self.banner.is_none() {
            sync.announced = true;
            self.banner = Some(Banner::WorkspacePersist(error));
        }
    }
}

#[cfg(test)]
#[path = "workspace_app_tests.rs"]
mod tests;
