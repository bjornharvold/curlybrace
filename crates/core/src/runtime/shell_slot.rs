//! The one `mongosh` run a session may have at a time.
//!
//! The lock is held only to check, insert or clear: never while a process is spawned or killed,
//! because `spawn_shell` may call its `on_done` (which clears the slot) before it returns.

use std::sync::Mutex;

use crate::shell::ShellHandle;

#[derive(Debug)]
struct Active {
    run_id: u64,
    /// `None` between the reservation and `spawn_shell` returning.
    handle: Option<ShellHandle>,
    /// A cancel arrived before the handle did; applied when it is attached.
    cancel_requested: bool,
}

#[derive(Debug, Default)]
pub(super) struct ShellRuns {
    slot: Mutex<Option<Active>>,
}

impl ShellRuns {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Active>> {
        // A plain Option: a poisoned lock is still valid.
        self.slot.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes the slot for `run_id`; false when a run already holds it.
    pub(super) fn reserve(&self, run_id: u64) -> bool {
        let mut slot = self.lock();
        if slot.is_some() {
            return false;
        }
        *slot = Some(Active {
            run_id,
            handle: None,
            cancel_requested: false,
        });
        true
    }

    /// Keeps `handle` for the run that reserved the slot. When that run was cancelled, replaced or
    /// has already finished, the handle is cancelled instead (a no-op for a finished run).
    pub(super) fn attach(&self, run_id: u64, handle: ShellHandle) {
        let stop = {
            let mut slot = self.lock();
            match slot.as_mut() {
                Some(active) if active.run_id == run_id && !active.cancel_requested => {
                    active.handle = Some(handle);
                    return;
                }
                _ => handle,
            }
        };
        stop.cancel();
    }

    /// Whether a cancel for `run_id` arrived before its handle was attached.
    pub(super) fn cancel_requested(&self, run_id: u64) -> bool {
        let slot = self.lock();
        slot.as_ref()
            .is_some_and(|a| a.run_id == run_id && a.cancel_requested)
    }

    /// Frees the slot if `run_id` still holds it.
    pub(super) fn release(&self, run_id: u64) {
        let mut slot = self.lock();
        if slot.as_ref().is_some_and(|a| a.run_id == run_id) {
            *slot = None;
        }
    }

    /// Kills the run `run_id`; any other id is ignored. The slot is freed by the run's `on_done`.
    pub(super) fn cancel(&self, run_id: u64) {
        let handle = {
            let mut slot = self.lock();
            match slot.as_mut() {
                Some(active) if active.run_id == run_id => {
                    active.cancel_requested = true;
                    active.handle.clone()
                }
                _ => None,
            }
        };
        if let Some(handle) = handle {
            handle.cancel();
        }
    }

    /// Frees the slot and kills whatever runs: the session ended.
    pub(super) fn cancel_all(&self) {
        let active = self.lock().take();
        if let Some(handle) = active.and_then(|a| a.handle) {
            handle.cancel();
        }
    }
}
