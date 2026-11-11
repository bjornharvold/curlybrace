//! State shared by a run's handle and threads: why it stopped, the output budget, and the child's
//! exit. The child is only reaped at the very end of the run, so while any thread may still
//! signal its process group the leader's pid (and with it the group id) cannot be reused.
use super::run::Outcome;
use crate::tools::kill_group;
use std::process::{Child, ExitStatus};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

/// Bytes the answer window may hold beyond the answer itself.
const WINDOW_SLACK: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stop {
    Cancelled,
    CapReached,
    ConnectFailed,
    /// `on_output` panicked; reported as `Cancelled`.
    CallbackPanicked,
}

#[derive(Debug, Default)]
struct State {
    /// The child has been waited for: its pid may be reused, so it is never signalled again.
    reaped: bool,
    status: Option<ExitStatus>,
    /// Why we stopped the run; the first reason wins.
    stop: Option<Stop>,
}

#[derive(Debug)]
pub(super) struct Control {
    pid: u32,
    state: Mutex<State>,
    max_output_bytes: usize,
    output_bytes: AtomicUsize,
    /// The budget of the epilogue's answer window, counted apart from the text's.
    max_window_bytes: usize,
    window_bytes: AtomicUsize,
    /// `window_bytes` went over `max_window_bytes`.
    window_overflowed: AtomicBool,
    saw_uncaught: AtomicBool,
    /// `mongosh` stopped reading the script before its end.
    incomplete: AtomicBool,
    /// The stdout reader saw `__CB_READY__` or `__CB_FAILED__`.
    handshake_done: AtomicBool,
    /// The stdin writer returned (stdin is closed); tests wait on it.
    writer_done: Mutex<bool>,
    writer_done_changed: Condvar,
}

impl Control {
    /// The answer window may take `max_result_bytes` plus [`WINDOW_SLACK`] (its marker, line
    /// ends, prompt and any late output in it) before its answer is dropped.
    pub(super) fn new(pid: u32, max_output_bytes: usize, max_result_bytes: usize) -> Self {
        Self {
            pid,
            state: Mutex::new(State::default()),
            max_output_bytes,
            output_bytes: AtomicUsize::new(0),
            max_window_bytes: max_result_bytes.saturating_add(WINDOW_SLACK),
            window_bytes: AtomicUsize::new(0),
            window_overflowed: AtomicBool::new(false),
            saw_uncaught: AtomicBool::new(false),
            incomplete: AtomicBool::new(false),
            handshake_done: AtomicBool::new(false),
            writer_done: Mutex::new(false),
            writer_done_changed: Condvar::new(),
        }
    }

    /// The stdin writer returned.
    pub(super) fn mark_writer_done(&self) {
        *self
            .writer_done
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = true;
        self.writer_done_changed.notify_all();
    }

