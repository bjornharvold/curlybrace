//! `Command::TestConnection` opens a throw-away connection: it never begins an epoch and never
//! touches the installed store or tunnel.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bson::{Bson, Document};
use russh::keys::PublicKey;
use russh::keys::ssh_key::{PrivateKey, private::Ed25519Keypair};
use tokio::sync::oneshot;

use super::tunneling::{TunnelHandle, TunnelOpener, TunnelRequest};
use super::*;
use crate::connection::{AuthMechanism, SshAuth, SshConfig};
use crate::secrets::{MemorySecretStore, SecretKind, SecretString, secret_key};
use crate::store::Page;
use crate::test_support::InMemoryStore;
use crate::tunnel::{HostKeyDecision, check_host_key};

const WAIT: Duration = Duration::from_secs(5);

// ---- fakes ----

/// Delegates data calls to an in-memory store; reports a fixed version; counts drops.
struct VersionedStore {
    inner: InMemoryStore,
    version: Option<String>,
    drops: Arc<AtomicUsize>,
}

impl Drop for VersionedStore {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl DocumentStore for VersionedStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        self.inner.list_databases().await
    }
    async fn list_collections(&self, db: &str) -> Result<Vec<String>> {
        self.inner.list_collections(db).await
    }
    async fn find(&self, ns: &Namespace, q: &Query) -> Result<Page> {
        self.inner.find(ns, q).await
    }
    async fn insert(&self, ns: &Namespace, doc: Document) -> Result<Bson> {
        self.inner.insert(ns, doc).await
    }
    async fn replace(&self, ns: &Namespace, id: &Bson, doc: Document) -> Result<()> {
        self.inner.replace(ns, id, doc).await
    }
    async fn delete(&self, ns: &Namespace, id: &Bson) -> Result<()> {
        self.inner.delete(ns, id).await
    }
    async fn server_version(&self) -> Result<Option<String>> {
        Ok(self.version.clone())
    }
}

/// Behaviour is keyed by the config name: `bad*` fails with `AuthFailed`, `panic*` panics,
/// the name in `gate_name` waits for the test to release it.
struct FakeConnector {
    version: Option<String>,
    calls: AtomicUsize,
    store_drops: Arc<AtomicUsize>,
    passwords: Mutex<Vec<Option<String>>>,
    tunnels: Mutex<Vec<Option<SocketAddr>>>,
    gate_name: &'static str,
    gate: Mutex<Option<oneshot::Receiver<()>>>,
}

