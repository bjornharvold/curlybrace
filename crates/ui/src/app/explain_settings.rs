//! Whether a Run also asks the server to explain the query, so the badge row can say how it is
//! served. One global switch (not per connection), on by default. Saved as `explain.json` beside
//! `connections.json`, with the same atomic 0600 write and the same rule for a file that cannot
//! be used: back it up, fall back to the default, and refuse to save when the backup failed so
//! the original is never overwritten.
use super::connections::ConfigPath;
use super::explain_actions::VerdictState;
use super::persistence::{
    BackupFn, PersistError, PersistGuard, Recovery, atomic_write, default_connections_path,
};
use super::reducer::Out;
use super::settings_file::{self, Startup};
use super::state::{AppState, Banner};
use super::{CurlybraceApp, persistence};
use serde_json::{Value, json};
use std::path::Path;

const FILE_NAME: &str = "explain.json";
const FIELD: &str = "automatic";

/// The explain setting. `automatic` is on unless the person turned it off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExplainSettings {
    pub automatic: bool,
}

impl Default for ExplainSettings {
    fn default() -> Self {
        Self { automatic: true }
    }
}

/// `<config dir>/curlybrace/explain.json`, beside the connection list.
pub fn default_explain_path() -> ConfigPath {
    default_connections_path().map(|path| path.with_file_name(FILE_NAME))
}

/// The explain setting found at startup.
pub type ExplainStartup = Startup<ExplainSettings>;

/// Reads the file (see `settings_file::load` for the rules).
pub fn load(path: &Path, backup: BackupFn<'_>) -> ExplainStartup {
    settings_file::load(path, backup, "explain settings", parse, |backed_up| {
        Recovery::Explain { backed_up }
    })
}

/// `{"automatic": true|false}`; a missing field is the default, anything of another shape is
/// corrupt.
fn parse(bytes: &[u8]) -> Result<ExplainSettings, PersistError> {
    let value: Value = settings_file::json_object(bytes)?;
    let automatic = match value.get(FIELD) {
        None => ExplainSettings::default().automatic,
        Some(flag) => flag.as_bool().ok_or(PersistError::Corrupt)?,
    };
    Ok(ExplainSettings { automatic })
}

/// Atomically writes the settings (pretty JSON, 0600).
pub fn save(path: &Path, settings: &ExplainSettings) -> Result<(), PersistError> {
    let bytes = serde_json::to_vec_pretty(&json!({ FIELD: settings.automatic }))
        .map_err(|_| PersistError::Write(std::io::ErrorKind::InvalidData))?;
    atomic_write(path, &bytes)
}

/// Why a change to the setting was not written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// The unusable original could not be backed up, so it must not be replaced.
    Blocked,
    Persist(PersistError),
}

impl SaveError {
    /// The banner for the refusal, raised on every attempt. A blocked save reuses the
    /// failed-backup recovery text, which says changes are not saved.
    pub fn banner(self) -> Banner {
        match self {
            Self::Blocked => Banner::Recovery(Recovery::Explain { backed_up: false }),
            Self::Persist(e) => Banner::Persist(e),
        }
    }
}

/// [`save`] unless the guard says the file must not be replaced.
pub fn saved(
    path: &Path,
    guard: &PersistGuard,
    settings: &ExplainSettings,
) -> Result<(), SaveError> {
    guard.check().map_err(|_| SaveError::Blocked)?;
    save(path, settings).map_err(SaveError::Persist)
}

/// `FindAction::SetAutomaticVerdict`: the switch changes for every collection. Turning it off
/// drops the verdict being shown, which would otherwise stay on screen as a stale badge.
pub(super) fn set_automatic(state: AppState, automatic: bool) -> Out {
    let verdict = if automatic {
        state.verdict
    } else {
        VerdictState::None
    };
    (
        AppState {
            explain: ExplainSettings { automatic },
            verdict,
            ..state
        },
        Vec::new(),
    )
}

impl CurlybraceApp {
    /// Loads the explain setting from `path`. A file that cannot be used is backed up and the
    /// default (on) applies, with the recovery banner (an earlier startup banner keeps its place
    /// and this one waits behind it).
    pub fn with_explain_path(mut self, path: ConfigPath) -> Self {
        let started = match &path {
            Ok(p) => load(p, &persistence::back_up),
            Err(_) => ExplainStartup {
                settings: Default::default(),
                guard: Default::default(),
                recovery: None,
            },
        };
        self.state.explain = started.settings;
        if let Some(recovery) = started.recovery {
            self.state = std::mem::take(&mut self.state).with_recovery_notice(recovery);
        }
        self.explain_path = path;
        self.explain_guard = started.guard;
        self
    }

    /// Writes the setting after a change. A failed write is a banner; the change itself stays
    /// applied for this run.
    pub(super) fn save_explain(&mut self) {
        let result = match &self.explain_path {
            Ok(path) => saved(path, &self.explain_guard, &self.state.explain),
            Err(e) => Err(SaveError::Persist(e.clone())),
        };
        if let Err(e) = result {
            log::warn!("could not save the explain settings: {e:?}");
            self.state.banner = Some(e.banner());
        }
    }
}
