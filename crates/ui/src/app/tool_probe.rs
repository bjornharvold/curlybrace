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

/// Starts the check; the answer arrives on `answers` and `repaint` wakes the window. `path` is
/// the `PATH` a `PATH` choice is searched in and the tool is given.
pub(super) fn spawn(
    request: ProbeRequest,
    answers: Sender<ProbeAnswer>,
    repaint: Repaint,
    path: Option<OsString>,
) {
    spawn_with(request, answers, repaint, path, |work| {
        std::thread::Builder::new()
            .name("tool-probe".into())
            .spawn(work)
            .map(|_| ())
    });
}

/// [`spawn`] with the way a thread is started. Whatever happens, one answer is sent: a thread
/// that cannot start or that panics answers `CouldNotRun`, so a row never stays on "Checking…".
pub(super) fn spawn_with(
    request: ProbeRequest,
    answers: Sender<ProbeAnswer>,
    repaint: Repaint,
    path: Option<OsString>,
    start: impl FnOnce(Box<dyn FnOnce() + Send>) -> std::io::Result<()>,
) {
    let ProbeRequest {
        scope,
        kind,
        request,
        choice,
    } = request;
    let reply = move |answers: &Sender<ProbeAnswer>,
                      repaint: &Repaint,
                      result: Result<ToolVersion, ToolError>| {
        let _ = answers.send(ProbeAnswer {
            scope,
            kind,
            request,
            result,
        });
        repaint();
    };
    let (worker_answers, worker_repaint) = (answers.clone(), repaint.clone());
    let work: Box<dyn FnOnce() + Send> = Box::new(move || {
        let run = std::panic::AssertUnwindSafe(|| {
            resolve(kind, &choice, path.as_deref())
                .and_then(|exe| probe_version_with_path(kind, &exe, PROBE_TIMEOUT, path.as_deref()))
        });
        let result = std::panic::catch_unwind(run).unwrap_or(Err(ToolError::CouldNotRun));
        reply(&worker_answers, &worker_repaint, result);
    });
    if let Err(e) = start(work) {
        log::warn!("could not start a tool version check: {e}");
        reply(&answers, &repaint, Err(ToolError::CouldNotRun));
    }
}