impl FakeConnector {
    fn build(
        version: Option<&str>,
        gate: Option<(&'static str, oneshot::Receiver<()>)>,
    ) -> Arc<Self> {
        let (gate_name, gate) = match gate {
            Some((name, rx)) => (name, Some(rx)),
            None => ("", None),
        };
        Arc::new(Self {
            version: version.map(str::to_owned),
            calls: AtomicUsize::new(0),
            store_drops: Arc::new(AtomicUsize::new(0)),
            passwords: Mutex::new(Vec::new()),
            tunnels: Mutex::new(Vec::new()),
            gate_name,
            gate: Mutex::new(gate),
        })
    }
    fn ok() -> Arc<Self> {
        Self::build(Some("7.0.5"), None)
    }
    fn gated(name: &'static str) -> (Arc<Self>, oneshot::Sender<()>) {
        let (release, gate) = oneshot::channel();
        (Self::build(Some("7.0.5"), Some((name, gate))), release)
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    fn last_password(&self) -> Option<String> {
        self.passwords.lock().unwrap().last().cloned().flatten()
    }
}

#[async_trait]
impl Connector for FakeConnector {
    async fn connect(
        &self,
        cfg: &ConnectionConfig,
        password: Option<&str>,
        tunnel: Option<SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.passwords
            .lock()
            .unwrap()
            .push(password.map(str::to_owned));
        self.tunnels.lock().unwrap().push(tunnel);
        if cfg.name.starts_with("panic") {
            panic!("connector blew up");
        }
        if cfg.name == self.gate_name {
            let gate = self.gate.lock().unwrap().take();
            if let Some(gate) = gate {
                let _ = gate.await;
            }
        }
        if cfg.name.starts_with("bad") {
            return Err(CoreError::AuthFailed);
        }
        Ok(Arc::new(VersionedStore {
            inner: InMemoryStore::new(),
            version: self.version.clone(),
            drops: self.store_drops.clone(),
        }))
    }
}

struct FakeTunnel {
    drops: Arc<AtomicUsize>,
}

impl TunnelHandle for FakeTunnel {
    fn local_addr(&self) -> SocketAddr {
        "127.0.0.1:50997".parse().unwrap()
    }
}

impl Drop for FakeTunnel {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

/// Runs the REAL host-key policy with the prompt the runtime hands over; records the secret.
struct PolicyOpener {
    key: PublicKey,
    drops: Arc<AtomicUsize>,
    secrets: Mutex<Vec<Option<String>>>,
    gate: Mutex<Option<oneshot::Receiver<()>>>,
}

impl PolicyOpener {
    fn new(gate: Option<oneshot::Receiver<()>>) -> Arc<Self> {
        let key = PrivateKey::from(Ed25519Keypair::from_seed(&[7; 32]))
            .public_key()
            .clone();
        Arc::new(Self {
            key,
            drops: Arc::new(AtomicUsize::new(0)),
            secrets: Mutex::new(Vec::new()),
            gate: Mutex::new(gate),
        })
    }
}

#[async_trait]
impl TunnelOpener for PolicyOpener {
    async fn open(&self, request: TunnelRequest<'_>) -> Result<Box<dyn TunnelHandle>> {
        self.secrets
            .lock()
            .unwrap()
            .push(request.secret.map(str::to_owned));
        let gate = self.gate.lock().unwrap().take();
        if let Some(gate) = gate {
            let _ = gate.await;
        }
        let (known_hosts, prompt, key) = (
            request.known_hosts.to_path_buf(),
            request.prompt.clone(),
            self.key.clone(),
        );
        tokio::task::spawn_blocking(move || {
            check_host_key(&known_hosts, "bastion", 22, &key, prompt.as_ref())
        })
        .await
        .map_err(|_| CoreError::Ssh("join".into()))??;
        Ok(Box::new(FakeTunnel {
            drops: self.drops.clone(),
        }))
    }
}

struct TrustPrompt;

impl HostKeyPrompt for TrustPrompt {
    fn decide(&self, _host: &str, _fingerprint: &str) -> HostKeyDecision {
        HostKeyDecision::TrustAndRemember
    }
}

// ---- helpers ----

fn cfg(name: &str) -> ConnectionConfig {
    ConnectionConfig::new(name, "mongodb://h")
}

fn ssh_cfg(name: &str) -> ConnectionConfig {
    let mut c = ConnectionConfig::new(name, "mongodb://db.internal:27018/app");
    c.ssh = Some(SshConfig {
        host: "bastion".into(),
        port: 22,
        username: "u".into(),
        auth: SshAuth::Password,
    });
    c
}

fn test_cmd(request_id: u64, cfg: ConnectionConfig) -> Command {
    Command::TestConnection {
        request_id,
        cfg,
        secrets: TestSecrets::default(),
    }
}

fn start(connector: Arc<FakeConnector>, config: RuntimeConfig) -> RuntimeHandle {
    start_with_secrets(None, connector, config)
}

fn start_with_secrets(
    secrets: Option<Arc<dyn SecretStore>>,
    connector: Arc<FakeConnector>,
    config: RuntimeConfig,
) -> RuntimeHandle {
    let repaint: Repaint = Arc::new(|| {});
    spawn_runtime_with_connector(secrets, None, connector, repaint, config)
}

async fn next(rt: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, rt.rx.recv())
        .await
        .expect("event in time")
        .expect("channel open")
}

async fn assert_silent(rt: &mut RuntimeHandle) {
    let waited = tokio::time::timeout(Duration::from_millis(200), rt.rx.recv()).await;
    assert!(waited.is_err(), "unexpected event: {waited:?}");
}

async fn start_connected(connector: Arc<FakeConnector>) -> RuntimeHandle {
    let mut rt = start(connector, RuntimeConfig::default());
    rt.tx.send(Command::Connect(cfg("main"))).unwrap();
    assert_eq!(next(&mut rt).await, Event::Connected { epoch: 1 });
    rt
}

fn report(version: Option<&str>) -> std::result::Result<TestReport, CoreError> {
    Ok(TestReport {
        server_version: version.map(str::to_owned),
        tls: false,
        authenticated: false,
    })
}

// ---- behaviour ----

#[tokio::test]
async fn a_successful_test_reports_the_server_version_and_installs_nothing() {
    let mut rt = start(FakeConnector::ok(), RuntimeConfig::default());
    rt.tx.send(test_cmd(1, cfg("a"))).unwrap();
    assert_eq!(
        next(&mut rt).await,
        Event::TestConnection {
            request_id: 1,
            outcome: report(Some("7.0.5")),
        }
    );
    rt.tx.send(Command::ListDatabases).unwrap();
    assert!(
        matches!(
            next(&mut rt).await,
            Event::Failed {
                context: "list_databases",
                ..
            }
        ),
        "a test must not leave a connected store behind"
    );
}

#[tokio::test]
async fn the_report_carries_tls_and_authentication_flags() {
    let mut rt = start(FakeConnector::ok(), RuntimeConfig::default());
    let mut c = cfg("a");
    c.tls.enabled = true;
    c.auth = AuthMechanism::ScramSha256;
    c.username = Some("u".into());
    rt.tx.send(test_cmd(1, c)).unwrap();
    assert_eq!(
        next(&mut rt).await,
        Event::TestConnection {
            request_id: 1,
            outcome: Ok(TestReport {
                server_version: Some("7.0.5".into()),
                tls: true,
                authenticated: true,
            }),
        }
    );
}

#[tokio::test]
async fn a_test_does_not_disturb_an_installed_session() {
    let mut rt = start_connected(FakeConnector::ok()).await;
    rt.tx.send(test_cmd(7, cfg("b"))).unwrap();
    assert!(matches!(
        next(&mut rt).await,
        Event::TestConnection {
            request_id: 7,
            outcome: Ok(_)
        }
    ));
    rt.tx.send(Command::ListDatabases).unwrap();
    assert!(matches!(next(&mut rt).await, Event::Databases(_)));
    // The epoch did not move: the next Connect is epoch 2, not 3.
    rt.tx.send(Command::Connect(cfg("main"))).unwrap();
    assert_eq!(next(&mut rt).await, Event::Connected { epoch: 2 });
}

#[tokio::test]
async fn a_failed_test_reports_the_typed_error_and_keeps_the_session() {
    let mut rt = start_connected(FakeConnector::ok()).await;
    rt.tx.send(test_cmd(2, cfg("bad"))).unwrap();
    assert_eq!(
        next(&mut rt).await,
        Event::TestConnection {
            request_id: 2,
            outcome: Err(CoreError::AuthFailed),
        }
    );
    rt.tx.send(Command::ListDatabases).unwrap();
    assert!(matches!(next(&mut rt).await, Event::Databases(_)));
}

#[tokio::test]
async fn an_invalid_config_fails_without_touching_the_connector() {
    let connector = FakeConnector::ok();
    let mut rt = start(connector.clone(), RuntimeConfig::default());
    rt.tx.send(test_cmd(3, cfg(" "))).unwrap();
    assert!(matches!(
        next(&mut rt).await,
        Event::TestConnection {
            request_id: 3,
            outcome: Err(CoreError::InvalidUri(_))
        }
    ));
    assert_eq!(connector.calls(), 0);
}

#[tokio::test]
async fn typed_secrets_win_over_stored_ones_and_are_not_in_debug_output() {
    let connector = FakeConnector::ok();
    let c = cfg("a");
    let secrets = MemorySecretStore::new();
    secrets
        .set(&secret_key(c.id, SecretKind::Password), "stored")
        .unwrap();
    let mut rt = start_with_secrets(
        Some(Arc::new(secrets)),
        connector.clone(),
        RuntimeConfig::default(),
    );
    let command = Command::TestConnection {
        request_id: 4,
        cfg: c.clone(),
        secrets: TestSecrets {
            password: Some(SecretString::new("typed-pw")),
            ssh_secret: Some(SecretString::new("typed-ssh")),
        },
    };
    let shown = format!("{command:?}");
    assert!(
        !shown.contains("typed-pw") && !shown.contains("typed-ssh"),
        "{shown}"
    );
    rt.tx.send(command).unwrap();
    let _ = next(&mut rt).await;
    assert_eq!(connector.last_password().as_deref(), Some("typed-pw"));

    // None means "use the stored one".
    rt.tx.send(test_cmd(5, c)).unwrap();
    let _ = next(&mut rt).await;
    assert_eq!(connector.last_password().as_deref(), Some("stored"));
}

#[tokio::test]
async fn the_event_carries_no_secret_in_its_debug_output() {
    let mut rt = start(FakeConnector::ok(), RuntimeConfig::default());
    rt.tx
        .send(Command::TestConnection {
            request_id: 1,
            cfg: cfg("a"),
            secrets: TestSecrets {
                password: Some(SecretString::new("typed-pw")),
                ssh_secret: None,
            },
        })
        .unwrap();
    let shown = format!("{:?}", next(&mut rt).await);
    assert!(!shown.contains("typed-pw"), "{shown}");
}

#[tokio::test]
async fn a_connect_in_flight_when_a_test_is_requested_still_completes() {
    let (connector, release) = FakeConnector::gated("main");
    let mut rt = start(connector, RuntimeConfig::default());
    rt.tx.send(Command::Connect(cfg("main"))).unwrap(); // epoch 1, parked
    rt.tx.send(test_cmd(1, cfg("other"))).unwrap();
    assert!(matches!(
        next(&mut rt).await,
        Event::TestConnection {
            request_id: 1,
            outcome: Ok(_)
        }
    ));
    release.send(()).unwrap();
    assert_eq!(next(&mut rt).await, Event::Connected { epoch: 1 });
    rt.tx.send(Command::ListDatabases).unwrap();
    assert!(matches!(next(&mut rt).await, Event::Databases(_)));
}

#[tokio::test]
async fn a_test_then_a_connect_the_connect_wins_and_the_test_is_still_delivered() {
    let (connector, release) = FakeConnector::gated("slow-test");
    let mut rt = start(connector, RuntimeConfig::default());
    rt.tx.send(test_cmd(9, cfg("slow-test"))).unwrap(); // parked in the connector
    rt.tx.send(Command::Connect(cfg("main"))).unwrap();
    assert_eq!(next(&mut rt).await, Event::Connected { epoch: 1 });
    release.send(()).unwrap();
    assert_eq!(
        next(&mut rt).await,
        Event::TestConnection {
            request_id: 9,
            outcome: report(Some("7.0.5")),
        }
    );
    rt.tx.send(Command::ListDatabases).unwrap();
    assert!(matches!(next(&mut rt).await, Event::Databases(_)));
}

#[tokio::test]
async fn a_panic_in_the_work_still_yields_exactly_one_event() {
    let mut rt = start(FakeConnector::ok(), RuntimeConfig::default());
    rt.tx.send(test_cmd(6, cfg("panic"))).unwrap();
    assert_eq!(
        next(&mut rt).await,
        Event::TestConnection {
            request_id: 6,
            outcome: Err(CoreError::Database("internal error".into())),
        }
    );
    assert_silent(&mut rt).await;
}

#[tokio::test]
async fn a_test_is_bounded_by_the_connect_timeout() {
    let (connector, _release) = FakeConnector::gated("hang");
    let config = RuntimeConfig {
        connect_timeout: Duration::from_millis(50),
        ..RuntimeConfig::default()
    };
    let mut rt = start(connector, config);
    rt.tx.send(test_cmd(8, cfg("hang"))).unwrap();
    assert_eq!(
        next(&mut rt).await,
        Event::TestConnection {
            request_id: 8,
            outcome: Err(CoreError::Database("timeout".into())),
        }
    );
}

#[tokio::test]
async fn the_store_is_dropped_before_the_answer_is_sent() {
    let connector = FakeConnector::ok();
    let mut rt = start(connector.clone(), RuntimeConfig::default());
    rt.tx.send(test_cmd(1, cfg("a"))).unwrap();
    let _ = next(&mut rt).await;
    assert_eq!(connector.store_drops.load(Ordering::SeqCst), 1);
}

// ---- ssh ----

struct SshRig {
    rt: RuntimeHandle,
    connector: Arc<FakeConnector>,
    opener: Arc<PolicyOpener>,
    known_hosts: PathBuf,
    _dir: tempfile::TempDir,
}

fn ssh_rig(
    prompt: Arc<dyn HostKeyPrompt>,
    gate: Option<oneshot::Receiver<()>>,
    secrets: Option<Arc<dyn SecretStore>>,
) -> SshRig {
    let dir = tempfile::tempdir().unwrap();
    let known_hosts = dir.path().join("known_hosts");
    let (connector, opener) = (FakeConnector::ok(), PolicyOpener::new(gate));
    let config = RuntimeConfig {
        known_hosts_path: Some(known_hosts.clone()),
        host_key_prompt: prompt,
        tunnel_opener: opener.clone(),
        ..RuntimeConfig::default()
    };
    SshRig {
        rt: start_with_secrets(secrets, connector.clone(), config),
        connector,
        opener,
        known_hosts,
        _dir: dir,
    }
}

#[tokio::test]
async fn an_unknown_host_key_fails_closed_with_the_default_prompt() {
    let mut r = ssh_rig(Arc::new(RejectUnknownHostKeys), None, None);
    r.rt.tx.send(test_cmd(1, ssh_cfg("s"))).unwrap();
    assert!(matches!(
        next(&mut r.rt).await,
        Event::TestConnection {
            request_id: 1,
            outcome: Err(_)
        }
    ));
    assert_eq!(
        r.connector.calls(),
        0,
        "no database connection after a rejected key"
    );
    assert!(!r.known_hosts.exists(), "nothing was remembered");
}

#[tokio::test]
async fn the_host_key_question_of_a_test_is_not_rejected_by_a_newer_connect() {
    let (release, gate) = oneshot::channel();
    let mut r = ssh_rig(Arc::new(TrustPrompt), Some(gate), None);
    r.rt.tx.send(test_cmd(1, ssh_cfg("s"))).unwrap(); // parked in the opener
    r.rt.tx.send(Command::Connect(cfg("main"))).unwrap(); // bumps the generation
    assert_eq!(next(&mut r.rt).await, Event::Connected { epoch: 1 });
    release.send(()).unwrap();
    assert!(matches!(
        next(&mut r.rt).await,
        Event::TestConnection {
            request_id: 1,
            outcome: Ok(_)
        }
    ));
    assert!(r.known_hosts.exists(), "the test's own prompt was honoured");
}

#[tokio::test]
async fn a_typed_ssh_secret_wins_and_the_tunnel_is_closed_before_the_answer() {
    let c = ssh_cfg("s");
    let stored = MemorySecretStore::new();
    stored
        .set(&secret_key(c.id, SecretKind::SshPassword), "stored-ssh")
        .unwrap();
    let mut r = ssh_rig(Arc::new(TrustPrompt), None, Some(Arc::new(stored)));
    r.rt.tx
        .send(Command::TestConnection {
            request_id: 1,
            cfg: c.clone(),
            secrets: TestSecrets {
                password: None,
                ssh_secret: Some(SecretString::new("typed-ssh")),
            },
        })
        .unwrap();
    assert!(matches!(
        next(&mut r.rt).await,
        Event::TestConnection { outcome: Ok(_), .. }
    ));
    assert_eq!(r.opener.drops.load(Ordering::SeqCst), 1);
    r.rt.tx.send(test_cmd(2, c)).unwrap();
    let _ = next(&mut r.rt).await;
    assert_eq!(
        r.opener.secrets.lock().unwrap().clone(),
        vec![Some("typed-ssh".into()), Some("stored-ssh".into())]
    );
}
