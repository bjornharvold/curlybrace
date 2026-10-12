//! One compare session: two live connections (each with its own SSH tunnel when configured),
//! the shared result budget, and the results of finished pairs.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::OnceCell;

use crate::compare::engine::CancelFlag;
use crate::compare::protocol::{CompareEvent, PairId, PairSpec, RunId, SessionId, Side};
use crate::compare::results::{Budget, Limits, ResultSet};
use crate::runtime::TunnelHandle;
use crate::runtime::tunneling::Currentness;
use crate::store::DocumentStore;

use super::{Events, Repaint};
use crate::runtime::Event;

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

pub(super) struct ActiveRun {
    pub id: RunId,
    pub cancel: CancelFlag,
}

/// A finished pair's results plus what is needed to look a row up again.
pub(super) struct StoredPair {
    pub spec: PairSpec,
    pub results: ResultSet,
}

pub(crate) struct Session {
    /// True until the session is closed. Held across "is it open" + "send" (see
    /// [`Session::emit_if_open`]) so a `Close` can never fall between the check and the send:
    /// once `close()` returned, nothing more is emitted for this session.
    open: Mutex<bool>,
    pub(super) connections: OnceCell<Connections>,
    pub budget: Budget,
    pub(super) pairs: Mutex<HashMap<PairId, StoredPair>>,
    pub(super) run: Mutex<Option<ActiveRun>>,
}

impl Session {
    pub fn new(limits: Limits) -> Arc<Self> {
        Arc::new(Self {
            open: Mutex::new(true),
            connections: OnceCell::new(),
            budget: Budget::new(limits),
            pairs: Mutex::new(HashMap::new()),
            run: Mutex::new(None),
        })
    }

    pub fn is_open(&self) -> bool {
        *self.open.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Marks the session closed, cancels any run and drops every stored result.
    pub fn close(&self) {
        *self.open.lock().unwrap_or_else(|e| e.into_inner()) = false;
        if let Some(run) = self.run.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            run.cancel.cancel();
        }
        self.pairs.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// Sends `event` unless the session is closed; the check and the (non-blocking, unbounded)
    /// send happen under one lock. Returns true when the event was sent.
    pub fn emit_if_open(&self, events: &Events, repaint: &Repaint, event: CompareEvent) -> bool {
        let sent = {
            let open = self.open.lock().unwrap_or_else(|e| e.into_inner());
            *open && events.send(Event::Compare(event)).is_ok()
        };
        if sent {
            repaint();
        }
        sent
    }

    /// Stores the connections and announces `Opened`, atomically with respect to `close()`.
    /// A closed session keeps nothing: the connections are dropped and nothing is sent.
    pub(super) fn finish_open(
        &self,
        connections: Connections,
        id: SessionId,
        events: &Events,
        repaint: &Repaint,
    ) -> bool {
        let sent = {
            let open = self.open.lock().unwrap_or_else(|e| e.into_inner());
            *open
                && self.connections.set(connections).is_ok()
                && events
                    .send(Event::Compare(CompareEvent::Opened { session: id }))
                    .is_ok()
        };
        if sent {
            repaint();
        }
        sent
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
