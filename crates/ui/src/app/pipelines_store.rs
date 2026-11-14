//! The saved aggregation pipelines: `pipelines.json` beside `connections.json`, with the same
//! atomic 0600 write and the same rule for a file that cannot be used: back it up, start with
//! an empty store, and refuse to save when the backup failed so the original is never
//! overwritten.
//!
//! Pipelines can hold values from the user's data, so nothing here logs a name or a stage.
use super::connections::ConfigPath;
use super::persistence::{
    BackupFn, PersistError, PersistGuard, Recovery, atomic_write, default_connections_path,
};
use super::settings_file::{self, Startup};
use super::state::Banner;
use curlybrace_core::saved_pipeline::SavedPipelines;
use std::path::Path;

const FILE_NAME: &str = "pipelines.json";

/// A file larger than this is not trusted: it is treated like a corrupt one (5 MB).
pub const MAX_FILE_BYTES: usize = 5 * 1024 * 1024;

/// `<config dir>/curlybrace/pipelines.json`, beside the connection list.
pub fn default_pipelines_path() -> ConfigPath {
    default_connections_path().map(|path| path.with_file_name(FILE_NAME))
}

/// The saved pipelines found at startup.
pub type PipelinesStartup = Startup<SavedPipelines>;

/// Reads the file (see `settings_file::load` for the rules).
pub fn load(path: &Path, backup: BackupFn<'_>) -> PipelinesStartup {
    settings_file::load(path, backup, "saved pipelines", parse, |backed_up| {
        Recovery::Pipelines { backed_up }
    })
}

/// A JSON object of the expected shape and size, of a version this build understands.
fn parse(bytes: &[u8]) -> Result<SavedPipelines, PersistError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(PersistError::Corrupt);
    }
    serde_json::from_value(settings_file::json_object(bytes)?).map_err(|_| PersistError::Corrupt)
}

/// Atomically writes the pipelines (pretty JSON, 0600).
pub fn save(path: &Path, pipelines: &SavedPipelines) -> Result<(), PersistError> {
    let bytes = serde_json::to_vec_pretty(pipelines)
        .map_err(|_| PersistError::Write(std::io::ErrorKind::InvalidData))?;
    atomic_write(path, &bytes)
}

/// Why a change to the saved pipelines was not written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelinesSaveError {
    /// The unusable original could not be backed up, so it must not be replaced.
    Blocked,
    Persist(PersistError),
}

impl PipelinesSaveError {
    /// The message for the refusal.
    pub fn i18n_key(&self) -> &'static str {
        match self {
            Self::Blocked => "aggregate.saved.blocked",
            Self::Persist(e) => e.pipelines_i18n_key(),
        }
    }

    /// The banner for a refusal that nothing else shows (a connection cleanup, say). A blocked
    /// save reuses the failed-backup recovery text.
    pub fn banner(self) -> Banner {
        match self {
            Self::Blocked => Banner::Recovery(Recovery::Pipelines { backed_up: false }),
            Self::Persist(e) => Banner::PipelinesPersist(e),
        }
    }
}

/// [`save`] unless the guard says the file must not be replaced.
pub fn saved(
    path: &Path,
    guard: &PersistGuard,
    pipelines: &SavedPipelines,
) -> Result<(), PipelinesSaveError> {
    guard.check().map_err(|_| PipelinesSaveError::Blocked)?;
    save(path, pipelines).map_err(PipelinesSaveError::Persist)
}
