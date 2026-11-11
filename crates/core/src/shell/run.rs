//! Running `mongosh` for the Shell tab with streamed output.
//!
//! Like `tools::probe`: never through a shell, an empty environment apart from `PATH` (absolute
//! entries only) and `HOME`, a neutral working directory, and its own process group so a stop
//! kills the child and everything it started.
//!
//! Threads per run (see `pipe.rs`):
//! - a writer: writes the preamble, waits for the stdout reader to see `__CB_READY__`, then writes
//!   the script (one block statement) a line at a time, each line only when `mongosh` asks for it,
//!   and closes stdin (EOF ends `mongosh`) as soon as `mongosh` evaluated what it read, or after
//!   the last line once the verdict on the epilogue is in: when the block was evaluated cleanly it
//!   first writes the epilogue (`invocation::epilogue`), which prints the block's value as one
//!   marked line of EJSON. Without READY the script is never written.
//! - one reader per output stream: raw bytes are counted against the cap, decoded (UTF-8, a
//!   character split across reads kept whole), then scrubbed; stdout is classified before it is
//!   scrubbed (the classifier also paces the writer, gives the verdict on the epilogue and lifts
//!   its answer out of the text, see `classify_result.rs`).
//! - a supervisor: the only thread that calls `on_output` and `on_result`, in the order the
//!   readers handed them over (text keeps its own order; text the scrubber held back may still
//!   come after the result), then `on_done` exactly once after the child has exited and both
//!   readers have finished (or were given up on, see [`DRAIN`]). It reaps the child last, so
//!   every kill before that targets a group that is still ours.
use super::control::{Control, Stop};
use super::decode::Utf8Decoder;
use super::invocation::{RESULT_MAX_CHARS, epilogue};
use super::pipe::{Delivery, Expect, Lane, Reader, write_input};
use super::{Invocation, Scrubber};
use crate::tools::{absolute_entries, kill_group};
use std::ffi::OsStr;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
pub const DEFAULT_CHUNK_BYTES: usize = 8 * 1024;
/// Most UTF-8 bytes one UTF-16 unit takes (a surrogate pair: 2 units, 4 bytes).
const MAX_UTF8_BYTES_PER_UTF16_UNIT: usize = 3;
/// The emitter caps its payload at [`RESULT_MAX_CHARS`] UTF-16 units, so in UTF-8 it is at most
/// three times that; plus a little slack. A result the emitter lets through therefore always
/// fits; one over this (a lower limit set in [`RunLimits`]) is dropped, and never stops the run.
pub const DEFAULT_MAX_RESULT_BYTES: usize =
    MAX_UTF8_BYTES_PER_UTF16_UNIT * RESULT_MAX_CHARS + 64 * 1024;
const _: () = assert!(DEFAULT_MAX_RESULT_BYTES >= MAX_UTF8_BYTES_PER_UTF16_UNIT * RESULT_MAX_CHARS);
/// A little over the driver's 30 s server selection timeout.
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(35);
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
    /// The shell exited 0 without an `Uncaught` error, but stopped reading the script before its
    /// end: a bracket, string or comment was left open (nothing ran), or a `}` closed the script
    /// early (what came after it never ran).
    Incomplete,
    /// The preamble did not connect (it failed, the shell ended before answering, or no answer
    /// came within [`RunLimits::handshake_timeout`]); the script was never sent.
    ConnectFailed,
    /// The shell was killed by a signal we did not send.
    Signalled,
    /// [`ShellHandle::cancel`] stopped the run, or `on_output` panicked (the run is then stopped).
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
    /// How long the preamble may take to answer before the run stops with `ConnectFailed`.
    pub handshake_timeout: Duration,
    /// Longest structured result kept (the payload line, in bytes); a longer one is dropped and
    /// the run goes on (it never ends in `CapReached` for it). It does not count against
    /// `max_output_bytes`.
    pub max_result_bytes: usize,
}

