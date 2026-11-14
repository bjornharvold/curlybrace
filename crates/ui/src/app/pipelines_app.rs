//! The saved pipelines inside the running app: loading `pipelines.json`, writing it, and
//! following a connection that is deleted or duplicated. The pure rules are in
//! `curlybrace_core::saved_pipeline`; the file rules are in `pipelines_store`.
use super::connection_ops;
use super::persistence::{self, PersistError};
use super::pipelines_store::{self, PipelinesSaveError};
use super::state::AppState;
use super::{ConfigPath, CurlybraceApp};
use curlybrace_core::saved_pipeline::SavedPipelines;
use uuid::Uuid;

impl AppState {
    /// Loads the saved pipelines from `path`. A file that cannot be used is backed up and the
    /// store starts empty, with the recovery banner (an earlier banner keeps its place and
    /// this one waits behind it). An unknown config folder keeps the store in memory only.
    pub fn with_pipelines_path(self, path: ConfigPath) -> Self {
        match path {
            Ok(path) => {
                let started = pipelines_store::load(&path, &persistence::back_up);
                let state = Self {
                    pipelines: started.settings,
                    pipelines_path: Some(path),
                    pipelines_guard: started.guard,
                    pipelines_seen: started.seen,
                    ..self
                };
                match started.recovery {
                    Some(recovery) => state.with_recovery_notice(recovery),
                    None => state,
                }
            }
            Err(_) => Self {
                pipelines: SavedPipelines::default(),
                pipelines_path: None,
                pipelines_guard: Default::default(),
                pipelines_seen: Default::default(),
                ..self
            },
        }
    }

    /// Writes `next` and makes it the list only once it is on disk: a failed write (or a
    /// blocked file) leaves `pipelines` exactly as it was. When another window changed the file
    /// since this one last read or wrote it, nothing is written and the list is replaced by
    /// what the file now holds, so the change can be made again on top of it.
    pub fn persist_pipelines(&mut self, next: SavedPipelines) -> Result<(), PipelinesSaveError> {
        let Some(path) = self.pipelines_path.clone() else {
            return Err(PipelinesSaveError::Persist(PersistError::ConfigDirUnknown));
        };
        match pipelines_store::saved(&path, &self.pipelines_guard, self.pipelines_seen, &next) {
            Ok(seen) => {
                self.pipelines = next;
                self.pipelines_seen = seen;
                Ok(())
            }
            Err(PipelinesSaveError::ChangedOnDisk) => Err(self.follow_the_file(&path)),
            Err(e) => Err(e),
        }
    }

    /// Another window changed the file: take what it holds, or, when that cannot be used, keep
    /// a copy of it and accept it as the thing the next save replaces. A copy that cannot be
    /// made blocks saving, so an unreadable file is never overwritten unseen.
    fn follow_the_file(&mut self, path: &std::path::Path) -> PipelinesSaveError {
        match pipelines_store::reloaded(path) {
            pipelines_store::Reload::Usable(store, seen) => {
                self.pipelines = store;
                self.pipelines_seen = seen;
                PipelinesSaveError::ChangedOnDisk
            }
            pipelines_store::Reload::Unusable { seen, tag } => {
                match persistence::back_up(path, tag) {
                    Ok(_) => {
                        self.pipelines_seen = seen;
                        PipelinesSaveError::UnreadableOnDisk
                    }
                    Err(e) => {
                        log::warn!("could not back up the saved pipelines ({tag}): {e}");
                        self.pipelines_guard = persistence::PersistGuard::blocked();
                        PipelinesSaveError::Blocked
                    }
                }
            }
        }
    }

    /// A cleanup after a connection change: the list follows at once (a stale entry for a
    /// deleted connection would only be written again later) and the write is tried; a failure
    /// is logged and shown unless another banner is up.
    fn apply_pipelines_cleanup(&mut self, next: SavedPipelines) {
        if next == self.pipelines {
            return;
        }
        let written = self.persist_pipelines(next.clone());
        // Another window's list (just reloaded) is kept over this stale cleanup.
        if written != Err(PipelinesSaveError::ChangedOnDisk) {
            self.pipelines = next;
        }
        if let Err(e) = written {
            log::warn!("could not save the saved pipelines: {e:?}");
            if self.banner.is_none() {
                self.banner = Some(e.banner());
            }
        }
    }
}

impl CurlybraceApp {
    /// Loads the saved pipelines from `path` (see [`AppState::with_pipelines_path`]).
    pub fn with_pipelines_path(mut self, path: ConfigPath) -> Self {
        self.state = std::mem::take(&mut self.state).with_pipelines_path(path);
        self
    }

    /// A delete that reached the disk takes the pipelines of the connections it removed along.
    pub(super) fn forget_pipelines_of_removed(&mut self, saved: bool, before: &[Uuid]) {
        if !saved {
            return;
        }
        let next = before
            .iter()
            .filter(|id| self.state.connection(**id).is_none())
            .fold(self.state.pipelines.clone(), |store, id| {
                store.without_connection(*id)
            });
        self.state.apply_pipelines_cleanup(next);
    }

    /// A duplicate starts with the pipelines of the connection it was copied from.
    pub(super) fn copy_pipelines_to_new(&mut self, source: Uuid, before: &[Uuid]) {
        let next = connection_ops::saved_ids(&self.state)
            .into_iter()
            .filter(|id| !before.contains(id))
            .fold(self.state.pipelines.clone(), |store, id| {
                store.with_connection_copied(source, id)
            });
        self.state.apply_pipelines_cleanup(next);
    }
}
