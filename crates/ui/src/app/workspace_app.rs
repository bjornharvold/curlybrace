//! The workspace file inside the running app: loading it at launch and writing it as the tabs
//! change (#159). The file rules are in `workspace_file`, the reopening of tabs in
//! `workspace_restore`.
use super::persistence::{self, PersistError, PersistGuard, Recovery};
use super::state::{AppState, Banner};
use super::tabs::TabId;
use super::workspace_file::{self, MAX_FILE_BYTES, WorkspaceFile, snapshot};
use super::workspace_restore::RestoreState;
use super::workspace_snapshot::fingerprint;
use super::workspace_snapshot::{Parts, snapshot_parts};
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
    /// How many snapshots were built: a frame in which nothing changed builds none.
    pub snapshots: u64,
    /// The backup copies of the file (they hold query text) are to be deleted.
    pub purge_backups: bool,
    /// The connection list could not be read, so no saved tab can be checked: the file is left
    /// as it is while tabs are reopened, and only the opt-out may rewrite it (with no tabs).
    pub unverified: bool,
    pub(super) pending: Option<(WorkspaceFile, Instant)>,
    pub(super) failed: Option<WorkspaceFile>,
    /// The fingerprint of the state the last snapshot was built from.
    pub(super) seen: Option<u64>,
    /// What tells a Run: the tab in front and the number of the latest find it sent.
    pub(super) marker: Option<(Option<TabId>, u64)>,
    /// The error that began the current streak of failed writes.
    pub(super) streak: Option<PersistError>,
    pub(super) announced: bool,
}

impl AppState {
    /// What the startup load of the connection list found, from its notices: `lost` when it could
    /// not be read at all (its ids prove nothing), `partial` when only some entries were skipped
    /// (the ids that loaded are good, the others are unknown, not deleted).
    fn connections_found(&self) -> (bool, bool) {
        let notices = || {
            self.banner
                .iter()
                .filter_map(|b| match b {
                    Banner::Recovery(r) => Some(r),
                    _ => None,
                })
                .chain(self.pending_recovery.iter())
        };
        let lost = self.persist.check().is_err()
            || notices()
                .any(|r| matches!(r, Recovery::Corrupt { .. } | Recovery::Unreadable { .. }));
        let partial = notices().any(|r| {
            matches!(
                r,
                Recovery::Skipped { .. } | Recovery::SkippedAndRepaired { .. }
            )
        });
        (lost, partial)
    }

    /// The saved tabs when the connection list was lost: they cannot be checked, so they are not
    /// reopened and the file is left alone, but the setting shows what the file says and the
    /// opt-out still works.
    fn workspace_unverified(self, path: std::path::PathBuf) -> Self {
        let (restore, may_save) = super::workspace_read::peek_restore(&path);
        let sync = WorkspaceSync {
            path: Some(path),
            guard: match may_save {
                true => PersistGuard::default(),
                false => PersistGuard::blocked(),
            },
            unverified: true,
            ..WorkspaceSync::default()
        };
        Self {
            restore: Box::new(RestoreState {
                restore,
                sync,
                ..RestoreState::default()
            }),
            ..self
        }
    }

    /// Whether the tabs are not being saved right now, for any reason: no place to save, saving
    /// turned off (a newer file, a failed backup) or a connection list that could not be read.
    pub(super) fn workspace_saving_off(&self) -> bool {
        let sync = &self.restore.sync;
        sync.path.is_none()
            || sync.guard.check().is_err()
            || (sync.unverified && self.restore.restore)
    }