impl Default for RunLimits {
    fn default() -> Self {
        Self {
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            chunk_bytes: DEFAULT_CHUNK_BYTES,
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
            max_result_bytes: DEFAULT_MAX_RESULT_BYTES,
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
    /// the shell has been reaped, and when called again. This is also the only way to end a run
    /// whose script silenced or replaced `process.stdout.write` (or otherwise stopped `mongosh`
    /// printing): no prompt comes after the block, so the writer waits for a verdict that never
    /// comes; the kill ends the stdout reader, which closes the pace channel and ends the writer.
    pub fn cancel(&self) {
        if let Some(control) = &self.control {
            control.stop(Stop::Cancelled);
        }
    }
}

/// Runs `exe` with `inv` and streams its output to `on_output`; `on_done` follows exactly once,
/// after the last `on_output`. `on_result` gets the structured result (the payload JSON text of
/// the epilogue's answer, masked, and whether masking changed it: then it may no longer parse,
/// see `parse_scrubbed`) at most once, and only before `on_done`; there is none when
/// the script failed, ended early or assigned `_`, when the end of the block was never seen,
/// when the shell did not answer, or when the answer was over [`RunLimits::max_result_bytes`]
/// (the preamble locks `prompt` and `__cbEmit`, so a script cannot replace either: see
/// `invocation::locked_global_js`). A script that silences or replaces `process.stdout.write`
/// (or otherwise stops `mongosh` printing) leaves no prompt after the block: the writer then
/// waits for a verdict that never comes and the run is ended only by [`ShellHandle::cancel`]
/// (or the process exiting). All three are called
/// from one background thread. `secrets` are masked in all output. A failed spawn is reported
/// as `on_done(Outcome::StartFailed)` before this returns. If `on_output` or `on_result`
/// panics, the run is stopped, later output is dropped and `on_done` still comes, with
/// `Outcome::Cancelled`.
#[allow(clippy::too_many_arguments)] // three callbacks with different signatures
pub fn spawn_shell(
    exe: &Path,
    inv: Invocation,
    secrets: Vec<String>,
    path_var: Option<&OsStr>,
    limits: RunLimits,
    on_output: impl Fn(Stream, String) + Send + 'static,
    on_result: impl FnMut(String, bool) + Send + 'static,
    on_done: impl FnOnce(Outcome) + Send + 'static,
) -> ShellHandle {
    let Invocation {
        args,
        preamble,
        script,
        result_token,
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
    let control = Arc::new(Control::new(
        child.id(),
        limits.max_output_bytes,
        limits.max_result_bytes,
    ));
    let chunk_bytes = limits.chunk_bytes.max(1);
    let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
    let (output_tx, output_rx) = mpsc::channel();
    let (pace_tx, pace_rx) = mpsc::channel();
    let lines = script.split_inclusive(|&b| b == b'\n').count();
    let epilogue_line = format!("{}\n", epilogue(&result_token)).into_bytes();
    std::thread::spawn(move || write_input(stdin, preamble, script, epilogue_line, &pace_rx));
    let reader = |lane| Reader {
        control: Arc::clone(&control),
        output: output_tx.clone(),
        scrubber: Scrubber::new(&secrets),
        decoder: Utf8Decoder::default(),
        lane,
    };
    let expect = Expect {
        token: &result_token,
        max_result_bytes: limits.max_result_bytes,
    };
    let stdout_lane = Lane::stdout(pace_tx, lines.saturating_sub(1), expect);
    let (out_reader, err_reader) = (reader(stdout_lane), reader(Lane::Stderr));
    std::thread::spawn(move || out_reader.run(stdout, chunk_bytes));
    std::thread::spawn(move || err_reader.run(stderr, chunk_bytes));
    drop(output_tx);
    let supervisor = Supervisor {
        control: Arc::clone(&control),
        output: output_rx,
        on_output,
        on_result,
        result_delivered: false,
        panicked: false,
    };
    let timeout = limits.handshake_timeout;
    std::thread::spawn(move || supervisor.run(child, timeout, on_done));
    ShellHandle {
        control: Some(control),
    }
}

struct Supervisor<F, R> {
    control: Arc<Control>,
    output: Receiver<Delivery>,
    on_output: F,
    on_result: R,
    /// `on_result` was called: it never is again.
    result_delivered: bool,
    /// A callback panicked: the run was stopped and later output is dropped.
    panicked: bool,
}

impl<F: Fn(Stream, String), R: FnMut(String, bool)> Supervisor<F, R> {
    /// Forwards output until the child has exited and the readers are done, then reports.
    /// `on_result` is called only from `forward`, on this thread, and `on_done` only after the
    /// last `forward` returned, so a result always comes before `on_done`; one a reader still
    /// produces after that (a reader given up on) is never delivered.
    fn run(mut self, mut child: Child, handshake_timeout: Duration, on_done: impl FnOnce(Outcome)) {
        let handshake_deadline = Instant::now() + handshake_timeout;
        let mut timed_out = false;
        let mut readers_done = false;
        loop {
            if readers_done {
                std::thread::sleep(POLL);
            } else {
                readers_done = self.forward(Instant::now() + POLL);
            }
            if !timed_out && !self.control.handshake_done() && Instant::now() >= handshake_deadline
            {
                timed_out = true;
                self.control.stop(Stop::ConnectFailed);
            }
            if self.control.exited(&mut child) {
                break;
            }
        }
        if !readers_done && !self.forward(Instant::now() + DRAIN) {
            // Something the shell started still holds its output open. The child is not reaped
            // yet, so its group id is still ours: stop the group, which closes the pipes. One
            // that left the group is out of reach; its reader is abandoned.
            self.control.kill();
            self.forward(Instant::now() + DRAIN);
        }
        let status = self.control.reap(&mut child);
        on_done(self.control.outcome(status));
    }

    /// Passes output on until both readers have finished (true) or `deadline` passes (false).
    fn forward(&mut self, deadline: Instant) -> bool {
        loop {
            let wait = deadline.saturating_duration_since(Instant::now());
            match self.output.recv_timeout(wait) {
                Ok(delivery) => self.deliver(delivery),
                Err(RecvTimeoutError::Timeout) => return false,
                Err(RecvTimeoutError::Disconnected) => return true,
            }
        }
    }

    fn deliver(&mut self, delivery: Delivery) {
        if self.panicked {
            return;
        }
        let call = match delivery {
            Delivery::Text(stream, text) => {
                catch_unwind(AssertUnwindSafe(|| (self.on_output)(stream, text)))
            }
            Delivery::Result(..) if self.result_delivered => return,
            Delivery::Result(payload, masked) => {
                self.result_delivered = true;
                catch_unwind(AssertUnwindSafe(|| (self.on_result)(payload, masked)))
            }
        };
        if call.is_err() {
            self.panicked = true;
            self.control.stop(Stop::CallbackPanicked);
        }
    }
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
