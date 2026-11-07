//! The tool paths: which `mongosh`, `mongodump` and `mongorestore` to run, globally and per
//! connection. Saved as `tools.json` beside `connections.json`, with the same atomic 0600 write
//! and the same rule for a file that cannot be used: back it up, fall back to the defaults, and
//! refuse to save when the backup failed so the original is never overwritten.
//!
//! The file holds executable paths and connection ids only. It is kept apart from
//! `connections.json` so that exporting connections never carries a path from this machine.
use super::connections::ConfigPath;
use super::persistence::{
    BackupFn, PersistError, PersistGuard, Recovery, atomic_write, default_connections_path,
};
use super::settings_file::{self, Startup};
use super::state::Banner;
use curlybrace_core::tools::ToolSettings;
use std::path::Path;

const FILE_NAME: &str = "tools.json";

/// `<config dir>/curlybrace/tools.json`, beside the connection list.
pub fn default_tools_path() -> ConfigPath {
    default_connections_path().map(|path| path.with_file_name(FILE_NAME))
}

/// The tool paths found at startup.
pub type ToolsStartup = Startup<ToolSettings>;

/// Reads the file (see `settings_file::load` for the rules).
pub fn load(path: &Path, backup: BackupFn<'_>) -> ToolsStartup {
    settings_file::load(path, backup, "tool paths", parse, |backed_up| {
        Recovery::Tools { backed_up }
    })
}

/// A JSON object of the expected shape.
fn parse(bytes: &[u8]) -> Result<ToolSettings, PersistError> {
    serde_json::from_value(settings_file::json_object(bytes)?).map_err(|_| PersistError::Corrupt)
}

/// Atomically writes the settings (pretty JSON, 0600).
pub fn save(path: &Path, settings: &ToolSettings) -> Result<(), PersistError> {
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|_| PersistError::Write(std::io::ErrorKind::InvalidData))?;
    atomic_write(path, &bytes)
}

/// Why a change to the tool paths was not written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolsSaveError {
    /// The unusable original could not be backed up, so it must not be replaced.
    Blocked,
    Persist(PersistError),
}

impl ToolsSaveError {
    /// The banner for the refusal, raised on every attempt. A blocked save reuses the
    /// failed-backup recovery text, which says changes are not saved.
    pub fn banner(self) -> Banner {
        match self {
            Self::Blocked => Banner::Recovery(Recovery::Tools { backed_up: false }),
            Self::Persist(e) => Banner::ToolsPersist(e),
        }
    }
}

/// [`save`] unless the guard says the file must not be replaced.
pub fn saved(
    path: &Path,
    guard: &PersistGuard,
    settings: &ToolSettings,
) -> Result<(), ToolsSaveError> {
    guard.check().map_err(|_| ToolsSaveError::Blocked)?;
    save(path, settings).map_err(ToolsSaveError::Persist)
}
