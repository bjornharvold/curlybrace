//! Connection list persistence: JSON without secrets, atomic 0600 writes.
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::connection_repair::repair_labels;
use std::path::{Path, PathBuf};

const APP_DIR: &str = "curlybrace";
const FILE_NAME: &str = "connections.json";

/// Why reading or writing app files failed. Detail goes to the log, never the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistError {
    /// No per-user config directory exists; nothing is read or written.
    ConfigDirUnknown,
    Read(std::io::ErrorKind),
    /// The file exists but is not a JSON array.
    Corrupt,
    Write(std::io::ErrorKind),
    /// The target path has no parent directory or file name.
    InvalidPath,
    /// Saving is refused because the existing file could not be backed up.
    Blocked,
}

impl PersistError {
    pub fn i18n_key(&self) -> &'static str {
        match self {
            Self::ConfigDirUnknown => "persist.config_dir_unknown",
            Self::Read(_) => "persist.read_failed",
            Self::Corrupt => "persist.corrupt",
            Self::Write(_) => "persist.write_failed",
            Self::InvalidPath => "persist.invalid_path",
            Self::Blocked => "persist.blocked",
        }
    }
}

impl PersistError {
    /// The message for a failure to save the tool paths (not the connection list).
    pub fn tools_i18n_key(&self) -> &'static str {
        match self {
            Self::ConfigDirUnknown => "persist.tools_config_dir_unknown",
            _ => "persist.tools_write_failed",
        }
    }
}

impl PersistError {
    /// The neutral message for a failure to save the pipelines (not the connection list), for
    /// where the operation is not known (a banner).
    pub fn pipelines_i18n_key(&self) -> &'static str {
        match self {
            Self::ConfigDirUnknown => "aggregate.saved.config_dir_unknown",
            _ => "aggregate.saved.write_failed",
        }
    }
}

/// What startup found wrong with the saved list and whether the original file was preserved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recovery {
    Corrupt {
        backed_up: bool,
    },
    Unreadable {
        backed_up: bool,
    },
    Skipped {
        count: usize,
        backed_up: bool,
    },
    /// Some entries were skipped AND others were adjusted: the report names both facts.
    SkippedAndRepaired {
        skipped: usize,
        repaired: usize,
        backed_up: bool,
    },
    /// Names or folders from an older release were adjusted to the current rules; no entry was
    /// lost.
    Repaired {
        count: usize,
        backed_up: bool,
    },
    /// The sampling settings file was not usable; the defaults apply.
    Sampling {
        backed_up: bool,
    },
    /// The explain setting file was not usable; the default (on) applies.
    Explain {
        backed_up: bool,
    },
    /// The tool paths file was not usable; the defaults apply.
    Tools {
        backed_up: bool,
    },
    /// The saved pipelines file was not usable; the store starts empty.
    Pipelines {
        backed_up: bool,
    },
    /// The saved pipelines file was written by a newer version: it is left as it is, the store
    /// starts empty and saving is turned off.
    PipelinesNewer,
    /// The saved tabs file was not usable; no tabs are reopened.
    Workspace {
        backed_up: bool,
    },
    /// The saved tabs file was written by a newer version: left as it is, saving is turned off.
    WorkspaceNewer,
    /// More saved tabs than the file may hold: the ones past the bound were left out.
    WorkspaceOverflow {
        count: usize,
        backed_up: bool,
    },
    /// Options of some saved tabs were not valid and were dropped from those tabs (they keep
    /// their text); the original file was backed up first.
    WorkspaceRepaired {
        count: usize,
        backed_up: bool,
    },
    /// Older waiting tabs did not fit in the file when open tabs went back to waiting, and were
    /// dropped.
    WorkspaceTrimmed {
        count: usize,
    },
    /// Saved tabs of a connection that connected did not fit under the tab limit: they wait and
    /// reopen as tabs are closed.
    WorkspaceWaiting {
        count: usize,
    },
    /// Saved tabs of a connection or collection that is gone were skipped.
    WorkspaceSkipped {
        count: usize,
        backed_up: bool,
    },
    /// The query history file was not usable; history and favourites start empty.
    QueryHistory {
        backed_up: bool,
    },
    /// The query history file was written by a newer version: left as it is, saving is off.
    QueryHistoryNewer,
}

