//! Async bridge between the (synchronous) UI and the document store.
//!
//! The UI sends [`Command`]s and drains [`Event`]s. Every command yields
//! EXACTLY ONE event, followed by one `repaint()` call, on every code path:
//! success, error, timeout and even a panic inside a handler. The one exception is
//! `Command::Compare`: compare sessions are long-lived and independent of the connections,
//! so `Cancel`, `Discard` and `Close` have no answer of their own and `Start` answers with a
//! stream of events (one `PairDone` per pair, then one `RunDone`). The answer rule per compare
//! command is documented in the `compare` module.
//!
//! Every connection is its own session (`Connect(cfg)` opens session `cfg.id`); data commands and
//! session events carry the session they belong to, and all epochs come from one counter.
//!
//! The spawn functions must be called from within a Tokio runtime (they use
//! [`tokio::runtime::Handle::current`]); the binary creates and owns that
//! runtime. Commands run concurrently, so a UI that needs ordering (e.g.
//! `Connect` before `Find`) must wait for the corresponding event first.

mod compare;
#[cfg(test)]
mod epoch_tests;
mod handlers;
mod job_flag;
#[cfg(test)]
mod lost_server_tests;
#[cfg(test)]
mod prompt_gate_tests;
mod read_only_registry;
#[cfg(test)]
mod read_only_tests;
mod session;
#[cfg(test)]
mod session_tests;
#[cfg(test)]
mod test_connection_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tunnel_tests;
mod tunneling;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use bson::{Bson, Document};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::connection::ConnectionConfig;
use crate::error::{CoreError, Result};
use crate::secrets::SecretStore;
use crate::store::{DocumentStore, Namespace, Page, Query, Topology};
use crate::tunnel::{HostKeyPrompt, RejectUnknownHostKeys};
use session::Session;

pub use session::{EMBEDDED_SESSION, SessionId};
/// What `RuntimeHandle::rx.try_recv()` reports, so callers can tell "no event yet" from "the
/// runtime is gone" without depending on tokio themselves.
pub use tokio::sync::mpsc::error::TryRecvError;
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

/// Upper bound for the reachability ping that follows a suspect failure of a data command.
pub const PING_TIMEOUT: Duration = Duration::from_secs(3);

/// Upper bound for the duplicate-key aggregation of a compare pair (it may scan a whole
/// collection).
pub const DUPLICATE_TIMEOUT: Duration = Duration::from_secs(120);

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
    /// Bound for the ping that settles a lost connection or a timed-out command.
    pub ping_timeout: Duration,
    /// Bound for a Connect, including an SSH host-key prompt waiting for the user.
    pub connect_timeout: Duration,
    /// Bound for the duplicate-key search of a compare pair.
    pub duplicate_timeout: Duration,
    /// `None` when the config directory is unknown: SSH connects then fail closed.
    pub known_hosts_path: Option<PathBuf>,
    pub host_key_prompt: Arc<dyn HostKeyPrompt>,
    pub tunnel_opener: Arc<dyn TunnelOpener>,
    /// Result budget and preflight thresholds of every compare session.
    pub compare_limits: crate::compare::results::Limits,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            command_timeout: COMMAND_TIMEOUT,
            ping_timeout: PING_TIMEOUT,
            connect_timeout: CONNECT_TIMEOUT,
            duplicate_timeout: DUPLICATE_TIMEOUT,
            known_hosts_path: default_known_hosts_path(),
            host_key_prompt: Arc::new(RejectUnknownHostKeys),
            tunnel_opener: Arc::new(SshTunnelOpener),
            compare_limits: Default::default(),
        }
    }
}

impl std::fmt::Debug for RuntimeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeConfig")
            .field("command_timeout", &self.command_timeout)
            .field("connect_timeout", &self.connect_timeout)
            .field("duplicate_timeout", &self.duplicate_timeout)
            .field("known_hosts_path", &self.known_hosts_path)
            .finish_non_exhaustive()
    }
}

/// Which password or SSH secret a connection test uses. Debug redacts through
/// [`SecretString`](crate::secrets::SecretString).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum PasswordChoice {
    /// Nothing typed: use the password stored for the connection, if any.
    #[default]
    Stored,
    /// The user chose to clear the saved password: connect without one, ignoring the stored one.
    Without,
    /// A password typed into the form for this test only.
    Typed(crate::secrets::SecretString),
}

