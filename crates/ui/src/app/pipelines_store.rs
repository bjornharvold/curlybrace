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
use super::settings_file;
use super::state::Banner;
use curlybrace_core::saved_pipeline::SavedPipelines;
use std::hash::{Hash, Hasher};
use std::path::Path;

const FILE_NAME: &str = "pipelines.json";

/// A file larger than this is not trusted: it is treated like a corrupt one (5 MB).
pub const MAX_FILE_BYTES: usize = 5 * 1024 * 1024;

/// `<config dir>/curlybrace/pipelines.json`, beside the connection list.
pub fn default_pipelines_path() -> ConfigPath {
    default_connections_path().map(|path| path.with_file_name(FILE_NAME))
}

/// The saved pipelines found at startup, and the stamp of the very bytes they were parsed from
/// (a second read could see another window's write and pair its stamp with the old list).
#[derive(Debug, PartialEq)]
pub struct PipelinesStartup {
    pub settings: SavedPipelines,
    pub guard: PersistGuard,
    pub recovery: Option<Recovery>,
    pub seen: DiskStamp,
}

/// Reads the file once (see `settings_file::load` for the rules).
pub fn load(path: &Path, backup: BackupFn<'_>) -> PipelinesStartup {
    let read = std::fs::read(path);
    let seen = read
        .as_ref()
        .map_or_else(|_| DiskStamp::default(), |bytes| DiskStamp::of(bytes));
    if read.as_ref().is_ok_and(|bytes| is_newer(bytes)) {
        // Not a guard: the file is checked again at every save, so the refusal ends with the
        // file. The list is empty, so it is stamped as if there were no file: a file that
        // is removed can be written to, any other file is reloaded first.
        return PipelinesStartup {
            settings: SavedPipelines::default(),
            guard: PersistGuard::default(),
            recovery: Some(Recovery::PipelinesNewer),
            seen: DiskStamp::default(),
        };
    }
    let started = settings_file::load_read(path, read, backup, "saved pipelines", parse, |b| {
        Recovery::Pipelines { backed_up: b }
    });
    PipelinesStartup {
        settings: started.settings,
        guard: started.guard,
        recovery: started.recovery,
        seen,
    }
}

/// Whether the bytes are a JSON object written by a newer version of the format: such a file is
/// not ours to back up or replace.
fn is_newer(bytes: &[u8]) -> bool {
    settings_file::json_object(bytes)
        .ok()
        .and_then(|value| value.get("version").and_then(serde_json::Value::as_u64))
        .is_some_and(|version| version > u64::from(curlybrace_core::saved_pipeline::VERSION))
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
    atomic_write(path, &serialised(pipelines)?)
}

fn serialised(pipelines: &SavedPipelines) -> Result<Vec<u8>, PersistError> {
    serde_json::to_vec_pretty(pipelines)
        .map_err(|_| PersistError::Write(std::io::ErrorKind::InvalidData))
}

/// Why a change to the saved pipelines was not written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelinesSaveError {
    /// The unusable original could not be backed up, so it must not be replaced.
    Blocked,
    /// The file would pass [`MAX_FILE_BYTES`], which the next launch would not read back.
    TooLarge,
    /// The file was written by a newer version of the app and is left as it is.
    NewerVersion,
    /// Another window changed the file since this one read or wrote it.
    ChangedOnDisk,
    Persist(PersistError),
}

impl PipelinesSaveError {
    /// The message for the refusal.
    pub fn i18n_key(&self) -> &'static str {
        match self {
            Self::Blocked => "aggregate.saved.blocked",
            Self::TooLarge => "aggregate.saved.error.file_too_large",
            Self::NewerVersion => "aggregate.saved.error.unsupported",
            Self::ChangedOnDisk => "aggregate.saved.error.changed_on_disk",
            Self::Persist(e) => e.pipelines_i18n_key(),
        }
    }

    /// The banner for a refusal that nothing else shows (a connection cleanup, say). A blocked
    /// save reuses the failed-backup recovery text.
    pub fn banner(self) -> Banner {
        match self {
            Self::Blocked => Banner::Recovery(Recovery::Pipelines { backed_up: false }),
            Self::NewerVersion => Banner::Recovery(Recovery::PipelinesNewer),
            other => Banner::PipelinesPersist(other),
        }
    }
}

/// What this window last read from or wrote to the file: its length and a hash. A save that
/// finds something else on disk was preceded by a write from another window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiskStamp(Option<(usize, u64)>);

impl DiskStamp {
    fn of(bytes: &[u8]) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hasher);
        Self(Some((bytes.len(), hasher.finish())))
    }

    /// The stamp of a file's bytes; a missing file has none.
    fn of_file(bytes: Option<&[u8]>) -> Self {
        bytes.map_or(Self(None), Self::of)
    }

    /// The stamp of the file as it is now.
    pub fn read(path: &Path) -> Result<Self, PersistError> {
        read_current(path).map(|bytes| Self::of_file(bytes.as_deref()))
    }
}

/// The file's bytes, or nothing when there is no file.
fn read_current(path: &Path) -> Result<Option<Vec<u8>>, PersistError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(PersistError::Read(e.kind())),
    }
}

/// The store as another window left it on disk, and the stamp of that file; `None` when it
/// cannot be read or is not usable (this window then keeps what it has).
pub fn reloaded(path: &Path) -> Option<(SavedPipelines, DiskStamp)> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Some((SavedPipelines::default(), DiskStamp::default()));
        }
        Err(_) => return None,
    };
    let store = parse(&bytes).ok()?;
    Some((store, DiskStamp::of(&bytes)))
}

/// [`save`] unless the guard says the file must not be replaced, the result would be too big
/// to read back, or the file is no longer what `seen` says. Gives the stamp of what it wrote.
pub fn saved(
    path: &Path,
    guard: &PersistGuard,
    seen: DiskStamp,
    pipelines: &SavedPipelines,
) -> Result<DiskStamp, PipelinesSaveError> {
    if guard.check().is_err() {
        let newer = std::fs::read(path).is_ok_and(|bytes| is_newer(&bytes));
        return Err(if newer {
            PipelinesSaveError::NewerVersion
        } else {
            PipelinesSaveError::Blocked
        });
    }
    let bytes = serialised(pipelines).map_err(PipelinesSaveError::Persist)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(PipelinesSaveError::TooLarge);
    }
    let current = read_current(path).map_err(PipelinesSaveError::Persist)?;
    if current.as_deref().is_some_and(is_newer) {
        return Err(PipelinesSaveError::NewerVersion);
    }
    if DiskStamp::of_file(current.as_deref()) != seen {
        return Err(PipelinesSaveError::ChangedOnDisk);
    }
    atomic_write(path, &bytes).map_err(PipelinesSaveError::Persist)?;
    Ok(DiskStamp::of(&bytes))
}
