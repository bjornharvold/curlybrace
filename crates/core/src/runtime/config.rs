//! Runtime tunables: the timeouts and the config the spawn functions take.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::tunneling::TunnelOpener;
use super::{SamplerConfig, SshTunnelOpener, default_known_hosts_path};
use crate::tunnel::{HostKeyPrompt, RejectUnknownHostKeys};

/// Upper bound for any single command (including connect).
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Upper bound for an index build (`Command::CreateIndex`). A build on a large collection runs
/// far longer than a data command, and cutting it off would leave the server building while the
/// app reports an unknown outcome.
pub const INDEX_BUILD_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// Upper bound for the reachability ping that follows a suspect failure of a data command.
pub const PING_TIMEOUT: Duration = Duration::from_secs(3);

/// Upper bound for the duplicate-key aggregation of a compare pair (it may scan a whole
/// collection).
pub const DUPLICATE_TIMEOUT: Duration = Duration::from_secs(120);

/// Upper bound for a Connect (connector, SSH tunnel and any host-key prompt the user must answer).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(150);

/// Runtime tunables; the timeouts are overridable so tests stay fast.
///
/// SSH tunnels: `known_hosts_path` is the app-owned host-key file and
/// `host_key_prompt` decides about UNKNOWN host keys. The default prompt is
/// [`RejectUnknownHostKeys`] (safe); the UI (Task 16) supplies an interactive
/// one. The prompt is synchronous and is called via `spawn_blocking`, so the
/// UI side may block on a channel until the user answers. Keep
/// `connect_timeout` in mind: it bounds the whole connect, prompt included, while
/// `command_timeout` bounds data commands only.
#[derive(Clone)]
pub struct RuntimeConfig {
    /// Bound for each data command (find, insert, ...).
    pub command_timeout: Duration,
    /// Bound for an index build; every other data command keeps `command_timeout`.
    pub index_build_timeout: Duration,
    /// Bound for the ping that settles a lost connection or a timed-out command.
    pub ping_timeout: Duration,
    /// Bound for a Connect, including an SSH host-key prompt waiting for the user.
    pub connect_timeout: Duration,
    /// Bound for the duplicate-key search of a compare pair.
    pub duplicate_timeout: Duration,
    /// `None` when the config directory is unknown: SSH connects then fail closed.
    pub known_hosts_path: Option<PathBuf>,
    pub host_key_prompt: Arc<dyn HostKeyPrompt>,
    pub tunnel_opener: Arc<dyn TunnelOpener>,
    /// Result budget and preflight thresholds of every compare session.
    pub compare_limits: crate::compare::results::Limits,
    /// The usage history database. `None` (the default) disables history and sampling: no
    /// sampler starts and `IndexHistory` answers with no series. The app passes
    /// [`default_history_path`]. The file is opened once, when the runtime is spawned; if that
    /// fails, or the file had to be set aside as corrupt, a warning is logged and history stays off
    /// (the app keeps working; a later task can surface it, see `Shared::history`).
    pub history_path: Option<PathBuf>,
    /// The first sampler tunables; `SetSamplingExcluded` changes the excluded databases later.
    /// The interval is raised to [`MIN_INTERVAL`].
    pub sampler: SamplerConfig,
}

/// `curlybrace/history.sqlite` in the OS config directory; `None` when that is unknown.
pub fn default_history_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("curlybrace").join("history.sqlite"))
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            command_timeout: COMMAND_TIMEOUT,
            index_build_timeout: INDEX_BUILD_TIMEOUT,
            ping_timeout: PING_TIMEOUT,
            connect_timeout: CONNECT_TIMEOUT,
            duplicate_timeout: DUPLICATE_TIMEOUT,
            known_hosts_path: default_known_hosts_path(),
            host_key_prompt: Arc::new(RejectUnknownHostKeys),
            tunnel_opener: Arc::new(SshTunnelOpener),
            compare_limits: Default::default(),
            // Off unless asked for: tests and embedders must not write into the user's config.
            history_path: None,
            sampler: SamplerConfig::default(),
        }
    }
}

impl std::fmt::Debug for RuntimeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeConfig")
            .field("command_timeout", &self.command_timeout)
            .field("index_build_timeout", &self.index_build_timeout)
            .field("connect_timeout", &self.connect_timeout)
            .field("duplicate_timeout", &self.duplicate_timeout)
            .field("known_hosts_path", &self.known_hosts_path)
            .field("history_path", &self.history_path)
            .finish_non_exhaustive()
    }
}