/// Secrets typed into the connection form for one test. `Stored` on either choice means "use the
/// stored one".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TestSecrets {
    pub password: PasswordChoice,
    pub ssh_secret: PasswordChoice,
}

/// What a successful connection test learned.
#[derive(Clone, Debug, PartialEq)]
pub struct TestReport {
    pub server_version: Option<String>,
    pub tls: bool,
    pub authenticated: bool,
}

// Commands are short-lived, moved once into a channel; boxing would ripple through every call site.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum Command {
    /// Opens the session `cfg.id`, replacing that session's earlier connection.
    Connect(ConnectionConfig),
    Disconnect {
        session: SessionId,
    },
    ListDatabases {
        session: SessionId,
    },
    ListCollections {
        session: SessionId,
        db: String,
    },
    Find {
        session: SessionId,
        ns: Namespace,
        query: Query,
    },
    Insert {
        session: SessionId,
        ns: Namespace,
        json: String,
    },
    Replace {
        session: SessionId,
        ns: Namespace,
        id: Bson,
        json: String,
    },
    Delete {
        session: SessionId,
        ns: Namespace,
        id: Bson,
    },
    Clone {
        session: SessionId,
        ns: Namespace,
        source: Document,
    },
    /// A data-compare session command; see [`compare`] for how it is answered.
    Compare(crate::compare::protocol::CompareCommand),
    /// Opens a throw-away connection and reports on it with [`Event::TestConnection`]. It never
    /// begins an epoch and never touches the installed store or tunnel.
    TestConnection {
        request_id: u64,
        cfg: ConnectionConfig,
        secrets: TestSecrets,
    },
    /// Makes every open store of the connection refuse writes from now on. Independent of the
    /// sessions like `TestConnection`: no epoch, no answer event, and an unknown id does nothing.
    LockReadOnly {
        connection: uuid::Uuid,
    },
}

impl Command {
    /// Whether this command changes data: a lost answer may hide a change the server applied.
    pub fn is_write(&self) -> bool {
        match self {
            Self::Insert { .. }
            | Self::Replace { .. }
            | Self::Delete { .. }
            | Self::Clone { .. } => true,
            Self::Connect(_)
            | Self::Disconnect { .. }
            | Self::ListDatabases { .. }
            | Self::ListCollections { .. }
            | Self::Find { .. }
            | Self::Compare(_)
            | Self::TestConnection { .. }
            | Self::LockReadOnly { .. } => false,
        }
    }

    /// i18n key suffix reported in [`Event::Failed`] for this command.
    pub fn context(&self) -> &'static str {
        match self {
            Self::Connect(_) => "connect",
            Self::Disconnect { .. } => "disconnect",
            Self::ListDatabases { .. } => "list_databases",
            Self::ListCollections { .. } => "list_collections",
            Self::Find { .. } => "find",
            Self::Insert { .. } => "insert",
            Self::Replace { .. } => "replace",
            Self::Delete { .. } => "delete",
            Self::Clone { .. } => "clone",
            Self::Compare(_) => "compare",
            Self::TestConnection { .. } => "test_connection",
            Self::LockReadOnly { .. } => "lock_read_only",
        }
    }

    /// The session this command addresses; `None` for compare, connection tests and read-only
    /// locks, which are independent of sessions.
    pub fn session(&self) -> Option<SessionId> {
        match self {
            Self::Connect(cfg) => Some(cfg.id),
            Self::Disconnect { session }
            | Self::ListDatabases { session }
            | Self::ListCollections { session, .. }
            | Self::Find { session, .. }
            | Self::Insert { session, .. }
            | Self::Replace { session, .. }
            | Self::Delete { session, .. }
            | Self::Clone { session, .. } => Some(*session),
            Self::Compare(_) | Self::TestConnection { .. } | Self::LockReadOnly { .. } => None,
        }
    }
}

