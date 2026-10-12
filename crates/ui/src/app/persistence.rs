//! Connection list persistence: JSON without secrets, atomic 0600 writes.
use mongostudio_core::connection::ConnectionConfig;
use std::path::{Path, PathBuf};

const APP_DIR: &str = "mongostudio";
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

/// What startup found wrong with the saved list and whether the original file was preserved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recovery {
    Corrupt { backed_up: bool },
    Unreadable { backed_up: bool },
    Skipped { count: usize, backed_up: bool },
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
        }) => Startup {
            connections,
            guard: PersistGuard::default(),
            recovery: None,
        },
        Ok(Loaded {
            connections,
            skipped,
        }) => recovered(path, backup, "skipped", connections, move |backed_up| {
            Recovery::Skipped {
                count: skipped,
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
            eprintln!("could not back up the connection list ({tag}): {e}");
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

/// Result of loading: the valid entries and how many were skipped as invalid.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub connections: Vec<ConnectionConfig>,
    pub skipped: usize,
}

/// `<config dir>/mongostudio/connections.json`; no fallback to the working directory.
pub fn default_connections_path() -> Result<PathBuf, PersistError> {
    dirs::config_dir()
        .map(|dir| dir.join(APP_DIR).join(FILE_NAME))
        .ok_or(PersistError::ConfigDirUnknown)
}

/// Reads and validates the list. A missing file is an empty list; entries that fail to
/// deserialize or `validate()` are skipped and counted.
pub fn load_connections(path: &Path) -> Result<Loaded, PersistError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Loaded {
                connections: Vec::new(),
                skipped: 0,
            });
        }
        Err(e) => return Err(PersistError::Read(e.kind())),
    };
    let entries: Vec<serde_json::Value> =
        serde_json::from_slice(&bytes).map_err(|_| PersistError::Corrupt)?;
    let total = entries.len();
    let connections: Vec<ConnectionConfig> = entries
        .into_iter()
        .filter_map(|v| serde_json::from_value::<ConnectionConfig>(v).ok())
        .filter(|c| c.validate().is_ok())
        .collect();
    Ok(Loaded {
        skipped: total - connections.len(),
        connections,
    })
}

/// Atomically writes the list (pretty JSON; the type carries no secrets).
pub fn save_connections(path: &Path, connections: &[ConnectionConfig]) -> Result<(), PersistError> {
    let bytes = serde_json::to_vec_pretty(connections).map_err(|_| PersistError::Corrupt)?;
    atomic_write(path, &bytes)
}

/// Copies the file to a new `<name>.<tag>-<unix seconds>[-n]` sibling (0600) and never
/// overwrites an existing backup. Fails when the file cannot be read.
pub fn back_up(path: &Path, tag: &str) -> std::io::Result<PathBuf> {
    use std::io::Write;
    let bytes = std::fs::read(path)?;
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

/// Writes `bytes` to a 0600 temp file in the same directory, syncs it and renames it over
/// `path`. The temp file is removed on failure.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), PersistError> {
    let write_err = |e: std::io::Error| PersistError::Write(e.kind());
    let (dir, name) = split_path(path)?;
    std::fs::create_dir_all(dir).map_err(write_err)?;
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        name.to_string_lossy(),
        std::process::id()
    ));
    let result = write_temp_then_rename(&tmp, path, bytes);
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(write_err)
}

