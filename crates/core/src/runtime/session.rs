//! One open connection: its store, tunnel and current epoch.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use super::tunneling::TunnelHandle;
use crate::store::DocumentStore;

/// Identifies one open connection. For a connection it is the `ConnectionConfig::id`.
pub type SessionId = uuid::Uuid;

/// The session of a store handed to `spawn_runtime_with_store` (tests, embedding).
pub const EMBEDDED_SESSION: SessionId = SessionId::nil();

/// One open connection: its store, tunnel and current epoch.
pub(crate) struct Session {
    store: RwLock<Option<Arc<dyn DocumentStore>>>,
    /// Kept alive next to the store; cleared with it on every new epoch.
    tunnel: Mutex<Option<Box<dyn TunnelHandle>>>,
    /// Set synchronously by the dispatcher on every Connect/Disconnect of this session, so the
    /// order of *requests* (not of completion) decides which one wins.
    generation: AtomicU64,
    /// Orders epoch bumps against the final check-and-send of data answers: `begin` holds the
    /// WRITE side while it bumps; a data answer holds the READ side from its last `is_current`
    /// check until it is sent, so a bump can never fall in between. This is a std lock and is
    /// never held across an `.await` (see `handlers::send_answer`).
    pub(super) epoch_lock: RwLock<()>,
}

impl Session {
    pub(super) fn new(generation: u64, store: Option<Arc<dyn DocumentStore>>) -> Self {
        Self {
            store: RwLock::new(store),
            tunnel: Mutex::new(None),
            generation: AtomicU64::new(generation),
            epoch_lock: RwLock::new(()),
        }
    }

    pub fn current_store(&self) -> Option<Arc<dyn DocumentStore>> {
        // The guarded value is a plain Option, so a poisoned lock is still valid.
        self.store.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The epoch this session is currently in.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// True while no newer Connect/Disconnect of this session has been requested since `generation`.
    pub fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }

    /// Moves to `generation` and drops the installed store and tunnel.
    pub(super) fn begin(&self, generation: u64) {
        let _bump = self.epoch_lock.write().unwrap_or_else(|e| e.into_inner());
        self.generation.store(generation, Ordering::SeqCst);
        let mut slot = self.store.write().unwrap_or_else(|e| e.into_inner());
        *slot = None;
        let old = self.tunnel.lock().unwrap_or_else(|e| e.into_inner()).take();
        drop(slot);
        drop(old);
    }

    /// Installs `store` and its tunnel only if `generation` is still the latest
    /// request. When superseded, both are dropped (closing the tunnel).
    pub fn install_if_current(
        &self,
        generation: u64,
        store: Arc<dyn DocumentStore>,
        tunnel: Option<Box<dyn TunnelHandle>>,
    ) -> bool {
        let mut slot = self.store.write().unwrap_or_else(|e| e.into_inner());
        if self.generation.load(Ordering::SeqCst) != generation {
            return false;
        }
        *slot = Some(store);
        let previous = std::mem::replace(
            &mut *self.tunnel.lock().unwrap_or_else(|e| e.into_inner()),
            tunnel,
        );
        drop(slot);
        drop(previous);
        true
    }
}
