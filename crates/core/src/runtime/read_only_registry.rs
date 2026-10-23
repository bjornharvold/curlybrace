//! The live read-only flags of the open stores, by connection id.
//!
//! Every connect gets its own fresh flag, so reconnecting with the read-only box unticked yields
//! a writable store again. A flag is only ever raised, never lowered, and only a flag some store
//! still holds is kept: the registry remembers them by `Weak`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use uuid::Uuid;

#[derive(Default)]
pub(crate) struct ReadOnlyRegistry {
    flags: Mutex<HashMap<Uuid, Vec<Weak<AtomicBool>>>>,
}

impl ReadOnlyRegistry {
    /// A fresh flag for a store opened for `connection`, starting at `initial`.
    pub(crate) fn attach(&self, connection: Uuid, initial: bool) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(initial));
        let mut flags = self.lock_map();
        flags.retain(|_, list| {
            list.retain(|w| w.strong_count() > 0);
            !list.is_empty()
        });
        flags
            .entry(connection)
            .or_default()
            .push(Arc::downgrade(&flag));
        flag
    }

    /// Makes every live store of `connection` refuse writes from now on. Unknown ids do nothing.
    pub(crate) fn lock(&self, connection: Uuid) {
        let flags = self.lock_map();
        for flag in flags.get(&connection).into_iter().flatten() {
            if let Some(flag) = flag.upgrade() {
                flag.store(true, Ordering::SeqCst);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn live_count(&self, connection: Uuid) -> usize {
        self.lock_map()
            .get(&connection)
            .map_or(0, |l| l.iter().filter(|w| w.strong_count() > 0).count())
    }

    fn lock_map(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, Vec<Weak<AtomicBool>>>> {
        self.flags.lock().unwrap_or_else(|e| e.into_inner())
    }
}
