//! The databases the usage sampler skips on top of its built-in list (`admin`, `local`,
//! `config`, which the sampler applies itself and this file never holds). Saved as
//! `sampling.json` beside `connections.json`, with the same atomic 0600 write and the same rule
//! for a file that cannot be used: back it up, fall back to the defaults, and refuse to save when
//! the backup failed so the original is never overwritten.
use super::connections::ConfigPath;
use super::persistence::{
    BackupFn, PersistError, PersistGuard, Recovery, atomic_write, default_connections_path,
};
use super::reducer::Out;
use super::state::AppState;
use curlybrace_core::runtime::Command;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::Path;

const FILE_NAME: &str = "sampling.json";
const FIELD: &str = "excluded_dbs";

/// The databases to leave out of sampling. Names are kept exactly as typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SamplingSettings {
    pub excluded_dbs: BTreeSet<String>,
}

impl SamplingSettings {
    /// What a typed name becomes: leading and trailing whitespace dropped, nothing else touched
    /// (database names may hold dots, spaces and any Unicode). `None` for an empty name.
    pub fn normalized(input: &str) -> Option<String> {
        let name = input.trim();
        (!name.is_empty()).then(|| name.to_owned())
    }

    /// These settings plus `input`; unchanged when `input` is empty or already excluded.
    pub fn excluding(&self, input: &str) -> Self {
        let Some(name) = Self::normalized(input) else {
            return self.clone();
        };
        let mut excluded_dbs = self.excluded_dbs.clone();
        excluded_dbs.insert(name);
        Self { excluded_dbs }
    }

    /// These settings without the exact name `name`.
    pub fn including(&self, name: &str) -> Self {
        let mut excluded_dbs = self.excluded_dbs.clone();
        excluded_dbs.remove(name);
        Self { excluded_dbs }
    }

    /// The command that hands the whole set to the sampler.
    pub fn command(&self) -> Command {
        Command::SetSamplingExcluded {
            databases: self.excluded_dbs.clone(),
        }
    }
}

/// `<config dir>/curlybrace/sampling.json`, beside the connection list.
pub fn default_sampling_path() -> ConfigPath {
    default_connections_path().map(|path| path.with_file_name(FILE_NAME))
}

/// The settings found at startup, whether saving them again is allowed and what, if anything, was
/// wrong with the file.
#[derive(Debug, PartialEq)]
pub struct SamplingStartup {
    pub settings: SamplingSettings,
    pub guard: PersistGuard,
    pub recovery: Option<Recovery>,
}

/// Reads the file. A missing file is the defaults. A file that cannot be read or is not the
/// expected shape is backed up first (tag `corrupt` or `unreadable`, like the connection list);
/// if that fails the guard refuses every later save.
pub fn load(path: &Path, backup: BackupFn<'_>) -> SamplingStartup {
    let tag = match std::fs::read(path) {
        Ok(bytes) => match parse(&bytes) {
            Ok(settings) => return usable(settings),
            Err(_) => "corrupt",
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return usable(SamplingSettings::default());
        }
        Err(_) => "unreadable",
    };
    let backed_up = match backup(path, tag) {
        Ok(_) => true,
        Err(e) => {
            log::warn!("could not back up the sampling settings ({tag}): {e}");
            false
        }
    };
    SamplingStartup {
        settings: SamplingSettings::default(),
        guard: if backed_up {
            PersistGuard::default()
        } else {
            PersistGuard::blocked()
        },
        recovery: Some(Recovery::Sampling { backed_up }),
    }
}

fn usable(settings: SamplingSettings) -> SamplingStartup {
    SamplingStartup {
        settings,
        guard: PersistGuard::default(),
        recovery: None,
    }
}

/// `{"excluded_dbs": ["name", ...]}`; a missing list is an empty one, anything of another shape
/// is corrupt.
fn parse(bytes: &[u8]) -> Result<SamplingSettings, PersistError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| PersistError::Corrupt)?;
    let object = value.as_object().ok_or(PersistError::Corrupt)?;
    let excluded_dbs = match object.get(FIELD) {
        None => BTreeSet::new(),
        Some(list) => list
            .as_array()
            .ok_or(PersistError::Corrupt)?
            .iter()
            .map(|name| {
                name.as_str()
                    .map(str::to_owned)
                    .ok_or(PersistError::Corrupt)
            })
            .collect::<Result<_, _>>()?,
    };
    Ok(SamplingSettings { excluded_dbs })
}

/// Atomically writes the settings (pretty JSON, 0600).
pub fn save(path: &Path, settings: &SamplingSettings) -> Result<(), PersistError> {
    let bytes = serde_json::to_vec_pretty(&json!({ FIELD: settings.excluded_dbs }))
        .map_err(|_| PersistError::Write(std::io::ErrorKind::InvalidData))?;
    atomic_write(path, &bytes)
}

/// [`save`] unless the guard says the file must not be replaced. A blocked guard skips the write
/// without an error: the startup banner already said that saving is off.
pub fn saved(
    path: &Path,
    guard: &PersistGuard,
    settings: &SamplingSettings,
) -> Result<(), PersistError> {
    if guard.check().is_err() {
        log::warn!("the sampling settings were not saved: the original could not be backed up");
        return Ok(());
    }
    save(path, settings)
}

/// `ExcludeDatabase`: the name joins the set and the sampler is told. Nothing happens for an
/// empty name or one that is already excluded.
pub(super) fn exclude_database(state: AppState, name: &str) -> Out {
    let next = state.sampling.excluding(name);
    replaced(state, next)
}

/// `IncludeDatabase`: the name leaves the set and the sampler is told.
pub(super) fn include_database(state: AppState, name: &str) -> Out {
    let next = state.sampling.including(name);
    replaced(state, next)
}

fn replaced(state: AppState, next: SamplingSettings) -> Out {
    if next == state.sampling {
        return (state, Vec::new());
    }
    let command = next.command();
    (
        AppState {
            sampling: next,
            ..state
        },
        vec![command],
    )
}