    /// Loads the saved tabs from `path`. They wait, grouped by connection, until their
    /// connection connects; nothing is sent. A file that cannot be used is backed up and no tabs
    /// are reopened, with a notice. An unknown settings folder means no path: nothing is read or
    /// written. So does a connection list that could not be loaded: every saved tab would look
    /// like one of a deleted connection, so the file is left exactly as it is.
    pub fn with_workspace_path(self, path: ConfigPath) -> Self {
        let Ok(path) = path else {
            return self;
        };
        let (lost, partial) = self.connections_found();
        if lost {
            return self.workspace_unverified(path);
        }
        let known: BTreeSet<Uuid> = self.conn.connections.iter().map(|c| c.id).collect();
        // A file too big to be ours is kept cut at the bound, not read whole.
        let backup = |p: &std::path::Path, tag: &str| {
            persistence::back_up_limited(p, tag, Some(MAX_FILE_BYTES as u64 + 1))
        };
        let started = workspace_file::load_with(&path, &backup, &known, partial);
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
        let state = match started.recovery {
            Some(recovery) => state.with_recovery_notice(recovery),
            None => state,
        };
        started
            .also
            .into_iter()
            .fold(state, AppState::with_recovery_notice)
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
    /// typed text is waiting to be written. A frame in which nothing changed does a fingerprint
    /// and no snapshot.
    pub(super) fn workspace_step(&mut self, now: Instant) -> Option<Duration> {
        self.workspace_failure_announced();
        self.workspace_backups_purged();
        if !self.workspace_may_write() {
            return None;
        }
        let fp = fingerprint(self);
        // A Run is the find the tab in front just sent; typing in a stage is not one.
        let marker = (self.active_tab(), self.issued_request);
        let sync = &mut self.restore.sync;
        let ran = sync.marker.replace(marker).is_some_and(|m| m != marker);
        if sync.seen == Some(fp) {
            return self.workspace_waiting_text(now, ran);
        }
        sync.seen = Some(fp);
        // Open tabs that took room the waiting ones had: the oldest of those go, and it is said.
        let trimmed = self.waiting_fitted(uuid::Uuid::nil(), 0, None);
        if trimmed > 0 {
            *self = std::mem::take(self).trimmed_notice_raised(trimmed);
            self.waiting_notice_checked();
        }
        let fp = if trimmed > 0 { fingerprint(self) } else { fp };
        self.restore.sync.seen = Some(fp);
        self.restore.sync.snapshots += 1;
        let parts = snapshot_parts(self);
        self.workspace_marked(&parts);
        let snap = parts.file;
        let sync = &mut self.restore.sync;
        if sync.written.as_ref() == Some(&snap) {
            sync.pending = None;
            return None;
        }
        // The same content that just failed is not tried again until something changes.
        if sync.failed.as_ref() == Some(&snap) {
            // What was typed since is not what the file should get: it is not written later.
            sync.pending = None;
            return None;
        }
        let structural = sync.written.as_ref().is_none_or(|w| !w.same_shape(&snap));
        if structural || ran {
            self.workspace_written(snap);
            return None;
        }
        // Typed text: it is written a second after the last change.
        sync.pending = Some((snap, now));
        Some(TEXT_DEBOUNCE)
    }

    /// What the snapshot says about the tabs of the strip, kept in the state: a tab whose text the
    /// file could not take (the fit dropped it) shows its note while the app runs, and a tab
    /// that was the saved front tab no longer needs remembering once a saved tab is in front.
    fn workspace_marked(&mut self, parts: &Parts) {
        for (at, tab) in parts.live.iter().enumerate() {
            let dropped = parts.file.tabs.get(at).is_some_and(|t| t.text_dropped);
            match dropped {
                true => self.tabs.text_dropped.insert(*tab),
                false => self.tabs.text_dropped.remove(tab),
            };
        }
        if parts.front_saved {
            self.restore.front_live = None;
        }
    }

    /// Nothing changed since the last look: typed text that was waiting is written once its
    /// second is up, or at once on a Run.
    fn workspace_waiting_text(&mut self, now: Instant, ran: bool) -> Option<Duration> {
        let since = self.restore.sync.pending.as_ref()?.1;
        let waited = now.saturating_duration_since(since);
        if ran || waited >= TEXT_DEBOUNCE {
            let (snap, _) = self.restore.sync.pending.take()?;
            self.workspace_written(snap);
            return None;
        }
        Some(TEXT_DEBOUNCE - waited)
    }

    /// The tabs written now, whatever is waiting or failed before.
    pub(super) fn workspace_flush(&mut self) {
        self.workspace_backups_purged();
        if !self.workspace_may_write() {
            return;
        }
        let snap = snapshot(self);
        if self.restore.sync.written.as_ref() != Some(&snap) {
            self.workspace_written(snap);
        }
    }

    fn workspace_may_write(&self) -> bool {
        !self.workspace_saving_off()
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
            Err(e) => self.workspace_failed(snap, e),
        }
    }

    fn workspace_failed(&mut self, snap: WorkspaceFile, e: PersistError) {
        log::warn!("could not save the open tabs: {e:?}");
        let sync = &mut self.restore.sync;
        sync.failed = Some(snap);
        if sync.streak.is_none() {
            sync.streak = Some(e);
            sync.announced = false;
        }
        self.workspace_failure_announced();
    }

    /// The backup copies of the file deleted when the person turned the setting off: they hold
    /// the query text it erased. A copy that cannot be deleted is a failure like a failed write.
    fn workspace_backups_purged(&mut self) {
        let sync = &mut self.restore.sync;
        let (true, Some(path)) = (sync.purge_backups, sync.path.clone()) else {
            return;
        };
        sync.purge_backups = false;
        if let Err(e) = workspace_file::remove_backups(&path) {
            log::warn!("could not delete the backup copies of the saved tabs: {e:?}");
            if sync.streak.is_none() {
                sync.streak = Some(e);
                sync.announced = false;
            }
            self.workspace_failure_announced();
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
pub(super) mod tests;
