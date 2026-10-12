//! One compare session: two live connections (each with its own SSH tunnel when configured),
//! the shared result budget, and the results of finished pairs.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::OnceCell;

use crate::compare::engine::CancelFlag;
use crate::compare::protocol::{PairId, PairSpec, RunId, Side};
use crate::compare::results::{Budget, Limits, ResultSet};
use crate::runtime::TunnelHandle;
use crate::runtime::tunneling::Currentness;
use crate::store::DocumentStore;

pub(super) struct Connections {
    pub source: Arc<dyn DocumentStore>,
    pub target: Arc<dyn DocumentStore>,
    /// Kept alive for the session; dropping them closes the tunnels.
    pub _tunnels: Vec<Box<dyn TunnelHandle>>,
}

impl Connections {
    pub fn store(&self, side: Side) -> Arc<dyn DocumentStore> {
        match side {
            Side::Source => self.source.clone(),
            Side::Target => self.target.clone(),
        }
    }
}

// Filled in by the runner (Task 8); the fields are not read before then.
#[allow(dead_code)]
pub(super) struct ActiveRun {
    pub id: RunId,
    pub cancel: CancelFlag,
}

/// A finished pair's results plus what is needed to look a row up again.
#[allow(dead_code)] // read by the runner (Task 8)
pub(super) struct StoredPair {
    pub spec: PairSpec,
    pub results: ResultSet,
}

pub(crate) struct Session {
    /// False once the session was closed; also answers "is this job still wanted".
    pub open: AtomicBool,
    pub(super) connections: OnceCell<Connections>,
    pub budget: Budget,
    pub(super) pairs: Mutex<HashMap<PairId, StoredPair>>,
    pub(super) run: Mutex<Option<ActiveRun>>,
}

impl Session {
    pub fn new(limits: Limits) -> Arc<Self> {
        Arc::new(Self {
            open: AtomicBool::new(true),
            connections: OnceCell::new(),
            budget: Budget::new(limits),
            pairs: Mutex::new(HashMap::new()),
            run: Mutex::new(None),
        })
    }

    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }

    /// Marks the session closed, cancels any run and drops every stored result.
    pub fn close(&self) {
        self.open.store(false, Ordering::SeqCst);
        if let Some(run) = self.run.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            run.cancel.cancel();
        }
        self.pairs.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

impl Currentness for Session {
    fn is_current(&self) -> bool {
        self.is_open()
    }
}

/// The sessions of one runtime.
#[derive(Default)]
pub(crate) struct Registry {
    sessions: Mutex<HashMap<u64, Arc<Session>>>,
}

impl Registry {
    /// Creates the session record synchronously (so a `Close` that follows is ordered after it).
    pub fn create(&self, id: u64, limits: Limits) -> Arc<Session> {
        let session = Session::new(limits);
        let replaced = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, session.clone());
        if let Some(old) = replaced {
            old.close();
        }
        session
    }

    pub fn get(&self, id: u64) -> Option<Arc<Session>> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .cloned()
    }

    /// Removes `id` only if it still maps to `session` (a replacement stays untouched).
    pub fn remove_exact(&self, id: u64, session: &Arc<Session>) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if sessions.get(&id).is_some_and(|s| Arc::ptr_eq(s, session)) {
            sessions.remove(&id);
        }
        drop(sessions);
        session.close();
    }

    pub fn remove(&self, id: u64) {
        let removed = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        if let Some(session) = removed {
            session.close();
        }
    }
}
