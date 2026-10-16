//! A flag that says whether one attempt (a connect or a connection test) is still wanted, and a
//! guard that clears it when the attempt ends. A host-key question can outlive the future that
//! asked it (the answer arrives on a blocking thread), so the flag, not the future, decides
//! whether a late "trust" is honoured.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::tunneling::Currentness;

/// Live from creation until [`JobFlag::end`]; never live again afterwards.
#[derive(Clone, Debug)]
pub(super) struct JobFlag(Arc<AtomicBool>);

impl JobFlag {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(true)))
    }

    pub fn end(&self) {
        self.0.store(false, Ordering::SeqCst);
    }

    /// Ends the flag when the guard drops: on success, error, timeout, panic or cancellation.
    pub fn end_on_drop(&self) -> EndOnDrop {
        EndOnDrop(self.clone())
    }
}

impl Currentness for JobFlag {
    fn is_current(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

pub(super) struct EndOnDrop(JobFlag);

impl Drop for EndOnDrop {
    fn drop(&mut self) {
        self.0.end();
    }
}
