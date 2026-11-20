//! The settings the app starts its runtime with.
use super::host_key::{HOST_KEY_PROMPT_TIMEOUT, InteractivePrompt};
use curlybrace_core::runtime::{COMMAND_TIMEOUT, RuntimeConfig};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Slack on top of the prompt timeout so a user who answers just in time still connects
/// (the runtime's connect timeout bounds the whole connect, prompt included).
const CONNECT_SLACK: Duration = Duration::from_secs(30);

/// Runtime settings for the app: interactive host-key prompt, a connect timeout that outlasts
/// it, and the normal short bound for data commands. `history_path` is where index usage is
/// recorded (`None` turns history off): the shipped app passes `default_history_path()`, anything
/// else that spawns a runtime passes a throwaway location so it never writes to the user's real
/// config directory. `snapshot_dir` is where the undo copies of bulk writes are kept (`None`
/// turns undo off): the shipped app passes `default_snapshot_dir()`, with the same rule.
pub fn runtime_config(
    known_hosts_path: Option<PathBuf>,
    history_path: Option<PathBuf>,
    snapshot_dir: Option<PathBuf>,
    prompt: Arc<InteractivePrompt>,
) -> RuntimeConfig {
    RuntimeConfig {
        command_timeout: COMMAND_TIMEOUT,
        connect_timeout: HOST_KEY_PROMPT_TIMEOUT + CONNECT_SLACK,
        known_hosts_path,
        host_key_prompt: prompt,
        history_path,
        snapshot_dir,
        ..RuntimeConfig::default()
    }
}
