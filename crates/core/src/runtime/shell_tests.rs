//! `ShellRun` / `ShellCancel` through the runtime, with the fake `mongosh` of `shell::fake`
//! (no real `mongosh` or server).

use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use uuid::Uuid;

use super::tunneling::{TunnelHandle, TunnelOpener, TunnelRequest};
use super::*;
use crate::connection::{SshAuth, SshConfig};
use crate::secrets::{MemorySecretStore, SecretKind, SecretStore, secret_key};
use crate::shell::fake::{Behaviour, FakeMongosh};
use crate::shell::{MASK, Outcome, ShellError, Stream, percent_encode_component};
use crate::test_support::InMemoryStore;

const WAIT: Duration = Duration::from_secs(10);
const QUIET: Duration = Duration::from_millis(400);
const PASSWORD: &str = "p@ss:w/rd";
const TUNNEL: &str = "127.0.0.1:50999";

struct StoreConnector;

#[async_trait]
impl Connector for StoreConnector {
    async fn connect(
        &self,
        _cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        Ok(Arc::new(InMemoryStore::new()))
    }
}

struct LoopbackTunnel;

impl TunnelHandle for LoopbackTunnel {
    fn local_addr(&self) -> SocketAddr {
        TUNNEL.parse().unwrap()
    }
}

struct LoopbackOpener;

#[async_trait]
impl TunnelOpener for LoopbackOpener {
    async fn open(&self, _request: TunnelRequest<'_>) -> Result<Box<dyn TunnelHandle>> {
        Ok(Box::new(LoopbackTunnel))
    }
}

struct Fixture {
    h: RuntimeHandle,
    id: SessionId,
    fake: FakeMongosh,
}

impl Fixture {
    /// A runtime with `cfg` connected (the password stored for it, if any).
    async fn connected(cfg: ConnectionConfig, password: Option<&str>) -> Self {
        let secrets = Arc::new(MemorySecretStore::new());
        if let Some(password) = password {
            secrets
                .set(&secret_key(cfg.id, SecretKind::Password), password)
                .unwrap();
        }
        Self::connected_with(cfg, secrets).await
    }

    async fn connected_with(cfg: ConnectionConfig, secrets: Arc<dyn SecretStore>) -> Self {
        let config = RuntimeConfig {
            tunnel_opener: Arc::new(LoopbackOpener),
            ..RuntimeConfig::default()
        };
        let repaint: Repaint = Arc::new(|| {});
        let mut h = spawn_runtime_with_connector(
            Some(secrets),
            None,
            Arc::new(StoreConnector),
            repaint,
            config,
        );
        let id = cfg.id;
        h.tx.send(Command::Connect(cfg)).unwrap();
        assert!(matches!(next(&mut h).await, Event::Connected { .. }));
        Self {
            h,
            id,
            fake: FakeMongosh::new(Behaviour::Repl),
        }
    }

    async fn plain() -> Self {
        Self::connected(
            ConnectionConfig::new("t", "mongodb://localhost:27017/app"),
            None,
        )
        .await
    }

    fn run(&self, run_id: u64, script: &str) {
        self.run_with(run_id, self.fake.exe.clone(), script);
    }

    fn run_with(&self, run_id: u64, exe: PathBuf, script: &str) {
        self.h
            .tx
            .send(Command::ShellRun {
                session: self.id,
                run_id,
                exe,
                script: script.into(),
            })
            .unwrap();
    }

    fn cancel(&self, run_id: u64) {
        let session = self.id;
        self.h
            .tx
            .send(Command::ShellCancel { session, run_id })
            .unwrap();
    }

    /// Events up to and including the first `ShellFinished` or `ShellRefused`.
    async fn until_done(&mut self) -> Vec<Event> {
        let mut seen = Vec::new();
        loop {
            let event = next(&mut self.h).await;
            let last = matches!(
                event,
                Event::ShellFinished { .. } | Event::ShellRefused { .. }
            );
            seen.push(event);
            if last {
                return seen;
            }
        }
    }

    async fn outcome(&mut self) -> Outcome {
        match self.until_done().await.pop() {
            Some(Event::ShellFinished { outcome, .. }) => outcome,
            other => panic!("expected ShellFinished, got {other:?}"),
        }
    }

