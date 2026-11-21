//! The query history and favourites: `query_history.json` beside `connections.json`, with the
//! same atomic 0600 write and the same rule for a file that cannot be used as the other settings
//! files: back it up, start empty, and refuse to save when the backup failed so the original is
//! never overwritten (#72).
//!
//! The file holds the text of the queries the person typed, which can contain personal values,
//! so nothing here logs a query, a name or a namespace: only error kinds.
use super::connections::ConfigPath;
use super::persistence::{
    BackupFn, PersistError, PersistGuard, Recovery, atomic_write, default_connections_path,
};
use super::pipelines_store::DiskStamp;
use super::settings_file;
use curlybrace_core::query_history::{QueryHistory, VERSION};
use std::path::Path;

const FILE_NAME: &str = "query_history.json";

/// A file larger than this is not trusted: it is treated like a corrupt one (5 MiB).
pub const MAX_FILE_BYTES: usize = 5 * 1024 * 1024;

/// `<config dir>/curlybrace/query_history.json`, beside the connection list.
pub fn default_query_history_path() -> ConfigPath {
    default_connections_path().map(|path| path.with_file_name(FILE_NAME))
}

/// What startup found, and the stamp of the very bytes it was parsed from.
#[derive(Debug, PartialEq)]
pub struct HistoryStartup {
    pub settings: QueryHistory,
    pub guard: PersistGuard,
    pub recovery: Option<Recovery>,
    pub seen: DiskStamp,
}

/// Whether the file on disk is, right now, one written by a newer version, whatever its size
/// (as `load` and `saved` decide it). Read in full, so it is asked only when it matters.
pub fn file_is_newer(path: &Path) -> bool {
    #[cfg(test)]
    NEWER_CHECKS.with(|n| n.set(n.get() + 1));
    std::fs::read(path).is_ok_and(|bytes| is_newer(&bytes))
}

#[cfg(test)]
thread_local! {
    /// How many times `file_is_newer` was asked, for the tests.
    pub static NEWER_CHECKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Reads the file once (see `settings_file::load` for the rules).
pub fn load(path: &Path, backup: BackupFn<'_>) -> HistoryStartup {
    let read = std::fs::read(path);
    let seen = read
        .as_ref()
        .map_or_else(|_| DiskStamp::default(), |bytes| DiskStamp::of(bytes));
    if read.as_ref().is_ok_and(|bytes| is_newer(bytes)) {
        // Not a guard: the file is checked again at every save, so the refusal ends with the
        // file. It is stamped as if there were none: a removed file can be written to, any
        // other file is reloaded first.
        return HistoryStartup {
            settings: QueryHistory::default(),
            guard: PersistGuard::default(),
            recovery: Some(Recovery::QueryHistoryNewer),
            seen: DiskStamp::default(),
        };
    }
    let started = settings_file::load_read(path, read, backup, "query history", parse, |b| {
        Recovery::QueryHistory { backed_up: b }
    });
    HistoryStartup {
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
        .is_some_and(|version| version > u64::from(VERSION))
}

/// A JSON object of the expected shape and size, of a version this build understands.
fn parse(bytes: &[u8]) -> Result<QueryHistory, PersistError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(PersistError::Corrupt);
    }
    serde_json::from_value(settings_file::json_object(bytes)?).map_err(|_| PersistError::Corrupt)
}

fn serialised(store: &QueryHistory) -> Result<Vec<u8>, PersistError> {
    serde_json::to_vec_pretty(store)
        .map_err(|_| PersistError::Write(std::io::ErrorKind::InvalidData))
}

/// Atomically writes the store (pretty JSON, 0600).
pub fn save(path: &Path, store: &QueryHistory) -> Result<(), PersistError> {
    atomic_write(path, &serialised(store)?)
}

/// Why a change was not written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistorySaveError {
    /// The unusable original could not be backed up, so it must not be replaced.
    Blocked,
    /// The favourites alone would pass [`MAX_FILE_BYTES`], which the next launch would not read.
    TooLarge,
    /// The file was written by a newer version of the app and is left as it is.
    NewerVersion,
    /// Another window changed the file since this one read or wrote it.
    ChangedOnDisk,
    /// A change was made again on top of what another window wrote and was refused there (a
    /// name that window took, a full list): the person saw it succeed, so this says it did not.
    ReplayDropped,
    /// A duplicated connection got none of the favourites of its source: they would not fit in
    /// the file together with the others.
    CopyRefused,
    /// The switch is off but some backup copies of the file (they can hold typed queries) could
    /// not be deleted.
    BackupsNotDeleted,
    Persist(PersistError),
}

impl HistorySaveError {
    /// The message for a refused write. Never names a query.
    pub fn i18n_key(&self) -> &'static str {
        match self {
            Self::Blocked => "persist.query_history_blocked",
            Self::TooLarge => "persist.query_history_too_large",
            Self::NewerVersion => "persist.query_history_newer",
            Self::ChangedOnDisk => "persist.query_history_changed",
            Self::ReplayDropped => "persist.query_history_replay_dropped",
            Self::CopyRefused => "persist.query_history_copy_refused",
            Self::BackupsNotDeleted => "persist.query_history_backups_not_deleted",
            Self::Persist(PersistError::ConfigDirUnknown) => "persist.query_history_no_folder",
            Self::Persist(_) => "persist.query_history_write_failed",
        }
    }
}