impl Recovery {
    pub fn i18n_key(&self) -> &'static str {
        match self {
            Self::Corrupt { backed_up: true } => "persist.corrupt_backed_up",
            Self::Corrupt { backed_up: false } => "persist.corrupt_backup_failed",
            Self::Unreadable { backed_up: true } => "persist.unreadable_backed_up",
            Self::Unreadable { backed_up: false } => "persist.unreadable_backup_failed",
            Self::Skipped {
                backed_up: true, ..
            } => "persist.skipped_backed_up",
            Self::Skipped {
                backed_up: false, ..
            } => "persist.skipped_backup_failed",
            Self::SkippedAndRepaired {
                backed_up: true, ..
            } => "persist.skipped_repaired_backed_up",
            Self::SkippedAndRepaired {
                backed_up: false, ..
            } => "persist.skipped_repaired_backup_failed",
            Self::Repaired {
                backed_up: true, ..
            } => "persist.repaired_backed_up",
            Self::Repaired {
                backed_up: false, ..
            } => "persist.repaired_backup_failed",
            Self::Sampling { backed_up: true } => "persist.sampling_backed_up",
            Self::Sampling { backed_up: false } => "persist.sampling_backup_failed",
            Self::Explain { backed_up: true } => "persist.explain_backed_up",
            Self::Explain { backed_up: false } => "persist.explain_backup_failed",
            Self::Tools { backed_up: true } => "persist.tools_backed_up",
            Self::Tools { backed_up: false } => "persist.tools_backup_failed",
            Self::Pipelines { backed_up: true } => "aggregate.saved.recovered_backed_up",
            Self::Pipelines { backed_up: false } => "aggregate.saved.recovered_backup_failed",
            Self::PipelinesNewer => "aggregate.saved.error.unsupported",
            Self::Workspace { backed_up: true } => "persist.workspace_backed_up",
            Self::Workspace { backed_up: false } => "persist.workspace_backup_failed",
            Self::WorkspaceNewer => "persist.workspace_newer",
            Self::WorkspaceOverflow {
                backed_up: true, ..
            } => "persist.workspace_overflow_backed_up",
            Self::WorkspaceOverflow {
                backed_up: false, ..
            } => "persist.workspace_overflow_backup_failed",
            Self::WorkspaceRepaired {
                backed_up: true, ..
            } => "persist.workspace_repaired_backed_up",
            Self::WorkspaceRepaired {
                backed_up: false, ..
            } => "persist.workspace_repaired_backup_failed",
            Self::WorkspaceTrimmed { .. } => "persist.workspace_overflow_trimmed",
            Self::WorkspaceWaiting { .. } => "tabs.restore.overflow",
            Self::WorkspaceSkipped {
                backed_up: true, ..
            } => "persist.workspace_skipped_backed_up",
            Self::WorkspaceSkipped {
                backed_up: false, ..
            } => "persist.workspace_skipped_backup_failed",
            Self::QueryHistory { backed_up: true } => "persist.query_history_backed_up",
            Self::QueryHistory { backed_up: false } => "persist.query_history_backup_failed",
            Self::QueryHistoryNewer => "persist.query_history_newer",
        }
    }
}

/// Whether the connection file may be overwritten. When the original could not be preserved
/// (backup failed) every save is refused so nothing is destroyed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PersistGuard {
    blocked: bool,
}

impl PersistGuard {
    pub fn blocked() -> Self {
        Self { blocked: true }
    }

    pub fn check(&self) -> Result<(), PersistError> {
        if self.blocked {
            Err(PersistError::Blocked)
        } else {
            Ok(())
        }
    }
}

/// Result of reading the saved list at startup.
#[derive(Debug, PartialEq)]
pub struct Startup {
    pub connections: Vec<ConnectionConfig>,
    pub guard: PersistGuard,
    pub recovery: Option<Recovery>,
}

/// Copies the file aside: `(path, tag) -> backup path`. Injectable so failure is testable.
pub type BackupFn<'a> = &'a dyn Fn(&Path, &str) -> std::io::Result<PathBuf>;

