//! Running a version check on a worker thread, so the window never waits for a program.
//!
//! The thread resolves the choice (nothing runs if the file is not an executable file), asks the
//! program for `--version` and sends the answer back for the next frame to pick up. Paths and
//! program output are never logged.
use super::tools_state::{ProbeRequest, ProbeScope};
use curlybrace_core::runtime::Repaint;
use curlybrace_core::tools::{ToolError, ToolKind, ToolVersion, probe_version_with_path, resolve};
use std::ffi::OsString;
use std::sync::mpsc::Sender;
use std::time::Duration;

/// How long a program may take to print its version.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// The answer to one check, stamped with the number of the request it belongs to.
#[derive(Debug)]
pub(super) struct ProbeAnswer {
    pub scope: ProbeScope,
    pub kind: ToolKind,
    pub request: u64,
    pub result: Result<ToolVersion, ToolError>,
}

/// Starts the check; the answer arrives on `answers` and `repaint` wakes the window.
/// `path` is the `PATH` a `PATH` choice is searched in and the tool is given.
pub(super) fn spawn(
    request: ProbeRequest,
    answers: Sender<ProbeAnswer>,
    repaint: Repaint,
    path: Option<OsString>,
) {
    let spawned = std::thread::Builder::new()
        .name("tool-probe".into())
        .spawn(move || {
            let ProbeRequest {
                scope,
                kind,
                request,
                choice,
            } = request;
            let result = resolve(kind, &choice, path.as_deref()).and_then(|exe| {
                probe_version_with_path(kind, &exe, PROBE_TIMEOUT, path.as_deref())
            });
            let _ = answers.send(ProbeAnswer {
                scope,
                kind,
                request,
                result,
            });
            repaint();
        });
    if let Err(e) = spawned {
        log::warn!("could not start a tool version check: {e}");
    }
}