    /// Waits up to `timeout` for the stdin writer to return; whether it did.
    #[cfg(test)]
    pub(super) fn wait_writer_done(&self, timeout: std::time::Duration) -> bool {
        let done = self
            .writer_done
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (done, _) = self
            .writer_done_changed
            .wait_timeout_while(done, timeout, |done| !*done)
            .unwrap_or_else(PoisonError::into_inner);
        *done
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records why the run stops and kills the group while the child is unreaped. A cancel
    /// after the child was reaped changes nothing; other reasons are still the outcome.
    pub(super) fn stop(&self, why: Stop) {
        let mut state = self.state();
        if state.reaped && why == Stop::Cancelled {
            return;
        }
        state.stop.get_or_insert(why);
        if !state.reaped {
            kill_group(self.pid);
        }
    }

    /// Whether a stop reason was recorded (the output may have been cut).
    pub(super) fn stopped(&self) -> bool {
        self.state().stop.is_some()
    }

    /// Whether the run was cancelled (or stopped as cancelled after a callback panic).
    pub(super) fn cancelled(&self) -> bool {
        matches!(
            self.state().stop,
            Some(Stop::Cancelled | Stop::CallbackPanicked)
        )
    }

    /// Kills the group without recording a reason, while the child is unreaped.
    pub(super) fn kill(&self) {
        if !self.state().reaped {
            kill_group(self.pid);
        }
    }

    /// Counts `n` more raw output bytes; returns how many fit under the cap and stops the run
    /// when they do not all fit. The only budget that stops a run.
    pub(super) fn admit(&self, n: usize) -> usize {
        let before = self.output_bytes.fetch_add(n, Ordering::SeqCst);
        let allowed = self.max_output_bytes.saturating_sub(before).min(n);
        if allowed < n {
            self.stop(Stop::CapReached);
        }
        allowed
    }

    /// Counts `n` bytes read in the epilogue's answer window against the window's own budget, so
    /// a large answer does not eat the text's. They are always all admitted and the run is never
    /// stopped here: the text in the window is still charged against the text cap (see
    /// `pipe::within_text_cap`), and the classifier holds at most `max_result_bytes` of the
    /// answer. Going over the budget only drops the answer ([`Control::window_overflowed`]).
    pub(super) fn admit_window(&self, n: usize) -> usize {
        let before = self.window_bytes.fetch_add(n, Ordering::SeqCst);
        if before.saturating_add(n) > self.max_window_bytes {
            self.window_overflowed.store(true, Ordering::SeqCst);
        }
        n
    }

    /// More was read in the answer window than its budget: the answer is dropped.
    pub(super) fn window_overflowed(&self) -> bool {
        self.window_overflowed.load(Ordering::SeqCst)
    }

    pub(super) fn mark_uncaught(&self) {
        self.saw_uncaught.store(true, Ordering::SeqCst);
    }

    pub(super) fn mark_incomplete(&self) {
        self.incomplete.store(true, Ordering::SeqCst);
    }

    pub(super) fn mark_handshake_done(&self) {
        self.handshake_done.store(true, Ordering::SeqCst);
    }

    pub(super) fn handshake_done(&self) -> bool {
        self.handshake_done.load(Ordering::SeqCst)
    }

    /// True once the child has exited. It is left unreaped (a zombie) where the platform can
    /// check that; otherwise it is reaped here.
    pub(super) fn exited(&self, child: &mut Child) -> bool {
        if let Some(exited) = exited_unreaped(self.pid) {
            return exited;
        }
        let mut state = self.state();
        if state.reaped {
            return true;
        }
        match child.try_wait() {
            Ok(None) => false,
            Ok(Some(status)) => {
                state.reaped = true;
                state.status = Some(status);
                true
            }
            Err(_) => {
                kill_group(self.pid);
                let _ = child.kill();
                state.status = child.wait().ok();
                state.reaped = true;
                true
            }
        }
    }

    /// Reaps the exited child (under the lock, so [`Control::stop`] never signals a reaped pid).
    pub(super) fn reap(&self, child: &mut Child) -> Option<ExitStatus> {
        let mut state = self.state();
        if !state.reaped {
            state.status = child.wait().ok();
            state.reaped = true;
        }
        state.status
    }

    pub(super) fn outcome(&self, status: Option<ExitStatus>) -> Outcome {
        match self.state().stop {
            Some(Stop::Cancelled | Stop::CallbackPanicked) => return Outcome::Cancelled,
            Some(Stop::CapReached) => return Outcome::CapReached,
            Some(Stop::ConnectFailed) => return Outcome::ConnectFailed,
            None => {}
        }
        match status.and_then(|status| status.code()) {
            Some(0) if self.saw_uncaught.load(Ordering::SeqCst) => Outcome::ScriptErrors,
            Some(0) if self.incomplete.load(Ordering::SeqCst) => Outcome::Incomplete,
            Some(code) => Outcome::Exited(code),
            None => Outcome::Signalled,
        }
    }
}

/// Whether `pid` has exited, without reaping it; `None` when that cannot be checked.
#[cfg(unix)]
fn exited_unreaped(pid: u32) -> Option<bool> {
    let id: libc::id_t = pid;
    // SAFETY: an all-zero `siginfo_t` is valid; `waitid` only writes into it.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let flags = libc::WEXITED | libc::WNOHANG | libc::WNOWAIT;
    // SAFETY: `info` is a valid, writable `siginfo_t`.
    if unsafe { libc::waitid(libc::P_PID, id, &mut info, flags) } == 0 {
        // With WNOHANG, `si_pid` stays 0 while the child is still running.
        // SAFETY: `waitid` succeeded, so `info` is initialised.
        return Some(unsafe { info.si_pid() } != 0);
    }
    match std::io::Error::last_os_error().kind() {
        std::io::ErrorKind::Interrupted => Some(false),
        _ => None,
    }
}

#[cfg(not(unix))]
fn exited_unreaped(_pid: u32) -> Option<bool> {
    None
}
