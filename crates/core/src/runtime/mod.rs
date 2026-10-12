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
#[cfg(test)]
mod tunnel_tests;
mod tunneling;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use bson::{Bson, Document};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::connection::ConnectionConfig;
use crate::error::{CoreError, Result};
use crate::secrets::SecretStore;
use crate::store::{DocumentStore, Namespace, Page, Query};
use crate::tunnel::{HostKeyPrompt, RejectUnknownHostKeys};

pub use tunneling::{
    SshTunnelOpener, TunnelHandle, TunnelOpener, TunnelRequest, default_known_hosts_path,
};

/// Opens a store for a connection config. Injectable so tests need no server.
#[async_trait::async_trait]
pub trait Connector: Send + Sync {
    async fn connect(
        &self,
        cfg: &ConnectionConfig,
        password: Option<&str>,
        tunnel: Option<SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>>;
}

/// Default connector: the real driver.
pub struct MongoConnector;

#[async_trait::async_trait]
impl Connector for MongoConnector {
    async fn connect(
        &self,
        cfg: &ConnectionConfig,
        password: Option<&str>,
        tunnel: Option<SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        let client = crate::client::connect(cfg, password, tunnel).await?;
        Ok(Arc::new(crate::mongo_store::MongoStore::new(client)))
    }
}

/// Upper bound for any single command (including connect).
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Upper bound for a Connect (connector, SSH tunnel and any host-key prompt the user must answer).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(150);

/// Runtime tunables; the timeouts are overridable so tests stay fast.
///
/// SSH tunnels: `known_hosts_path` is the app-owned host-key file and
/// `host_key_prompt` decides about UNKNOWN host keys. The default prompt is
/// [`RejectUnknownHostKeys`] (safe); the UI (Task 16) supplies an interactive
/// one. The prompt is synchronous and is called via `spawn_blocking`, so the
/// UI side may block on a channel until the user answers. Keep
/// `connect_timeout` in mind: it bounds the whole connect, prompt included, while
/// `command_timeout` bounds data commands only.
#[derive(Clone)]
pub struct RuntimeConfig {
    /// Bound for each data command (find, insert, ...).
    pub command_timeout: Duration,
    /// Bound for a Connect, including an SSH host-key prompt waiting for the user.
    pub connect_timeout: Duration,
    /// `None` when the config directory is unknown: SSH connects then fail closed.
    pub known_hosts_path: Option<PathBuf>,
    pub host_key_prompt: Arc<dyn HostKeyPrompt>,
    pub tunnel_opener: Arc<dyn TunnelOpener>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            command_timeout: COMMAND_TIMEOUT,
            connect_timeout: CONNECT_TIMEOUT,
            known_hosts_path: default_known_hosts_path(),
            host_key_prompt: Arc::new(RejectUnknownHostKeys),
            tunnel_opener: Arc::new(SshTunnelOpener),
        }
    }
}

impl std::fmt::Debug for RuntimeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeConfig")
            .field("command_timeout", &self.command_timeout)
            .field("connect_timeout", &self.connect_timeout)
            .field("known_hosts_path", &self.known_hosts_path)
            .finish_non_exhaustive()
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
    /// The connection requested by the `Connect` whose epoch is `epoch` is installed.
    /// Epochs grow with every Connect/Disconnect request, so a receiver can ignore an outcome
    /// older than the latest one it requested. Only a still-current connect reports
    /// `Connected` or `ConnectFailed`; a replaced one reports `Failed { error: Superseded }`.
    Connected {
        epoch: u64,
    },
    /// The `Disconnect` with this `epoch` took effect.
    Disconnected {
        epoch: u64,
    },
    /// The still-current connect with this `epoch` failed for real. A replaced connect never
    /// reports this: it answers `Failed { context: "connect", error: Superseded }`.
    ConnectFailed {
        epoch: u64,
        error: CoreError,
    },
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
    store: RwLock<Option<Arc<dyn DocumentStore>>>,
    /// Kept alive next to the store; cleared with it on every new epoch.
    tunnel: Mutex<Option<Box<dyn TunnelHandle>>>,
    /// Bumped synchronously by the dispatcher on every Connect/Disconnect, so
    /// the order of *requests* (not of completion) decides which one wins.
    generation: AtomicU64,
    pub secrets: Option<Arc<dyn SecretStore>>,
    pub connector: Arc<dyn Connector>,
    pub config: RuntimeConfig,
}

impl Shared {
    pub fn current_store(&self) -> Option<Arc<dyn DocumentStore>> {
        // The guarded value is a plain Option, so a poisoned lock is still valid.
        self.store.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// True while no newer Connect/Disconnect has been requested since `generation`.
    pub fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }

    /// Starts a new connection epoch and drops the installed store and tunnel.
    fn begin_epoch(&self) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let mut slot = self.store.write().unwrap_or_else(|e| e.into_inner());
        *slot = None;
        let old = self.tunnel.lock().unwrap_or_else(|e| e.into_inner()).take();
        drop(slot);
        drop(old);
        generation
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
    spawn_runtime_with_connector(secrets, store, Arc::new(MongoConnector), repaint, config)
}

pub fn spawn_runtime_with_connector(
    secrets: Option<Arc<dyn SecretStore>>,
    store: Option<Arc<dyn DocumentStore>>,
    connector: Arc<dyn Connector>,
    repaint: Repaint,
    config: RuntimeConfig,
) -> RuntimeHandle {
    let (tx, mut commands) = unbounded_channel::<Command>();
    let (events, rx) = unbounded_channel::<Event>();
    let shared = Arc::new(Shared {
        store: RwLock::new(store),
        tunnel: Mutex::new(None),
        generation: AtomicU64::new(0),
        secrets,
        connector,
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
    // Synchronous, so request order decides which connection wins.
    let generation = match command {
        Command::Connect(_) | Command::Disconnect => shared.begin_epoch(),
        _ => shared.generation.load(Ordering::SeqCst),
    };
    tokio::spawn(async move {
        let guard = shared.clone();
        let event = match tokio::spawn(handlers::execute(shared, command, generation)).await {
            Ok(event) => event,
            Err(_) => handlers::failure_event(
                context,
                generation,
                handlers::gate(
                    &guard,
                    generation,
                    CoreError::Database("internal error".into()),
                ),
            ),
        };
        if events.send(event).is_ok() {
            repaint();
        }
    });
}