/// Loads the list. On ANY problem (corrupt, unreadable, entries skipped) the original file is
/// backed up first; if that fails the guard blocks saving. A healthy or missing file is a
/// plain load.
pub fn startup(path: &Path, backup: BackupFn<'_>) -> Startup {
    match load_connections(path) {
        Ok(Loaded {
            connections,
            skipped: 0,
            repaired: 0,
        }) => Startup {
            connections,
            guard: PersistGuard::default(),
            recovery: None,
        },
        Ok(Loaded {
            connections,
            skipped,
            repaired,
        }) if skipped > 0 && repaired > 0 => {
            recovered(path, backup, "skipped", connections, move |backed_up| {
                Recovery::SkippedAndRepaired {
                    skipped,
                    repaired,
                    backed_up,
                }
            })
        }
        Ok(Loaded {
            connections,
            skipped,
            ..
        }) if skipped > 0 => recovered(path, backup, "skipped", connections, move |backed_up| {
            Recovery::Skipped {
                count: skipped,
                backed_up,
            }
        }),
        Ok(Loaded {
            connections,
            repaired,
            ..
        }) => recovered(path, backup, "repaired", connections, move |backed_up| {
            Recovery::Repaired {
                count: repaired,
                backed_up,
            }
        }),
        Err(PersistError::Corrupt) => recovered(path, backup, "corrupt", Vec::new(), |backed_up| {
            Recovery::Corrupt { backed_up }
        }),
        Err(_) => recovered(path, backup, "unreadable", Vec::new(), |backed_up| {
            Recovery::Unreadable { backed_up }
        }),
    }
}

fn recovered(
    path: &Path,
    backup: BackupFn<'_>,
    tag: &str,
    connections: Vec<ConnectionConfig>,
    recovery: impl FnOnce(bool) -> Recovery,
) -> Startup {
    let backed_up = match backup(path, tag) {
        Ok(_) => true,
        Err(e) => {
            log::warn!("could not back up the connection list ({tag}): {e}");
            false
        }
    };
    Startup {
        connections,
        guard: if backed_up {
            PersistGuard::default()
        } else {
            PersistGuard::blocked()
        },
        recovery: Some(recovery(backed_up)),
    }
}

/// Result of loading: the valid entries, how many were skipped as invalid and how many had
/// their name or folder repaired or a damaged production flag read as production (those are in
/// `connections`).
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub connections: Vec<ConnectionConfig>,
    pub skipped: usize,
    pub repaired: usize,
}

/// `<config dir>/curlybrace/connections.json`; no fallback to the working directory.
pub fn default_connections_path() -> Result<PathBuf, PersistError> {
    dirs::config_dir()
        .map(|dir| dir.join(APP_DIR).join(FILE_NAME))
        .ok_or(PersistError::ConfigDirUnknown)
}

/// Reads the saved list. A missing file is an empty list. Labels saved by an older release are
/// repaired (see `repair_labels`) before validating; entries that fail to deserialize or still
/// fail `validate()` are skipped and counted.
pub fn load_connections(path: &Path) -> Result<Loaded, PersistError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Loaded {
                connections: Vec::new(),
                skipped: 0,
                repaired: 0,
            });
        }
        Err(e) => return Err(PersistError::Read(e.kind())),
    };
    parse_entries(read_entries(&bytes)?, true)
}

/// The JSON array's entries, each still undecoded. The only place the text is parsed.
pub(crate) fn read_entries(bytes: &[u8]) -> Result<Vec<serde_json::Value>, PersistError> {
    serde_json::from_slice(bytes).map_err(|_| PersistError::Corrupt)
}

/// Decodes and checks the entries. With `repair` (the saved list) only an entry that fails
/// `validate()` gets its labels repaired; one that already validates is kept exactly as it is,
/// so a later launch finds nothing new to adjust. Without it (an import) nothing is repaired.
pub(crate) fn parse_entries(
    entries: Vec<serde_json::Value>,
    repair: bool,
) -> Result<Loaded, PersistError> {
    let total = entries.len();
    let mut repaired = 0;
    let mut connections = Vec::with_capacity(total);
    for value in entries {
        // A damaged production flag loads as production (fail safe) and is rewritten as `true`
        // on the next save, so the saved list reports it like any other adjusted entry.
        // The read-only flag is treated the same way (it loads as read-only).
        let flag_damaged = repair
            && ["production", "read_only"]
                .iter()
                .any(|key| value.get(key).is_some_and(|v| !v.is_boolean()));
        let Ok(cfg) = serde_json::from_value::<ConnectionConfig>(value) else {
            continue;
        };
        let (cfg, labels_fixed) = if repair && cfg.validate().is_err() {
            let (fixed, changed) = repair_labels(cfg);
            let usable = changed && fixed.validate().is_ok();
            (fixed, usable)
        } else {
            (cfg, false)
        };
        // The strict host check applies to new input only: a saved connection is never dropped
        // by it (`repair` is true for the saved list).
        let hosts_ok = repair || curlybrace_core::uri::check_hosts(&cfg.uri).is_ok();
        if hosts_ok && cfg.validate().is_ok() {
            repaired += usize::from(labels_fixed || flag_damaged);
            connections.push(cfg);
        }
    }
    Ok(Loaded {
        skipped: total - connections.len(),
        connections,
        repaired,
    })
}

