//! Running `mongosh` for the Shell tab with streamed output.
//!
//! Like `tools::probe`: never through a shell, an empty environment apart from `PATH` (absolute
//! entries only) and `HOME`, a neutral working directory, and its own process group so a stop
//! kills the child and everything it started.
//!
//! Threads per run:
//! - a writer: writes the preamble, waits for the stdout reader to see `__CB_READY__`, writes
//!   the script and closes stdin (EOF ends `mongosh`). On `__CB_FAILED__` the script is never
//!   written. A broken pipe just ends it.
//! - one reader per output stream: raw bytes are counted against the cap, decoded (UTF-8, a
//!   character split across reads kept whole), scrubbed, then (stdout) classified.
//! - a supervisor: the only thread that calls `on_output`, then `on_done` exactly once after the
//!   child is reaped and both readers have finished (or were given up on, see [`DRAIN`]).
use super::classify::{Classifier, Piece};
use super::decode::Utf8Decoder;
use super::{Invocation, Scrubber};
use crate::tools::{absolute_entries, kill_group};
use std::ffi::OsStr;
use std::io::{ErrorKind, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
pub const DEFAULT_CHUNK_BYTES: usize = 8 * 1024;
/// How often the supervisor checks whether the child has exited.
const POLL: Duration = Duration::from_millis(10);
/// Grace period for the readers after the child has exited. Something it started may still hold
/// the pipes open: the group is then killed, and after a second grace period the readers are
/// given up on (their output is dropped) so `on_done` still comes.
const DRAIN: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The shell exited on its own with this status and printed no `Uncaught` error.
    Exited(i32),
    /// The shell exited 0 but the script raised at least one `Uncaught` error.
    ScriptErrors,
    /// The preamble could not connect; the script was never sent.
    ConnectFailed,
    /// The shell was killed by a signal we did not send.
    Signalled,
    Cancelled,
    /// Output exceeded [`RunLimits::max_output_bytes`]; the run was stopped.
    CapReached,
    /// The executable could not be started.
    StartFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunLimits {
    /// Most output bytes shown (both streams, counted before scrubbing).
    pub max_output_bytes: usize,
    /// Read size, so the largest piece of text one `on_output` call carries (roughly).
    pub chunk_bytes: usize,
}

impl Default for RunLimits {
    fn default() -> Self {
        Self {
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            chunk_bytes: DEFAULT_CHUNK_BYTES,
        }
    }
}

/// Controls a running shell. Dropping it does not stop the run.
#[derive(Clone, Debug)]
pub struct ShellHandle {
    control: Option<Arc<Control>>,
}

impl ShellHandle {
    /// Kills the shell and everything it started; the run then ends with `Cancelled`. A no-op once
    /// the shell has exited, and when called again.
    pub fn cancel(&self) {
        if let Some(control) = &self.control {
            control.stop(Stop::Cancelled);
        }
    }
}

/// Runs `exe` with `inv` and streams its output to `on_output`; `on_done` follows exactly once,
/// after the last `on_output`. Both are called from a background thread. `secrets` are masked
/// in all output. A failed spawn is reported as `on_done(Outcome::StartFailed)` before this
/// returns.
pub fn spawn_shell(
    exe: &Path,
    inv: Invocation,
    secrets: Vec<String>,
    path_var: Option<&OsStr>,
    limits: RunLimits,
    on_output: impl Fn(Stream, String) + Send + 'static,
    on_done: impl FnOnce(Outcome) + Send + 'static,
) -> ShellHandle {
    let Invocation {
        args,
        preamble,
        script,
    } = inv;
    let mut command = Command::new(exe);
    command
        .args(&args)
        .env_clear()
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = path_var.and_then(absolute_entries) {
        command.env("PATH", path);
    }
    // `mongosh` keeps its config and logs under the home directory.
    if let Some(home) = std::env::var_os("HOME").filter(|home| Path::new(home).is_absolute()) {
        command.env("HOME", home);
    }
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let Ok(mut child) = command.spawn() else {
        on_done(Outcome::StartFailed);
        return ShellHandle { control: None };
    };
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        kill_group(child.id());
        let _ = child.kill();
        let _ = child.wait();
        on_done(Outcome::StartFailed);
        return ShellHandle { control: None };
    };
    let control = Arc::new(Control::new(child.id(), limits.max_output_bytes));
    let chunk_bytes = limits.chunk_bytes.max(1);
    let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
    let (output_tx, output_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::spawn(move || write_input(stdin, preamble, script, &ready_rx));
    let readers = [
        (
            Box::new(stdout) as Box<dyn Read + Send>,
            Lane::Stdout {
                classifier: Classifier::default(),
                ready: Some(ready_tx),
            },
        ),
        (Box::new(stderr), Lane::Stderr),
    ];
    for (source, lane) in readers {
        let reader = Reader {
            control: Arc::clone(&control),
            output: output_tx.clone(),
            scrubber: Scrubber::new(&secrets),
            decoder: Utf8Decoder::default(),
            lane,
        };
        std::thread::spawn(move || reader.run(source, chunk_bytes));
    }
    drop(output_tx);
    let supervised = Arc::clone(&control);
    std::thread::spawn(move || supervise(child, &supervised, &output_rx, on_output, on_done));
    ShellHandle {
        control: Some(control),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    Cancelled,
    CapReached,
    ConnectFailed,
}

#[derive(Debug, Default)]
struct State {
    /// The child has been waited for: its pid may be reused, so it is never signalled again.
    reaped: bool,
    /// Why we stopped the run; the first reason wins.
    stop: Option<Stop>,
}

/// Shared by the handle and the run's threads.
#[derive(Debug)]
struct Control {
    pid: u32,
    state: Mutex<State>,
    max_output_bytes: usize,
    output_bytes: AtomicUsize,
    saw_uncaught: AtomicBool,
}

impl Control {
    fn new(pid: u32, max_output_bytes: usize) -> Self {
        Self {
            pid,
            state: Mutex::new(State::default()),
            max_output_bytes,
            output_bytes: AtomicUsize::new(0),
            saw_uncaught: AtomicBool::new(false),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records why the run stops and kills the group while the child is unreaped. A cancel after
    /// the child exited changes nothing; a cap or connect failure is still the outcome.
    fn stop(&self, why: Stop) {
        let mut state = self.state();
        if state.reaped && why == Stop::Cancelled {
            return;
        }
        state.stop.get_or_insert(why);
        if !state.reaped {
            kill_group(self.pid);
        }
    }

    /// `Some(status)` once the child has exited. Reaps under the lock, so [`Control::stop`]
    /// never signals a pid that was reaped (and possibly reused).
    fn try_reap(&self, child: &mut Child) -> Option<Option<ExitStatus>> {
        let mut state = self.state();
        let status = match child.try_wait() {
            Ok(None) => return None,
            Ok(Some(status)) => Some(status),
            Err(_) => {
                kill_group(self.pid);
                let _ = child.kill();
                child.wait().ok()
            }
        };
        state.reaped = true;
        Some(status)
    }

    /// Counts `n` more raw output bytes; returns how many fit under the cap and stops the run
    /// when they do not all fit.
    fn admit(&self, n: usize) -> usize {
        let before = self.output_bytes.fetch_add(n, Ordering::SeqCst);
        let allowed = self.max_output_bytes.saturating_sub(before).min(n);
        if allowed < n {
            self.stop(Stop::CapReached);
        }
        allowed
    }

    fn outcome(&self, status: Option<ExitStatus>) -> Outcome {
        match self.state().stop {
            Some(Stop::Cancelled) => return Outcome::Cancelled,
            Some(Stop::CapReached) => return Outcome::CapReached,
            Some(Stop::ConnectFailed) => return Outcome::ConnectFailed,
            None => {}
        }
        match status.and_then(|status| status.code()) {
            Some(0) if self.saw_uncaught.load(Ordering::SeqCst) => Outcome::ScriptErrors,
            Some(code) => Outcome::Exited(code),
            None => Outcome::Signalled,
        }
    }
}

/// Writes the preamble, then the script once `ready` says the shell connected, then closes
/// stdin. Any write error (the child exited or never reads) just ends it.
fn write_input(mut stdin: ChildStdin, preamble: Vec<u8>, script: Vec<u8>, ready: &Receiver<()>) {
    let sent = stdin.write_all(&preamble).and_then(|()| stdin.flush());
    // The password's only copy outside the scrubbers: do not keep it for the whole run.
    drop(preamble);
    if sent.is_ok() && ready.recv().is_ok() {
        let _ = stdin.write_all(&script);
    }
}

/// What a reader does with scrubbed text.
enum Lane {
    Stdout {
        classifier: Classifier,
        /// Tells the writer the shell connected; dropped unsent when it did not.
        ready: Option<Sender<()>>,
    },
    Stderr,
}

struct Reader {
    control: Arc<Control>,
    output: Sender<(Stream, String)>,
    scrubber: Scrubber,
    decoder: Utf8Decoder,
    lane: Lane,
}

impl Reader {
    fn run(mut self, mut source: Box<dyn Read + Send>, chunk_bytes: usize) {
        let mut buf = vec![0; chunk_bytes];
        loop {
            let n = match source.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            let allowed = self.control.admit(n);
            let text = self.scrubber.push(&self.decoder.push(&buf[..allowed]));
            if !self.pass(&text, false) || allowed < n {
                break;
            }
        }
        let mut text = self.scrubber.push(&self.decoder.finish());
        text.push_str(&self.scrubber.finish());
        self.pass(&text, true);
    }

    /// Sends what `text` shows; false once nothing more should be read.
    fn pass(&mut self, text: &str, at_end: bool) -> bool {
        match &mut self.lane {
            Lane::Stderr => {
                let text = text.replace('\r', "");
                text.is_empty() || self.output.send((Stream::Stderr, text)).is_ok()
            }
            Lane::Stdout { classifier, ready } => {
                let mut pieces = classifier.push(text);
                if at_end {
                    pieces.extend(classifier.finish());
                }
                if classifier.saw_uncaught() {
                    self.control.saw_uncaught.store(true, Ordering::SeqCst);
                }
                for piece in pieces {
                    match piece {
                        Piece::Out(stream, text) => {
                            if self.output.send((stream, text)).is_err() {
                                return false;
                            }
                        }
                        Piece::Ready => {
                            if let Some(ready) = ready.take() {
                                let _ = ready.send(());
                            }
                        }
                        Piece::Failed(message) => {
                            ready.take();
                            let _ = self.output.send((Stream::Stderr, format!("{message}\n")));
                            self.control.stop(Stop::ConnectFailed);
                            return false;
                        }
                    }
                }
                true
            }
        }
    }
}

/// Forwards output until the child is reaped and the readers are done, then reports the outcome.
fn supervise(
    mut child: Child,
    control: &Control,
    output: &Receiver<(Stream, String)>,
    on_output: impl Fn(Stream, String),
    on_done: impl FnOnce(Outcome),
) {
    let mut readers_done = false;
    let status = loop {
        if readers_done {
            std::thread::sleep(POLL);
        } else {
            readers_done = forward(output, &on_output, Instant::now() + POLL);
        }
        if let Some(status) = control.try_reap(&mut child) {
            break status;
        }
    };
    if !readers_done && !forward(output, &on_output, Instant::now() + DRAIN) {
        // Something the shell started still holds its output open: stop its group, which
        // closes the pipes. One that left the group is out of reach; its reader is abandoned.
        kill_group(control.pid);
        forward(output, &on_output, Instant::now() + DRAIN);
    }
    on_done(control.outcome(status));
}

/// Passes output on until both readers have finished (true) or `deadline` passes (false).
fn forward(
    output: &Receiver<(Stream, String)>,
    on_output: &impl Fn(Stream, String),
    deadline: Instant,
) -> bool {
    loop {
        let wait = deadline.saturating_duration_since(Instant::now());
        match output.recv_timeout(wait) {
            Ok((stream, text)) => on_output(stream, text),
            Err(RecvTimeoutError::Timeout) => return false,
            Err(RecvTimeoutError::Disconnected) => return true,
        }
    }
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