fn write_temp_then_rename(tmp: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = private_options();
    options.create(true).truncate(true);
    let mut file = options.open(tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongostudio_core::connection::{SshAuth, SshConfig};

    fn cfg(name: &str) -> ConnectionConfig {
        ConnectionConfig::new(name, "mongodb://localhost:27017")
    }

    fn path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("sub").join(FILE_NAME)
    }

    #[test]
    fn round_trips_including_ssh_and_unicode() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = cfg("Prod ☃ Überdatenbank");
        c.ssh = Some(SshConfig {
            host: "bastion".into(),
            port: 2222,
            username: "me".into(),
            auth: SshAuth::Password,
        });
        let list = vec![cfg("a"), c];
        save_connections(&path(&dir), &list).unwrap();
        let loaded = load_connections(&path(&dir)).unwrap();
        assert_eq!(
            loaded,
            Loaded {
                connections: list,
                skipped: 0
            }
        );
    }

    #[test]
    fn missing_file_is_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_connections(&path(&dir)).unwrap();
        assert_eq!((loaded.connections.len(), loaded.skipped), (0, 0));
    }

    #[test]
    fn corrupt_file_is_an_error_and_is_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        std::fs::write(&p, "{ not an array").unwrap();
        assert_eq!(load_connections(&p), Err(PersistError::Corrupt));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{ not an array");
    }

    #[test]
    fn invalid_and_undecodable_entries_are_skipped_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        let good = serde_json::to_value(cfg("ok")).unwrap();
        let blank_name = serde_json::to_value(cfg("  ")).unwrap();
        let bad_uri = serde_json::to_value(ConnectionConfig::new("x", "http://nope")).unwrap();
        let garbage = serde_json::json!({"id": 5});
        let all = serde_json::json!([good, blank_name, bad_uri, garbage]);
        std::fs::write(&p, all.to_string()).unwrap();
        let loaded = load_connections(&p).unwrap();
        assert_eq!(loaded.connections.len(), 1);
        assert_eq!(loaded.connections[0].name, "ok");
        assert_eq!(loaded.skipped, 3);
    }

    #[test]
    fn write_leaves_no_temp_file_and_sets_private_mode() {
        let dir = tempfile::tempdir().unwrap();
        let p = path(&dir);
        save_connections(&p, &[cfg("a")]).unwrap();
        save_connections(&p, &[cfg("b")]).unwrap();
        let names: Vec<_> = std::fs::read_dir(p.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![FILE_NAME.to_string()]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn file_never_contains_a_password_key() {
        let dir = tempfile::tempdir().unwrap();
        let p = path(&dir);
        save_connections(&p, &[cfg("a")]).unwrap();
        let text = std::fs::read_to_string(&p).unwrap().to_lowercase();
        assert!(!text.contains("password") && !text.contains("passphrase"));
    }

    #[test]
    fn failed_write_removes_the_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        std::fs::create_dir(&p).unwrap(); // renaming a file onto a directory fails
        assert!(matches!(
            save_connections(&p, &[cfg("a")]),
            Err(PersistError::Write(_))
        ));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    fn ok_backup(path: &Path, tag: &str) -> std::io::Result<PathBuf> {
        back_up(path, tag)
    }

    fn failing_backup(_: &Path, _: &str) -> std::io::Result<PathBuf> {
        Err(std::io::ErrorKind::PermissionDenied.into())
    }

    fn siblings(dir: &tempfile::TempDir, prefix: &str) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(prefix))
            .collect();
        found.sort();
        found
    }

    #[test]
    fn corrupt_with_a_working_backup_allows_saves_and_keeps_the_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        std::fs::write(&p, "{ corrupt").unwrap();
        let s = startup(&p, &ok_backup);
        assert_eq!(s.recovery, Some(Recovery::Corrupt { backed_up: true }));
        assert!(s.guard.check().is_ok() && s.connections.is_empty());
        let backups = siblings(&dir, "connections.json.corrupt-");
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read_to_string(&backups[0]).unwrap(), "{ corrupt");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{ corrupt");
    }

    #[test]
    fn a_failed_backup_blocks_saving() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        std::fs::write(&p, "{ corrupt").unwrap();
        let s = startup(&p, &failing_backup);
        assert_eq!(s.recovery, Some(Recovery::Corrupt { backed_up: false }));
        assert_eq!(s.guard.check(), Err(PersistError::Blocked));
    }

    #[test]
    fn skipped_entries_are_backed_up_before_any_save_and_the_good_ones_load() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        let good = serde_json::to_value(cfg("ok")).unwrap();
        let original = serde_json::json!([good, {"id": 1, "from": "the future"}]).to_string();
        std::fs::write(&p, &original).unwrap();
        let s = startup(&p, &ok_backup);
        assert_eq!(s.connections.len(), 1);
        assert_eq!(
            s.recovery,
            Some(Recovery::Skipped {
                count: 1,
                backed_up: true
            })
        );
        let backups = siblings(&dir, "connections.json.skipped-");
        assert_eq!(std::fs::read_to_string(&backups[0]).unwrap(), original);
        let blocked = startup(&p, &failing_backup);
        assert_eq!(blocked.guard.check(), Err(PersistError::Blocked));
    }

    #[test]
    fn repeated_corrupt_startups_never_overwrite_earlier_backups() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        for version in ["first", "second", "third"] {
            std::fs::write(&p, version).unwrap();
            assert!(startup(&p, &ok_backup).guard.check().is_ok());
        }
        let backups = siblings(&dir, "connections.json.corrupt-");
        let mut contents: Vec<String> = backups
            .iter()
            .map(|b| std::fs::read_to_string(b).unwrap())
            .collect();
        contents.sort();
        assert_eq!(contents, ["first", "second", "third"]);
    }

    #[test]
    fn an_unreadable_file_that_cannot_be_copied_blocks_saving() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        std::fs::create_dir(&p).unwrap(); // reading a directory fails
        let s = startup(&p, &ok_backup);
        assert_eq!(s.recovery, Some(Recovery::Unreadable { backed_up: false }));
        assert_eq!(s.guard.check(), Err(PersistError::Blocked));
    }

    #[test]
    fn healthy_and_missing_files_need_no_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        assert_eq!(startup(&p, &failing_backup).recovery, None);
        save_connections(&p, &[cfg("a")]).unwrap();
        let s = startup(&p, &failing_backup);
        assert_eq!((s.recovery, s.connections.len()), (None, 1));
        assert!(s.guard.check().is_ok());
    }

    #[test]
    fn backups_are_private() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        std::fs::write(&p, "x").unwrap();
        let b = back_up(&p, "corrupt").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(b).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn a_path_without_a_parent_is_an_invalid_path_not_a_missing_config_dir() {
        assert_eq!(
            atomic_write(Path::new("/"), b"x"),
            Err(PersistError::InvalidPath)
        );
    }

    #[test]
    fn error_keys_are_distinct() {
        let mut keys = vec![
            PersistError::ConfigDirUnknown.i18n_key(),
            PersistError::Read(std::io::ErrorKind::Other).i18n_key(),
            PersistError::Corrupt.i18n_key(),
            PersistError::Write(std::io::ErrorKind::Other).i18n_key(),
            PersistError::InvalidPath.i18n_key(),
            PersistError::Blocked.i18n_key(),
        ];
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), 6);
    }
}
