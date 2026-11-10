//! The race branches of the one-run slot, with real `ShellHandle`s from `spawn_shell` (the fake
//! `mongosh`, or an executable that does not exist).

use std::ffi::OsString;
use std::sync::mpsc;
use std::time::Duration;

use super::shell_slot::ShellRuns;
use crate::connection::ConnectionConfig;
use crate::shell::fake::{Behaviour, FAKE_PATH, FakeMongosh};
use crate::shell::{Outcome, RunLimits, ShellHandle, build_invocation, spawn_shell};

const LONG: Duration = Duration::from_secs(10);
const QUIET: Duration = Duration::from_millis(300);

/// A started run: its handle and the outcome it reports. Cancels on drop so no fake outlives a
/// failing test.
struct Started {
    handle: ShellHandle,
    done: mpsc::Receiver<Outcome>,
}

impl Drop for Started {
    fn drop(&mut self) {
        self.handle.cancel();
    }
}

impl Started {
    fn outcome(&self) -> Outcome {
        self.done.recv_timeout(LONG).expect("the run must finish")
    }

    fn assert_still_running(&self) {
        assert!(self.done.recv_timeout(QUIET).is_err(), "the run ended");
    }
}

fn start(exe: &std::path::Path) -> Started {
    let cfg = ConnectionConfig::new("t", "mongodb://localhost:27017/app");
    let inv = build_invocation(&cfg, None, None, "hang").unwrap();
    let (tx, done) = mpsc::channel();
    let path = OsString::from(FAKE_PATH);
    let handle = spawn_shell(
        exe,
        inv,
        Vec::new(),
        Some(&path),
        RunLimits::default(),
        |_, _| {},
        move |outcome| {
            let _ = tx.send(outcome);
        },
    );
    Started { handle, done }
}

fn fake() -> FakeMongosh {
    FakeMongosh::new(Behaviour::Repl)
}

#[test]
fn attach_after_cancel_all_cancels_the_handle() {
    let (runs, fake) = (ShellRuns::default(), fake());
    assert!(runs.reserve(1));
    // The session ended before `spawn_shell` returned: no handle to stop yet.
    runs.cancel_all();
    let run = start(&fake.exe);
    runs.attach(1, run.handle.clone());
    assert_eq!(run.outcome(), Outcome::Cancelled);
    assert!(runs.reserve(2), "cancel_all freed the slot");
}

#[test]
fn attach_after_a_cancel_request_cancels_the_handle() {
    let (runs, fake) = (ShellRuns::default(), fake());
    assert!(runs.reserve(1));
    runs.cancel(1); // no handle yet: remembered
    assert!(runs.cancel_requested(1));
    let run = start(&fake.exe);
    runs.attach(1, run.handle.clone());
    assert_eq!(run.outcome(), Outcome::Cancelled);
    // The slot stays held until the run's own on_done releases it.
    assert!(!runs.reserve(2));
    runs.release(1);
    assert!(runs.reserve(2));
}

#[test]
fn attach_after_the_slot_was_released_cancels_the_handle() {
    let (runs, fake) = (ShellRuns::default(), fake());
    assert!(runs.reserve(1));
    runs.release(1);
    let run = start(&fake.exe);
    runs.attach(1, run.handle.clone());
    assert_eq!(run.outcome(), Outcome::Cancelled);
}

#[test]
fn attach_after_the_slot_went_to_another_run_leaves_that_run_alone() {
    let (runs, fake) = (ShellRuns::default(), fake());
    assert!(runs.reserve(1));
    runs.cancel_all();
    assert!(runs.reserve(2));
    let stale = start(&fake.exe);
    runs.attach(1, stale.handle.clone());
    assert_eq!(stale.outcome(), Outcome::Cancelled);
    // Run 2 holds the slot and is untouched by the stale attach.
    let current = start(&fake.exe);
    runs.attach(2, current.handle.clone());
    current.assert_still_running();
    assert!(!runs.reserve(3));
    runs.cancel(2);
    assert_eq!(current.outcome(), Outcome::Cancelled);
}

#[test]
fn release_by_a_stale_run_id_keeps_the_current_run() {
    let runs = ShellRuns::default();
    assert!(runs.reserve(1));
    runs.release(2);
    assert!(!runs.reserve(3), "run 1 still holds the slot");
    runs.release(1);
    assert!(runs.reserve(3));
}

#[test]
fn cancel_of_another_run_id_changes_nothing() {
    let (runs, fake) = (ShellRuns::default(), fake());
    assert!(runs.reserve(1));
    runs.cancel(2);
    assert!(!runs.cancel_requested(1));
    let run = start(&fake.exe);
    runs.attach(1, run.handle.clone());
    run.assert_still_running();
}

#[test]
fn a_handle_of_a_failed_start_attaches_and_cancels_harmlessly() {
    let (runs, fake) = (ShellRuns::default(), fake());
    assert!(runs.reserve(1));
    let run = start(&fake.path("no-such-mongosh"));
    assert_eq!(run.outcome(), Outcome::StartFailed);
    runs.attach(1, run.handle.clone());
    runs.cancel(1);
    runs.cancel_all();
    assert!(runs.reserve(2));
}
