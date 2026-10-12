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
}

impl PersistError {
    pub fn i18n_key(&self) -> &'static str {
        match self {
            Self::ConfigDirUnknown => "persist.config_dir_unknown",
            Self::Read(_) => "persist.read_failed",
            Self::Corrupt => "persist.corrupt",
            Self::Write(_) => "persist.write_failed",
        }
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

/// Copies an unparseable file aside so a later save cannot destroy it.
/// Best effort; returns the backup path when one was made.
pub fn back_up_corrupt(path: &Path) -> Option<PathBuf> {
    let backup = path.with_extension("json.corrupt");
    std::fs::copy(path, &backup).ok().map(|_| backup)
}

/// Writes `bytes` to a 0600 temp file in the same directory, syncs it and renames it over
/// `path`. The temp file is removed on failure.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), PersistError> {
    let write_err = |e: std::io::Error| PersistError::Write(e.kind());
    let dir = path.parent().ok_or(PersistError::ConfigDirUnknown)?;
    std::fs::create_dir_all(dir).map_err(write_err)?;
    let name = path.file_name().ok_or(PersistError::ConfigDirUnknown)?;
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
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
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

    #[test]
    fn back_up_keeps_the_corrupt_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        std::fs::write(&p, "garbage").unwrap();
        let backup = back_up_corrupt(&p).unwrap();
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "garbage");
    }

    #[test]
    fn error_keys_are_distinct() {
        let mut keys = vec![
            PersistError::ConfigDirUnknown.i18n_key(),
            PersistError::Read(std::io::ErrorKind::Other).i18n_key(),
            PersistError::Corrupt.i18n_key(),
            PersistError::Write(std::io::ErrorKind::Other).i18n_key(),
        ];
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), 4);
    }
}
