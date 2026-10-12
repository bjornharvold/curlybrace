//! Async bridge between the (synchronous) UI and the document store.
//!
//! The UI sends [`Command`]s and drains [`Event`]s. Every command yields
//! EXACTLY ONE event, followed by one `repaint()` call, on every code path:
//! success, error, timeout and even a panic inside a handler.
//!
//! The spawn functions must be called from within a Tokio runtime (they use
//! [`tokio::runtime::Handle::current`]); the binary creates and owns that
//! runtime. Commands run concurrently, so a UI that needs ordering (e.g.
//! `Connect` before `Find`) must wait for the corresponding event first.

mod handlers;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use bson::{Bson, Document};
use tokio::sync::RwLock;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::connection::ConnectionConfig;
use crate::error::CoreError;
use crate::secrets::SecretStore;
use crate::store::{DocumentStore, Namespace, Page, Query};

pub use handlers::open_tunnel_if_configured;

/// Upper bound for any single command (including connect).
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Runtime tunables; the timeout is overridable so tests stay fast.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeConfig {
    pub command_timeout: Duration,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            command_timeout: COMMAND_TIMEOUT,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Command {
    Connect(ConnectionConfig),
    Disconnect,
    ListDatabases,
    ListCollections {
        db: String,
    },
    Find {
        ns: Namespace,
        query: Query,
    },
    Insert {
        ns: Namespace,
        json: String,
    },
    Replace {
        ns: Namespace,
        id: Bson,
        json: String,
    },
    Delete {
        ns: Namespace,
        id: Bson,
    },
    Clone {
        ns: Namespace,
        source: Document,
    },
}

impl Command {
    /// i18n key suffix reported in [`Event::Failed`] for this command.
    pub fn context(&self) -> &'static str {
        match self {
            Self::Connect(_) => "connect",
            Self::Disconnect => "disconnect",
            Self::ListDatabases => "list_databases",
            Self::ListCollections { .. } => "list_collections",
            Self::Find { .. } => "find",
            Self::Insert { .. } => "insert",
            Self::Replace { .. } => "replace",
            Self::Delete { .. } => "delete",
            Self::Clone { .. } => "clone",
        }
    }
}

// Events are moved through a channel once; boxing `Query` would only add noise
// for the UI, which pattern-matches on the brief's public shape.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Connected,
    Disconnected,
    Databases(Vec<String>),
    Collections {
        db: String,
        names: Vec<String>,
    },
    /// Carries the originating `ns`/`query` so the UI can drop stale results.
    Results {
        ns: Namespace,
        query: Query,
        page: Page,
    },
    /// The UI re-runs its current query.
    Mutated {
        ns: Namespace,
    },
    /// `context` is an i18n key suffix, e.g. `"connect"` or `"find"`.
    Failed {
        context: &'static str,
        error: CoreError,
    },
}

pub struct RuntimeHandle {
    pub tx: UnboundedSender<Command>,
    pub rx: UnboundedReceiver<Event>,
}

pub type Repaint = Arc<dyn Fn() + Send + Sync>;

/// State shared by all handler tasks.
pub(crate) struct Shared {
    pub store: RwLock<Option<Arc<dyn DocumentStore>>>,
    pub secrets: Option<Arc<dyn SecretStore>>,
    pub config: RuntimeConfig,
}

/// Starts a runtime with no store; `Command::Connect` installs one.
pub fn spawn_runtime(secrets: Arc<dyn SecretStore>, repaint: Repaint) -> RuntimeHandle {
    spawn_runtime_with_config(Some(secrets), None, repaint, RuntimeConfig::default())
}

/// Starts a runtime that is already connected to `store` (tests, embedding).
pub fn spawn_runtime_with_store(store: Arc<dyn DocumentStore>, repaint: Repaint) -> RuntimeHandle {
    spawn_runtime_with_config(None, Some(store), repaint, RuntimeConfig::default())
}

pub fn spawn_runtime_with_config(
    secrets: Option<Arc<dyn SecretStore>>,
    store: Option<Arc<dyn DocumentStore>>,
    repaint: Repaint,
    config: RuntimeConfig,
) -> RuntimeHandle {
    let (tx, mut commands) = unbounded_channel::<Command>();
    let (events, rx) = unbounded_channel::<Event>();
    let shared = Arc::new(Shared {
        store: RwLock::new(store),
        secrets,
        config,
    });
    tokio::runtime::Handle::current().spawn(async move {
        while let Some(command) = commands.recv().await {
            dispatch(shared.clone(), events.clone(), repaint.clone(), command);
        }
    });
    RuntimeHandle { tx, rx }
}

/// Runs one command in its own task. The work runs in an inner task so a
/// panic surfaces as a `JoinError` here and still produces one `Failed` event.
fn dispatch(
    shared: Arc<Shared>,
    events: UnboundedSender<Event>,
    repaint: Repaint,
    command: Command,
) {
    let context = command.context();
    tokio::spawn(async move {
        let event = match tokio::spawn(handlers::execute(shared, command)).await {
            Ok(event) => event,
            Err(_) => Event::Failed {
                context,
                error: CoreError::Database("internal error".into()),
            },
        };
        if events.send(event).is_ok() {
            repaint();
        }
    });
}
