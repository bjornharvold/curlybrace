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

/// Upper bound for a bulk update or delete. It can touch a whole collection, so it runs as long
/// as an index build; cutting it off answers an unknown outcome, never a failure.
pub const BULK_WRITE_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// Most bytes one undo snapshot (originals plus after-state) may take.
pub const SNAPSHOT_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// Server-side time limit of the match count shown before a bulk write.
pub const BULK_COUNT_MAX_TIME: Duration = Duration::from_secs(10);

/// Client-side bound of an aggregate preview: a little above the server-side `maxTimeMS`
/// ([`crate::pipeline::PREVIEW_MAX_TIME_MS`]), so the server's own timeout normally answers
/// first. The runtime still applies `command_timeout` when that is shorter.
pub const AGGREGATE_PREVIEW_TIMEOUT: Duration =
    Duration::from_millis(crate::pipeline::PREVIEW_MAX_TIME_MS + 2_000);

/// Client-side bound of an aggregate write run: a little above the server-side `maxTimeMS`
/// ([`crate::pipeline::AGGREGATE_RUN_MAX_TIME_MS`]). Capped by `index_build_timeout`, the
/// runtime's tunable for long-running commands (not by the short `command_timeout`).
pub const AGGREGATE_RUN_TIMEOUT: Duration =
    Duration::from_millis(crate::pipeline::AGGREGATE_RUN_MAX_TIME_MS + 5_000);

/// Client-side bound of the count that follows a write run. Its own bound, apart from the
/// write's: a slow count after a write that went through leaves the run a success without a
/// count, never an unknown outcome.
pub const AGGREGATE_RUN_COUNT_TIMEOUT: Duration = Duration::from_secs(5);

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
    /// Bound for a bulk update or delete; their previews, counts and cancels keep `command_timeout`.
    pub bulk_write_timeout: Duration,
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
    /// Where undo snapshots of bulk writes are kept. `None` (the default) turns undo off: a
    /// bulk write asking for a snapshot is refused instead of running without one. The app
    /// passes [`default_snapshot_dir`].
    pub snapshot_dir: Option<PathBuf>,
    /// Size cap of one run's snapshot files together.
    pub snapshot_max_bytes: u64,
    /// The first sampler tunables; `SetSamplingExcluded` changes the excluded databases later.
    /// The interval is raised to [`MIN_INTERVAL`].
    pub sampler: SamplerConfig,
}

/// `curlybrace/history.sqlite` in the OS config directory; `None` when that is unknown.
pub fn default_history_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("curlybrace").join("history.sqlite"))
}

/// `curlybrace/snapshots` in the OS config directory; `None` when that is unknown.
pub fn default_snapshot_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("curlybrace").join("snapshots"))
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            command_timeout: COMMAND_TIMEOUT,
            index_build_timeout: INDEX_BUILD_TIMEOUT,
            bulk_write_timeout: BULK_WRITE_TIMEOUT,
            ping_timeout: PING_TIMEOUT,
            connect_timeout: CONNECT_TIMEOUT,
            duplicate_timeout: DUPLICATE_TIMEOUT,
            known_hosts_path: default_known_hosts_path(),
            host_key_prompt: Arc::new(RejectUnknownHostKeys),
            tunnel_opener: Arc::new(SshTunnelOpener),
            compare_limits: Default::default(),
            // Off unless asked for: tests and embedders must not write into the user's config.
            history_path: None,
            // Off unless asked for, for the same reason.
            snapshot_dir: None,
            snapshot_max_bytes: SNAPSHOT_MAX_BYTES,
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