// Events are moved through a channel once; boxing `Query` would only add noise
// for the UI, which pattern-matches on the brief's public shape.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The connection requested by the `Connect` whose epoch is `epoch` is installed.
    /// Epochs come from one counter shared by all sessions and grow with every Connect/Disconnect
    /// request, so a receiver can ignore an outcome older than the latest one it requested. Only a
    /// still-current connect reports `Connected` or `ConnectFailed`; a replaced one reports
    /// `Failed { error: Superseded }`.
    Connected {
        session: SessionId,
        epoch: u64,
        /// How the server is deployed; `None` when it could not be read (never fails a connect).
        topology: Option<Topology>,
    },
    /// The `Disconnect` with this `epoch` took effect.
    Disconnected { session: SessionId, epoch: u64 },
    /// The still-current connect with this `epoch` failed for real. A replaced connect never
    /// reports this: it answers `Failed { context: "connect", error: Superseded }`.
    ConnectFailed {
        session: SessionId,
        epoch: u64,
        error: CoreError,
    },
    Databases {
        session: SessionId,
        names: Vec<String>,
    },
    Collections {
        session: SessionId,
        db: String,
        names: Vec<String>,
    },
    /// Carries the originating `ns`/`query` so the UI can drop stale results.
    Results {
        session: SessionId,
        ns: Namespace,
        query: Query,
        page: Page,
    },
    /// The UI re-runs its current query.
    Mutated { session: SessionId, ns: Namespace },
    /// `context` is an i18n key suffix, e.g. `"connect"` or `"find"`.
    Failed {
        session: SessionId,
        context: &'static str,
        error: CoreError,
    },
    /// An answer or progress report of a compare session (see [`compare`]).
    Compare(crate::compare::protocol::CompareEvent),
    /// The answer to `Command::TestConnection { request_id, .. }`; carries no secret.
    TestConnection {
        request_id: u64,
        outcome: std::result::Result<TestReport, CoreError>,
    },
}

/// A config whose session is [`EMBEDDED_SESSION`], so a test that connects and then sends data
/// commands keeps talking to the one session it always used.
#[cfg(test)]
fn embedded_config(name: &str, uri: &str) -> ConnectionConfig {
    ConnectionConfig {
        id: EMBEDDED_SESSION,
        ..ConnectionConfig::new(name, uri)
    }
}

impl Event {
    /// The session this event belongs to; `None` for compare and connection-test events.
    pub fn session(&self) -> Option<SessionId> {
        match self {
            Self::Connected { session, .. }
            | Self::Disconnected { session, .. }
            | Self::ConnectFailed { session, .. }
            | Self::Databases { session, .. }
            | Self::Collections { session, .. }
            | Self::Results { session, .. }
            | Self::Mutated { session, .. }
            | Self::Failed { session, .. } => Some(*session),
            Self::Compare(_) | Self::TestConnection { .. } => None,
        }
    }
}

pub struct RuntimeHandle {
    pub tx: UnboundedSender<Command>,
    pub rx: UnboundedReceiver<Event>,
}

pub type Repaint = Arc<dyn Fn() + Send + Sync>;

/// State shared by all handler tasks.
pub(crate) struct Shared {
    /// Open connections by session id; a session is removed on `Disconnect`.
    sessions: RwLock<HashMap<SessionId, Arc<Session>>>,
    /// The global epoch counter: every Connect/Disconnect, whichever session it names, takes the
    /// next number, so epochs never repeat and a late event of a closed session can never look
    /// current to a new one.
    next_epoch: AtomicU64,
    pub secrets: Option<Arc<dyn SecretStore>>,
    pub connector: Arc<dyn Connector>,
    pub config: RuntimeConfig,
    /// Compare sessions: independent of the connections above.
    compare: compare::Registry,
    /// The flag of the newest connection test: a newer test ends it, so at most one test's
    /// host-key question is live. Locked only briefly, never across an await.
    latest_test: std::sync::Mutex<Option<job_flag::JobFlag>>,
    /// The read-only flags of the open stores, so a lock reaches stores that are already open.
    pub(crate) read_only: read_only_registry::ReadOnlyRegistry,
}

impl Shared {
    /// Gets or creates the session and starts its next epoch. Called only from the dispatcher
    /// (single-threaded), so request order decides.
    fn begin_session_epoch(&self, id: SessionId) -> (Arc<Session>, u64) {
        let generation = self.next_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let session = {
            let mut map = self.sessions.write().unwrap_or_else(|e| e.into_inner());
            map.entry(id)
                .or_insert_with(|| Arc::new(Session::new(generation, None)))
                .clone()
        };
        session.begin(generation);
        (session, generation)
    }

