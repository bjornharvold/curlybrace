//! The rules the small settings files share (`sampling.json`, `tools.json`): a missing file is
//! the defaults; a file that cannot be read or is not the expected shape is backed up first and
//! the defaults apply; if that backup fails, every later save is refused so the original is never
//! overwritten. Each file brings its own parser and its own recovery notice.
use super::persistence::{BackupFn, PersistError, PersistGuard, Recovery};
use std::path::Path;

/// What was found at startup: the settings, whether saving them again is allowed and what, if
/// anything, was wrong with the file.
#[derive(Debug, PartialEq)]
pub struct Startup<T> {
    pub settings: T,
    pub guard: PersistGuard,
    pub recovery: Option<Recovery>,
}

impl<T> Startup<T> {
    fn usable(settings: T) -> Self {
        Self {
            settings,
            guard: PersistGuard::default(),
            recovery: None,
        }
    }
}

/// Reads the file at `path` with `parse`. `what` names the file in the log, `recovery` builds the
/// notice from whether the backup worked. Backups use the tags `corrupt` and `unreadable`, like
/// the connection list.
pub(super) fn load<T: Default>(
    path: &Path,
    backup: BackupFn<'_>,
    what: &str,
    parse: impl Fn(&[u8]) -> Result<T, PersistError>,
    recovery: impl Fn(bool) -> Recovery,
) -> Startup<T> {
    load_read(path, std::fs::read(path), backup, what, parse, recovery)
}

/// [`load`] for a caller that already read the file, so the bytes it parses are the bytes it
/// knows about.
pub(super) fn load_read<T: Default>(
    path: &Path,
    read: std::io::Result<Vec<u8>>,
    backup: BackupFn<'_>,
    what: &str,
    parse: impl Fn(&[u8]) -> Result<T, PersistError>,
    recovery: impl Fn(bool) -> Recovery,
) -> Startup<T> {
    let tag = match read {
        Ok(bytes) => match parse(&bytes) {
            Ok(settings) => return Startup::usable(settings),
            Err(_) => "corrupt",
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Startup::usable(T::default());
        }
        Err(_) => "unreadable",
    };
    let backed_up = match backup(path, tag) {
        Ok(_) => true,
        Err(e) => {
            log::warn!("could not back up the {what} ({tag}): {e}");
            false
        }
    };
    Startup {
        settings: T::default(),
        guard: if backed_up {
            PersistGuard::default()
        } else {
            PersistGuard::blocked()
        },
        recovery: Some(recovery(backed_up)),
    }
}

/// The bytes as a JSON object. Anything else (an array, a string, text that is not JSON) is
/// corrupt: serde alone would accept an array as a struct.
pub(super) fn json_object(bytes: &[u8]) -> Result<serde_json::Value, PersistError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| PersistError::Corrupt)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(PersistError::Corrupt)
    }
}