    async fn refusal(&mut self) -> ShellRefusal {
        match self.until_done().await.pop() {
            Some(Event::ShellRefused { reason, .. }) => reason,
            other => panic!("expected ShellRefused, got {other:?}"),
        }
    }

    /// Nothing arrives for a while.
    async fn assert_quiet(&mut self) {
        if let Ok(event) = tokio::time::timeout(QUIET, self.h.rx.recv()).await {
            panic!("unexpected event {event:?}");
        }
    }
}

async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("event in time")
        .expect("channel open")
}

fn output_text(events: &[Event], wanted: Stream) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ShellOutput { stream, text, .. } if *stream == wanted => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

async fn wait_for_file(path: &Path) -> String {
    let start = Instant::now();
    while start.elapsed() < WAIT {
        match std::fs::read_to_string(path) {
            Ok(text) if !text.trim().is_empty() => return text,
            _ => {}
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{} never appeared", path.display());
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid.trim()])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn wait_dead(pid: &str) {
    let start = Instant::now();
    while alive(pid) {
        assert!(start.elapsed() < WAIT, "process {pid} is still alive");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

// ---- commands ----

#[test]
fn exhaustive_matches_cover_new_commands() {
    let session = Uuid::new_v4();
    let run = Command::ShellRun {
        session,
        run_id: 7,
        exe: "mongosh".into(),
        script: "x".into(),
    };
    let cancel = Command::ShellCancel { session, run_id: 7 };
    assert!(run.is_write());
    assert!(!cancel.is_write());
    assert_eq!(run.context(), "shell_run");
    assert_eq!(cancel.context(), "shell_cancel");
    assert_eq!(run.session(), Some(session));
    assert_eq!(cancel.session(), Some(session));
    let refused = Event::ShellRefused {
        session,
        run_id: 7,
        reason: ShellRefusal::ReadOnly,
    };
    assert_eq!(refused.session(), Some(session));
}

// ---- runs ----

#[tokio::test]
async fn run_streams_output_then_finishes() {
    let mut f = Fixture::plain().await;
    f.run(1, "hello shell");
    let events = f.until_done().await;
    assert!(output_text(&events, Stream::Stdout).contains("hello shell"));
    assert!(events.iter().all(|e| e.session() == Some(f.id)));
    assert!(events.iter().all(|e| match e {
        Event::ShellOutput { run_id, .. } | Event::ShellFinished { run_id, .. } => *run_id == 1,
        _ => false,
    }));
    assert!(matches!(
        events.last(),
        Some(Event::ShellFinished {
            outcome: Outcome::Exited(0),
            ..
        })
    ));
}

/// The preamble's `passwordPrompt` override reaches the shell: the call is a script error with
/// its message, instead of a prompt that silently reads the rest of the script.
#[tokio::test]
async fn password_prompt_is_a_script_error_not_a_silent_end() {
    let mut f = Fixture::plain().await;
    f.run(1, "before\npasswordPrompt()\nafter");
    let events = f.until_done().await;
    let err = output_text(&events, Stream::Stderr);
    assert_eq!(
        err,
        "Uncaught Error: passwordPrompt() is not available in the shell tab\n"
    );
    assert!(output_text(&events, Stream::Stdout).starts_with("before\n"));
    assert!(matches!(
        events.last(),
        Some(Event::ShellFinished {
            outcome: Outcome::ScriptErrors,
            ..
        })
    ));
}

#[tokio::test]
async fn read_only_connection_is_refused_before_anything_starts() {
    let mut cfg = ConnectionConfig::new("ro", "mongodb://localhost:27017/app");
    cfg.read_only = true;
    let mut f = Fixture::connected(cfg, None).await;
    let pidfile = f.fake.path("ro.pid");
    f.run(1, &format!("spawn {}", pidfile.display()));
    assert_eq!(f.refusal().await, ShellRefusal::ReadOnly);
    f.assert_quiet().await;
    assert!(!pidfile.exists(), "nothing may be spawned");
}

#[tokio::test]
async fn a_lock_sent_after_the_connect_refuses_too() {
    let mut f = Fixture::plain().await;
    let connection = f.id;
    f.h.tx.send(Command::LockReadOnly { connection }).unwrap();
    f.run(1, "hello");
    assert_eq!(f.refusal().await, ShellRefusal::ReadOnly);
}

#[tokio::test]
async fn second_run_while_running_is_refused() {
    let mut f = Fixture::plain().await;
    let pidfile = f.fake.path("second.pid");
    f.run(1, &format!("spawn {}\nhang", pidfile.display()));
    f.run(2, "hello");
    assert_eq!(f.refusal().await, ShellRefusal::AlreadyRunning);
    // The first run is untouched and still ours to stop.
    wait_for_file(&pidfile).await;
    f.cancel(1);
    assert_eq!(f.outcome().await, Outcome::Cancelled);
    f.run(3, "again");
    assert_eq!(f.outcome().await, Outcome::Exited(0));
}

#[tokio::test]
async fn run_on_disconnected_session_is_refused() {
    let mut f = Fixture::plain().await;
    let stranger = Uuid::new_v4();
    f.h.tx
        .send(Command::ShellRun {
            session: stranger,
            run_id: 1,
            exe: f.fake.exe.clone(),
            script: "hello".into(),
        })
        .unwrap();
    match next(&mut f.h).await {
        Event::ShellRefused {
            session,
            run_id: 1,
            reason: ShellRefusal::NotConnected,
        } => assert_eq!(session, stranger),
        other => panic!("{other:?}"),
    }
    let session = f.id;
    f.h.tx.send(Command::Disconnect { session }).unwrap();
    assert!(matches!(next(&mut f.h).await, Event::Disconnected { .. }));
    f.run(2, "hello");
    assert_eq!(f.refusal().await, ShellRefusal::NotConnected);
}

#[tokio::test]
async fn an_invalid_script_is_refused_with_the_reason() {
    let mut f = Fixture::plain().await;
    f.run(1, "   ");
    assert_eq!(
        f.refusal().await,
        ShellRefusal::Invalid(ShellError::EmptyScript)
    );
    // The refusal freed the slot.
    f.run(2, "hello");
    assert_eq!(f.outcome().await, Outcome::Exited(0));
}

#[tokio::test]
async fn disconnect_kills_the_running_shell() {
    let mut f = Fixture::plain().await;
    let pidfile = f.fake.path("kill.pid");
    f.run(1, &format!("spawn {}\nhang", pidfile.display()));
    let pid = wait_for_file(&pidfile).await;
    assert!(alive(&pid));
    let session = f.id;
    f.h.tx.send(Command::Disconnect { session }).unwrap();
    let mut finished = None;
    let mut disconnected = false;
    while finished.is_none() || !disconnected {
        match next(&mut f.h).await {
            Event::ShellFinished { outcome, .. } => finished = Some(outcome),
            Event::Disconnected { .. } => disconnected = true,
            Event::ShellOutput { .. } => {}
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(finished, Some(Outcome::Cancelled));
    wait_dead(&pid).await;
}

#[tokio::test]
async fn a_new_connect_kills_the_running_shell() {
    let cfg = ConnectionConfig::new("t", "mongodb://localhost:27017/app");
    let mut f = Fixture::connected(cfg.clone(), None).await;
    let pidfile = f.fake.path("reconnect.pid");
    f.run(1, &format!("spawn {}\nhang", pidfile.display()));
    let pid = wait_for_file(&pidfile).await;
    f.h.tx.send(Command::Connect(cfg)).unwrap();
    let (mut finished, mut connected) = (None, false);
    while finished.is_none() || !connected {
        match next(&mut f.h).await {
            Event::ShellFinished { outcome, .. } => finished = Some(outcome),
            Event::Connected { .. } => connected = true,
            Event::ShellOutput { .. } | Event::Sampler { .. } | Event::Profiler { .. } => {}
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(finished, Some(Outcome::Cancelled));
    wait_dead(&pid).await;
    f.run(2, "hello");
    assert_eq!(f.outcome().await, Outcome::Exited(0));
}

#[tokio::test]
async fn cancel_of_a_stale_run_id_is_ignored() {
    let mut f = Fixture::plain().await;
    f.cancel(1); // nothing runs yet
    let pidfile = f.fake.path("stale.pid");
    f.run(5, &format!("spawn {}\nhang", pidfile.display()));
    let pid = wait_for_file(&pidfile).await;
    f.cancel(4);
    f.cancel(6);
    f.assert_quiet().await;
    assert!(
        alive(&pid),
        "a cancel of another run must not stop this one"
    );
    f.cancel(5);
    assert_eq!(f.outcome().await, Outcome::Cancelled);
    wait_dead(&pid).await;
}

#[tokio::test]
async fn start_failed_frees_the_slot() {
    let mut f = Fixture::plain().await;
    f.run_with(1, f.fake.path("no-such-mongosh"), "hello");
    // on_done fires on the caller's thread, before spawn_shell returns.
    assert_eq!(f.outcome().await, Outcome::StartFailed);
    f.run(2, "hello");
    assert_eq!(f.outcome().await, Outcome::Exited(0));
}

// ---- connection details ----

/// A `mongosh` that records the preamble (which holds the connection string) and exits.
fn capturing_exe(dir: &Path, log: &Path) -> PathBuf {
    let exe = dir.join("capture-mongosh");
    let script = format!(
        "#!/bin/bash\nprintf '> '\nIFS= read -r preamble\nprintf '%s\\n' \"$preamble\" > '{}'\nprintf '__CB_READY__\\n\\n'\nexit 0\n",
        log.display()
    );
    std::fs::write(&exe, script).unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    exe
}

#[tokio::test]
async fn tunnel_address_is_used_when_the_session_has_one() {
    let mut cfg = ConnectionConfig::new("t", "mongodb://db.internal:27018/app");
    cfg.auth = crate::connection::AuthMechanism::ScramSha256;
    cfg.username = Some("alice".into());
    cfg.ssh = Some(SshConfig {
        host: "bastion".into(),
        port: 22,
        username: "u".into(),
        auth: SshAuth::Password,
    });
    let mut f = Fixture::connected(cfg, Some(PASSWORD)).await;
    let log = f.fake.path("preamble.log");
    let exe = capturing_exe(f.fake.dir(), &log);
    f.run_with(1, exe, "hello");
    assert_eq!(f.outcome().await, Outcome::Exited(0));
    let preamble = wait_for_file(&log).await;
    assert!(preamble.contains(TUNNEL), "{preamble}");
    assert!(!preamble.contains("db.internal"), "{preamble}");
    assert!(preamble.contains(&percent_encode_component(PASSWORD)));
}

#[tokio::test]
async fn without_a_tunnel_the_uri_host_is_used() {
    let mut f = Fixture::plain().await;
    let log = f.fake.path("plain.log");
    let exe = capturing_exe(f.fake.dir(), &log);
    f.run_with(1, exe, "hello");
    assert_eq!(f.outcome().await, Outcome::Exited(0));
    assert!(wait_for_file(&log).await.contains("localhost:27017"));
}

#[tokio::test]
async fn password_never_appears_in_any_event_or_debug_output() {
    let mut cfg = ConnectionConfig::new("t", "mongodb://localhost:27017/app");
    cfg.auth = crate::connection::AuthMechanism::ScramSha256;
    cfg.username = Some("alice".into());
    let mut f = Fixture::connected(cfg, Some(PASSWORD)).await;
    let encoded = percent_encode_component(PASSWORD);
    f.run(1, &format!("{PASSWORD}\n{encoded}\nstderr {PASSWORD}"));
    let events = f.until_done().await;
    let debug = events
        .iter()
        .map(|e| format!("{e:?}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!debug.contains(PASSWORD), "{debug}");
    assert!(!debug.contains(&encoded), "{debug}");
    assert!(
        output_text(&events, Stream::Stdout).contains(MASK),
        "{debug}"
    );
    assert!(
        output_text(&events, Stream::Stderr).contains(MASK),
        "{debug}"
    );
}

// ---- races ----

/// A secret store whose next lookup, once armed, waits for the test: the window between the slot
/// being taken and the invocation being built, held open.
struct GatedSecrets {
    inner: MemorySecretStore,
    gate: std::sync::Mutex<Option<ArmedGate>>,
}

/// The store's side of [`Gate`].
struct ArmedGate {
    release: std::sync::mpsc::Receiver<()>,
    taken: tokio::sync::oneshot::Sender<()>,
}

/// The test's side: `taken` resolves once the next lookup has started (and is parked), which
/// then waits until `release` sends (or is dropped).
struct Gate {
    release: std::sync::mpsc::Sender<()>,
    taken: tokio::sync::oneshot::Receiver<()>,
}

impl GatedSecrets {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: MemorySecretStore::new(),
            gate: Default::default(),
        })
    }

    fn arm(&self) -> Gate {
        let (release_tx, release) = std::sync::mpsc::channel();
        let (taken, taken_rx) = tokio::sync::oneshot::channel();
        *self.gate.lock().unwrap() = Some(ArmedGate { release, taken });
        Gate {
            release: release_tx,
            taken: taken_rx,
        }
    }
}

impl SecretStore for GatedSecrets {
    fn get(&self, key: &str) -> Result<Option<String>> {
        let gate = self.gate.lock().unwrap().take();
        if let Some(gate) = gate {
            let _ = gate.taken.send(());
            let _ = gate.release.recv();
        }
        self.inner.get(key)
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.inner.set(key, value)
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.inner.delete(key)
    }
}

/// The next event that is not background sampling or profiling.
async fn next_relevant(h: &mut RuntimeHandle) -> Event {
    loop {
        match next(h).await {
            Event::Sampler { .. } | Event::Profiler { .. } => {}
            event => return event,
        }
    }
}

async fn gated_fixture() -> (Fixture, Arc<GatedSecrets>) {
    let secrets = GatedSecrets::new();
    let cfg = ConnectionConfig::new("t", "mongodb://localhost:27017/app");
    let f = Fixture::connected_with(cfg, secrets.clone()).await;
    (f, secrets)
}

#[tokio::test]
async fn dropping_the_runtime_kills_a_running_shell() {
    let f = Fixture::plain().await;
    let pidfile = f.fake.path("drop.pid");
    f.run(1, &format!("spawn {}\nhang", pidfile.display()));
    let pid = wait_for_file(&pidfile).await;
    assert!(alive(&pid));
    // No more commands: the dispatcher ends and the sessions are dropped with it.
    drop(f.h);
    wait_dead(&pid).await;
}

#[tokio::test]
async fn disconnect_during_the_password_read_refuses_and_frees_the_slot() {
    let (mut f, secrets) = gated_fixture().await;
    let pidfile = f.fake.path("late.pid");
    let gate = secrets.arm();
    f.run(1, &format!("spawn {}", pidfile.display()));
    gate.taken.await.unwrap();
    let session = f.id;
    f.h.tx.send(Command::Disconnect { session }).unwrap();
    // Sent after the session ended: the run sees the disconnect when its lookup returns.
    let event = next_relevant(&mut f.h).await;
    assert!(matches!(event, Event::Disconnected { .. }), "{event:?}");
    gate.release.send(()).unwrap();
    match next_relevant(&mut f.h).await {
        Event::ShellRefused { reason, .. } => assert_eq!(reason, ShellRefusal::NotConnected),
        other => panic!("{other:?}"),
    }
    f.assert_quiet().await;
    assert!(!pidfile.exists(), "nothing may be spawned");
}

#[tokio::test]
async fn a_new_connect_during_the_password_read_refuses_and_frees_the_slot() {
    let secrets = GatedSecrets::new();
    let cfg = ConnectionConfig::new("t", "mongodb://localhost:27017/app");
    let mut f = Fixture::connected_with(cfg.clone(), secrets.clone()).await;
    let gate = secrets.arm();
    f.run(1, "hello");
    // The run's lookup holds the gate, so the new connect's own lookup is not held up.
    gate.taken.await.unwrap();
    f.h.tx.send(Command::Connect(cfg)).unwrap();
    let event = next_relevant(&mut f.h).await;
    assert!(matches!(event, Event::Connected { .. }), "{event:?}");
    gate.release.send(()).unwrap();
    match next_relevant(&mut f.h).await {
        Event::ShellRefused { reason, .. } => assert_eq!(reason, ShellRefusal::NotConnected),
        other => panic!("{other:?}"),
    }
    f.run(2, "hello");
    assert_eq!(f.outcome().await, Outcome::Exited(0));
}

#[tokio::test]
async fn cancel_during_the_password_read_never_spawns() {
    let (mut f, secrets) = gated_fixture().await;
    // A spawn is seen: this executable does not exist, so a spawned run ends `StartFailed` (a run
    // that was spawned and then cancelled when its handle came would not be told apart).
    let missing = f.fake.dir().join("no-such-mongosh");
    let gate = secrets.arm();
    f.run_with(1, missing, "hello");
    gate.taken.await.unwrap();
    f.cancel(1);
    // Commands are handled in order and a refusal is decided synchronously: this one proves the
    // cancel was handled while the password lookup is still parked.
    f.run(2, "hello");
    assert_eq!(f.refusal().await, ShellRefusal::AlreadyRunning);
    gate.release.send(()).unwrap();
    assert_eq!(f.outcome().await, Outcome::Cancelled);
    f.assert_quiet().await;
    f.run(3, "hello");
    assert_eq!(f.outcome().await, Outcome::Exited(0));
}

fn bare_shared() -> Arc<Shared> {
    Arc::new(Shared {
        sessions: std::sync::RwLock::new(Default::default()),
        next_epoch: std::sync::atomic::AtomicU64::new(0),
        secrets: None,
        connector: Arc::new(StoreConnector),
        config: RuntimeConfig::default(),
        compare: Default::default(),
        copy: Default::default(),
        latest_test: Default::default(),
        read_only: Default::default(),
        cancelled: Default::default(),
        history: None,
        sampler_config: tokio::sync::watch::channel(SamplerConfig::default().clamped()).0,
    })
}

#[tokio::test]
async fn a_configured_tunnel_that_is_gone_refuses_without_spawning() {
    let shared = bare_shared();
    let mut cfg = ConnectionConfig::new("t", "mongodb://db.internal:27018/app");
    cfg.ssh = Some(SshConfig {
        host: "bastion".into(),
        port: 22,
        username: "u".into(),
        auth: SshAuth::Password,
    });
    let id = cfg.id;
    let (session, generation) = shared.begin_session_epoch(id);
    // Installed with its config but without the tunnel, as after an epoch change.
    assert!(session.install_with_config(
        generation,
        Arc::new(InMemoryStore::new()),
        None,
        Some(cfg)
    ));
    let fake = FakeMongosh::new(Behaviour::Repl);
    let pidfile = fake.path("tunnel.pid");
    let script = format!("spawn {}", pidfile.display());
    let run = super::shell_handlers::claim(&shared, id, 1, fake.exe.clone(), script).unwrap();
    let (events, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let repaint: Repaint = Arc::new(|| {});
    let spawned = Arc::new(std::sync::atomic::AtomicBool::new(false));
    super::shell_handlers::start(shared, events, repaint, run, spawned).await;
    assert_eq!(
        rx.try_recv(),
        Ok(Event::ShellRefused {
            session: id,
            run_id: 1,
            reason: ShellRefusal::NotConnected,
        })
    );
    assert!(rx.try_recv().is_err());
    assert!(!pidfile.exists(), "nothing may be spawned");
    assert!(session.shell.reserve(2), "the slot is free");
}

async fn panicking_start(spawned: bool) -> (Option<Event>, bool) {
    let shared = bare_shared();
    let id = Uuid::new_v4();
    let (session, _) = shared.begin_session_epoch(id);
    assert!(session.shell.reserve(1));
    let (events, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let repaint: Repaint = Arc::new(|| {});
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(spawned));
    let work = async { panic!("injected start panic") };
    super::shell_handlers::supervised(session.clone(), id, 1, events, repaint, flag, work).await;
    (rx.try_recv().ok(), session.shell.reserve(2))
}

#[tokio::test]
async fn a_panic_before_the_spawn_frees_the_slot_and_ends_the_run() {
    let (event, free) = panicking_start(false).await;
    assert!(matches!(
        event,
        Some(Event::ShellFinished {
            run_id: 1,
            outcome: Outcome::StartFailed,
            ..
        })
    ));
    assert!(free);
}

#[tokio::test]
async fn a_panic_after_the_spawn_leaves_the_report_to_the_runner() {
    let (event, free) = panicking_start(true).await;
    assert_eq!(event, None);
    assert!(!free, "the runner's on_done frees the slot");
}