/// Atomically writes the list (pretty JSON; the type carries no secrets).
pub fn save_connections(path: &Path, connections: &[ConnectionConfig]) -> Result<(), PersistError> {
    let bytes = serde_json::to_vec_pretty(connections)
        .map_err(|_| PersistError::Write(std::io::ErrorKind::InvalidData))?;
    atomic_write(path, &bytes)
}

/// Copies the file to a new `<name>.<tag>-<unix seconds>[-n]` sibling (0600) and never
/// overwrites an existing backup. Fails when the file cannot be read.
pub fn back_up(path: &Path, tag: &str) -> std::io::Result<PathBuf> {
    back_up_limited(path, tag, None)
}

/// [`back_up`] reading at most `limit` bytes of the file (a file too big to be one of ours is
/// kept cut at the limit rather than read whole).
pub fn back_up_limited(path: &Path, tag: &str, limit: Option<u64>) -> std::io::Result<PathBuf> {
    use std::io::{Read, Write};
    let mut bytes = Vec::new();
    let file = std::fs::File::open(path)?;
    match limit {
        Some(limit) => file.take(limit).read_to_end(&mut bytes)?,
        None => {
            let mut file = file;
            file.read_to_end(&mut bytes)?
        }
    };
    let (dir, name) = split_path(path)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid path"))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    for n in 0..1000 {
        let suffix = if n == 0 {
            format!("{tag}-{stamp}")
        } else {
            format!("{tag}-{stamp}-{n}")
        };
        let dest = dir.join(format!("{}.{suffix}", name.to_string_lossy()));
        let mut options = private_options();
        options.create_new(true);
        match options.open(&dest) {
            Ok(mut file) => {
                file.write_all(&bytes)?;
                file.sync_all()?;
                return Ok(dest);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::ErrorKind::AlreadyExists.into())
}

fn split_path(path: &Path) -> Result<(&Path, &std::ffi::OsStr), PersistError> {
    let dir = path.parent().ok_or(PersistError::InvalidPath)?;
    let name = path.file_name().ok_or(PersistError::InvalidPath)?;
    Ok((dir, name))
}

fn private_options() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

/// Writes `bytes` to a new 0600 temp file in the same directory, syncs it and renames it over
/// `path`. The temp file is created exclusively under a name nobody else can have picked, so a
/// planted file or link at that name is never opened or followed; it is removed on failure.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), PersistError> {
    let write_err = |e: std::io::Error| PersistError::Write(e.kind());
    let (dir, name) = split_path(path)?;
    std::fs::create_dir_all(dir).map_err(write_err)?;
    sweep_orphans(dir, &name.to_string_lossy());
    let (tmp, file) = create_temp(dir, &name.to_string_lossy()).map_err(write_err)?;
    let result = write_temp_then_rename(file, &tmp, path, bytes);
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(write_err)
}

/// How old a temp file of a write must be (ten minutes) before it is taken for one a crash left
/// behind.
const ORPHAN_AGE: std::time::Duration = std::time::Duration::from_secs(600);

/// Deletes the temp files (`.<name>.tmp-*`) that earlier writes of `name` left behind. Only ones
/// not touched for ten minutes: a write in progress, ours or another window's, is never in the way,
/// and the file being written now does not exist yet.
fn sweep_orphans(dir: &Path, name: &str) {
    let prefix = format!(".{name}.tmp-");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with(&prefix) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= ORPHAN_AGE);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// A temp file that did not exist, created with mode 0600 from the start.
fn create_temp(dir: &Path, name: &str) -> std::io::Result<(PathBuf, std::fs::File)> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    for _ in 0..100 {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!(".{name}.tmp-{}-{nanos}-{n}", std::process::id()));
        let mut options = private_options();
        options.create_new(true);
        match options.open(&tmp) {
            Ok(file) => return Ok((tmp, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::ErrorKind::AlreadyExists.into())
}

fn write_temp_then_rename(
    mut file: std::fs::File,
    tmp: &Path,
    path: &Path,
    bytes: &[u8],
) -> std::io::Result<()> {
    use std::io::Write;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
