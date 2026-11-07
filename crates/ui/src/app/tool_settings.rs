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
use super::state::Banner;
use curlybrace_core::tools::ToolSettings;
use std::path::Path;

const FILE_NAME: &str = "tools.json";

/// `<config dir>/curlybrace/tools.json`, beside the connection list.
pub fn default_tools_path() -> ConfigPath {
    default_connections_path().map(|path| path.with_file_name(FILE_NAME))
}

/// The settings found at startup, whether saving them again is allowed and what, if anything,
/// was wrong with the file.
#[derive(Debug, PartialEq)]
pub struct ToolsStartup {
    pub settings: ToolSettings,
    pub guard: PersistGuard,
    pub recovery: Option<Recovery>,
}

/// Reads the file. A missing file is the defaults. A file that cannot be read or is not the
/// expected shape is backed up first (tag `corrupt` or `unreadable`, like the connection list);
/// if that fails the guard refuses every later save.
pub fn load(path: &Path, backup: BackupFn<'_>) -> ToolsStartup {
    let tag = match std::fs::read(path) {
        Ok(bytes) => match parse(&bytes) {
            Ok(settings) => return usable(settings),
            Err(_) => "corrupt",
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return usable(ToolSettings::default());
        }
        Err(_) => "unreadable",
    };
    let backed_up = match backup(path, tag) {
        Ok(_) => true,
        Err(e) => {
            log::warn!("could not back up the tool paths ({tag}): {e}");
            false
        }
    };
    ToolsStartup {
        settings: ToolSettings::default(),
        guard: if backed_up {
            PersistGuard::default()
        } else {
            PersistGuard::blocked()
        },
        recovery: Some(Recovery::Tools { backed_up }),
    }
}

fn usable(settings: ToolSettings) -> ToolsStartup {
    ToolsStartup {
        settings,
        guard: PersistGuard::default(),
        recovery: None,
    }
}

/// A JSON object of the expected shape. Anything else (an array, a string, a wrong field type)
/// is corrupt: serde alone would accept an array as a struct.
fn parse(bytes: &[u8]) -> Result<ToolSettings, PersistError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| PersistError::Corrupt)?;
    if !value.is_object() {
        return Err(PersistError::Corrupt);
    }
    serde_json::from_value(value).map_err(|_| PersistError::Corrupt)
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