    /// Starts the next epoch for `id`, drops its store and tunnel and removes it from the map.
    /// Tasks that still hold the `Arc<Session>` see a stale generation.
    fn end_session(&self, id: SessionId) -> u64 {
        let generation = self.next_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let removed = self
            .sessions
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        if let Some(session) = removed {
            session.begin(generation);
        }
        generation
    }

    fn session(&self, id: SessionId) -> Option<Arc<Session>> {
        self.sessions
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&id)
            .cloned()
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
    // A store handed in at spawn time is the embedded session, in generation 0.
    let sessions: HashMap<SessionId, Arc<Session>> = store
        .map(|store| (EMBEDDED_SESSION, Arc::new(Session::new(0, Some(store)))))
        .into_iter()
        .collect();
    let shared = Arc::new(Shared {
        sessions: RwLock::new(sessions),
        next_epoch: AtomicU64::new(0),
        secrets,
        connector,
        config,
        compare: Default::default(),
        latest_test: Default::default(),
        read_only: Default::default(),
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
    if let Command::Compare(c) = command {
        compare::dispatch(shared, events, repaint, c);
        return;
    }
    // Like Compare: independent of the connection epoch (no begin_epoch, no generation read).
    if let Command::TestConnection {
        request_id,
        cfg,
        secrets,
    } = command
    {
        handlers::spawn_test(shared, events, repaint, request_id, cfg, secrets);
        return;
    }
    // Synchronous, so a write sent after the lock is already refused. No session, no epoch, no
    // answer: the caller learns of it from the next refused write.
    if let Command::LockReadOnly { connection } = command {
        shared.read_only.lock(connection);
        return;
    }
    // Synchronous, so request order decides: ending the session bumps its epoch before any later
    // command is dispatched. The session is gone afterwards, so the answer needs no `Session`.
    if let Command::Disconnect { session: id } = command {
        let epoch = shared.end_session(id);
        if events
            .send(Event::Disconnected { session: id, epoch })
            .is_ok()
        {
            repaint();
        }
        return;
    }
    let context = command.context();
    // Synchronous, so request order decides which connection wins.
    let (target, work) = match command {
        Command::Connect(cfg) => {
            let (session, generation) = shared.begin_session_epoch(cfg.id);
            // Synchronous like the epoch, so a LockReadOnly sent after this Connect reaches the
            // store it installs even while the connect is still in flight. A connect that fails
            // or is superseded just drops the flag.
            let read_only = shared.read_only.attach(cfg.id, cfg.read_only);
            let target = Target::new(cfg.id, session, generation, false);
            (
                target,
                handlers::Work::Connect {
                    cfg: Box::new(cfg),
                    read_only,
                },
            )
        }
        data => {
            let id = data.session().expect("a data command names its session");
            let Some(session) = shared.session(id) else {
                let unknown = Event::Failed {
                    session: id,
                    context,
                    error: CoreError::Unsupported("not connected".into()),
                };
                if events.send(unknown).is_ok() {
                    repaint();
                }
                return;
            };
            let generation = session.generation();
            let target = Target::new(id, session, generation, true);
            (target, handlers::Work::Data(Box::new(data)))
        }
    };
    tokio::spawn(async move {
        let Target {
            id,
            session,
            generation,
            is_data,
        } = target;
        let work = handlers::execute(shared, session.clone(), id, work, generation);
        let event = match tokio::spawn(work).await {
            Ok(event) => event,
            Err(_) => handlers::failure_event(
                id,
                context,
                generation,
                handlers::gate(
                    &session,
                    generation,
                    CoreError::Database("internal error".into()),
                ),
            ),
        };
        if handlers::send_answer(&session, id, generation, is_data, context, event, &events) {
            repaint();
        }
    });
}

/// The session a dispatched command runs against, with the epoch it was dispatched in.
struct Target {
    id: SessionId,
    session: Arc<Session>,
    generation: u64,
    is_data: bool,
}

impl Target {
    fn new(id: SessionId, session: Arc<Session>, generation: u64, is_data: bool) -> Self {
        Self {
            id,
            session,
            generation,
            is_data,
        }
    }
}
