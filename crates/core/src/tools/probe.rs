//! Asking a tool for its version.
use super::{ToolError, ToolVersion};
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// How much of the output is read; a version fits in far less.
const OUTPUT_LIMIT: u64 = 8 * 1024;
const POLL: Duration = Duration::from_millis(10);
/// Grace period for the reader after the tool has exited.
const DRAIN: Duration = Duration::from_secs(1);

/// Runs `<exe> --version` and parses the answer. Never through a shell; no input, no error
/// stream, a neutral working directory and an empty environment apart from `PATH` (wrappers such
/// as a `node` script need it to find their interpreter). A tool still running after `timeout`
/// is killed together with anything it started.
pub fn probe_version(exe: &Path, timeout: Duration) -> Result<ToolVersion, ToolError> {
    probe_version_with_path(exe, timeout, std::env::var_os("PATH").as_deref())
}

/// [`probe_version`] with the `PATH` the tool gets. Only its absolute entries are passed on: a
/// relative or empty one would let a wrapper run whatever sits in the working directory.
pub fn probe_version_with_path(
    exe: &Path,
    timeout: Duration,
    path: Option<&OsStr>,
) -> Result<ToolVersion, ToolError> {
    let mut command = Command::new(exe);
    command
        .arg("--version")
        .env_clear()
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(path) = path.and_then(absolute_entries) {
        command.env("PATH", path);
    }
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().map_err(|_| ToolError::CouldNotRun)?;
    let stdout = child.stdout.take().ok_or(ToolError::CouldNotRun)?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.take(OUTPUT_LIMIT).read_to_end(&mut bytes);
        let _ = tx.send(bytes);
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            Ok(None) => {
                stop(&mut child);
                return Err(ToolError::TimedOut);
            }
            Err(_) => {
                stop(&mut child);
                return Err(ToolError::CouldNotRun);
            }
        }
    }
    let bytes = rx.recv_timeout(DRAIN).map_err(|_| ToolError::NoVersion)?;
    ToolVersion::parse(&String::from_utf8_lossy(&bytes)).ok_or(ToolError::NoVersion)
}

/// `path` without its relative and empty entries; `None` when nothing is left.
fn absolute_entries(path: &OsStr) -> Option<OsString> {
    let kept: Vec<_> = std::env::split_paths(path)
        .filter(|entry| entry.is_absolute())
        .collect();
    if kept.is_empty() {
        return None;
    }
    std::env::join_paths(kept).ok()
}

/// Kills the child and, on Unix, its whole process group, then reaps it.
fn stop(child: &mut std::process::Child) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(child.id()) {
        // The child leads its own group (see `process_group(0)`), so this reaches the processes
        // it started as well and never anything of ours.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
#[path = "probe_tests.rs"]
mod tests;