/// What a successful write left behind: the store as written (history may have been trimmed to
/// fit) and the stamp of its bytes.
#[derive(Debug, PartialEq)]
pub struct Written {
    pub store: QueryHistory,
    pub stamp: DiskStamp,
}

/// The store with the oldest history dropped until it serialises under [`MAX_FILE_BYTES`], and
/// its bytes. A store that fits is measured once (the bytes are the ones `saved` writes). One
/// that does not has the excess dropped in one pass by the size each entry takes, then is
/// measured again: the estimate can fall a little short, so this repeats, but a few rounds at
/// most, never a copy and a measurement per entry. Favourites are never dropped: when they alone
/// do not fit, `TooLarge`.
fn fitted(store: &QueryHistory) -> Result<(QueryHistory, Vec<u8>), HistorySaveError> {
    let mut bytes = serialised(store).map_err(HistorySaveError::Persist)?;
    let mut current = store.clone();
    while bytes.len() > MAX_FILE_BYTES {
        let excess = bytes.len() - MAX_FILE_BYTES;
        current = current
            .oldest_history_dropped_by(excess)
            .ok_or(HistorySaveError::TooLarge)?;
        bytes = serialised(&current).map_err(HistorySaveError::Persist)?;
    }
    Ok((current, bytes))
}

/// The file's bytes, or nothing when there is no file.
fn read_current(path: &Path) -> Result<Option<Vec<u8>>, PersistError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(PersistError::Read(e.kind())),
    }
}

/// [`save`] unless the guard says the file must not be replaced, a newer file is there, or the
/// file is no longer what `seen` says. Never fails for size alone: history is trimmed, oldest
/// first, until the file fits.
pub fn saved(
    path: &Path,
    guard: &PersistGuard,
    seen: DiskStamp,
    store: &QueryHistory,
) -> Result<Written, HistorySaveError> {
    if guard.check().is_err() {
        let newer = std::fs::read(path).is_ok_and(|bytes| is_newer(&bytes));
        return Err(match newer {
            true => HistorySaveError::NewerVersion,
            false => HistorySaveError::Blocked,
        });
    }
    let (store, bytes) = fitted(store)?;
    let current = read_current(path).map_err(HistorySaveError::Persist)?;
    if current.as_deref().is_some_and(is_newer) {
        return Err(HistorySaveError::NewerVersion);
    }
    if current
        .as_deref()
        .map_or(DiskStamp::default(), DiskStamp::of)
        != seen
    {
        return Err(HistorySaveError::ChangedOnDisk);
    }
    atomic_write(path, &bytes).map_err(HistorySaveError::Persist)?;
    Ok(Written {
        store,
        stamp: DiskStamp::of(&bytes),
    })
}

/// What a window finds on disk after another window changed the file.
#[derive(Debug, PartialEq)]
pub enum Reload {
    /// The store as it is now, and the stamp of that file.
    Usable(QueryHistory, DiskStamp),
    /// A file that cannot be used (corrupt, too large or unreadable): its stamp, and the tag
    /// for its backup.
    Unusable { seen: DiskStamp, tag: &'static str },
}

/// Reads the store another window left on disk.
pub fn reloaded(path: &Path) -> Reload {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Reload::Usable(QueryHistory::default(), DiskStamp::default());
        }
        Err(_) => {
            return Reload::Unusable {
                seen: DiskStamp::default(),
                tag: "unreadable",
            };
        }
    };
    match parse(&bytes) {
        Ok(store) => Reload::Usable(store, DiskStamp::of(&bytes)),
        Err(_) => Reload::Unusable {
            seen: DiskStamp::of(&bytes),
            tag: "corrupt",
        },
    }
}

/// Deletes the backup copies of the file (they can hold the text of queries). Temp files are not
/// touched: one may be another window's write in progress, and `atomic_write` sweeps the ones a
/// crash left behind once they are ten minutes old. Every copy is tried; the first failure is
/// reported.
pub fn remove_backups(path: &Path) -> Result<(), PersistError> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(PersistError::InvalidPath);
    };
    let name = name.to_string_lossy();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(PersistError::Write(e.kind())),
    };
    let mut failed = None;
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        let is_backup = ["corrupt", "unreadable"]
            .iter()
            .any(|tag| file.starts_with(&format!("{name}.{tag}-")));
        if is_backup {
            if let Err(e) = std::fs::remove_file(entry.path()) {
                failed.get_or_insert(e.kind());
            }
        }
    }
    failed.map_or(Ok(()), |kind| Err(PersistError::Write(kind)))
}

#[cfg(test)]
#[path = "query_history_store_tests.rs"]
mod tests;
