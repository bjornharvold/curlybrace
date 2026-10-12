//! Runtime wiring of the SSH tunnel, using a fake opener (no SSH server).

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::oneshot;

use super::tunneling::{TunnelHandle, TunnelOpener, TunnelRequest, parse_target};
use super::*;
use crate::connection::{SshAuth, SshConfig};
use crate::secrets::{MemorySecretStore, SecretKind, secret_key};
use crate::test_support::InMemoryStore;

const WAIT: Duration = Duration::from_secs(5);

// ---- target parsing ----

fn target(uri: &str) -> Result<(String, u16)> {
    parse_target(uri)
}

#[test]
fn target_defaults_port_and_handles_forms() {
    assert_eq!(
        target("mongodb://db.internal"),
        Ok(("db.internal".into(), 27017))
    );
    assert_eq!(
        target("mongodb://db:27018/app?x=1"),
        Ok(("db".into(), 27018))
    );
    assert_eq!(target("mongodb://u@db:5/"), Ok(("db".into(), 5)));
    assert_eq!(target("mongodb://[::1]:27018/"), Ok(("::1".into(), 27018)));
    assert_eq!(target("mongodb://[::1]"), Ok(("::1".into(), 27017)));
    assert_eq!(
        target("mongodb://db?retryWrites=true"),
        Ok(("db".into(), 27017))
    );
}

#[test]
fn target_rejects_multi_host_srv_and_garbage() {
    assert!(matches!(
        target("mongodb://a:1,b:2/"),
        Err(CoreError::Unsupported(_))
    ));
    assert!(matches!(
        target("mongodb+srv://c.example.com"),
        Err(CoreError::Unsupported(_))
    ));
    assert!(matches!(
        target("mongodb://"),
        Err(CoreError::InvalidUri(_))
    ));
    assert!(matches!(
        target("mongodb://db:notaport"),
        Err(CoreError::InvalidUri(_))
    ));
    assert!(matches!(
        target("mongodb://db:99999"),
        Err(CoreError::InvalidUri(_))
    ));
    assert!(matches!(target("http://db"), Err(CoreError::InvalidUri(_))));
}

// ---- fakes ----

struct FakeTunnel {
    drops: Arc<AtomicUsize>,
}

impl TunnelHandle for FakeTunnel {
    fn local_addr(&self) -> SocketAddr {
        "127.0.0.1:50999".parse().unwrap()
    }
}

impl Drop for FakeTunnel {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Seen {
    ssh_host: String,
    target_host: String,
    target_port: u16,
    secret: Option<String>,
    key_auth: bool,
}

struct FakeOpener {
    drops: Arc<AtomicUsize>,
    seen: Mutex<Vec<Seen>>,
    fail: bool,
}

impl FakeOpener {
    fn new(fail: bool) -> Arc<Self> {
        Arc::new(Self {
            drops: Arc::new(AtomicUsize::new(0)),
            seen: Mutex::new(Vec::new()),
            fail,
        })
    }
    fn drops(&self) -> usize {
        self.drops.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl TunnelOpener for FakeOpener {
    async fn open(&self, request: TunnelRequest<'_>) -> Result<Box<dyn TunnelHandle>> {
        self.seen.lock().unwrap().push(Seen {
            ssh_host: request.ssh.host.clone(),
            target_host: request.target_host.to_string(),
            target_port: request.target_port,
            secret: request.secret.map(str::to_owned),
            key_auth: matches!(request.ssh.auth, SshAuth::PrivateKey { .. }),
        });
        if self.fail {
            return Err(CoreError::Ssh("ssh authentication failed".into()));
        }
        Ok(Box::new(FakeTunnel {
            drops: self.drops.clone(),
        }))
    }
}

type Opened = Result<Arc<dyn DocumentStore>>;

/// Records the tunnel address; the result is optionally gated by the test.
struct RecordingConnector {
    tunnels: Mutex<Vec<Option<SocketAddr>>>,
    gate: Mutex<Option<oneshot::Receiver<()>>>,
}

#[async_trait]
impl Connector for RecordingConnector {
    async fn connect(
        &self,
        _cfg: &ConnectionConfig,
        _password: Option<&str>,
        tunnel: Option<SocketAddr>,
    ) -> Opened {
        self.tunnels.lock().unwrap().push(tunnel);
        let gate = self.gate.lock().unwrap().take();
        if let Some(gate) = gate {
            let _ = gate.await;
        }
        Ok(Arc::new(InMemoryStore::new()))
    }
}

fn recording(gate: Option<oneshot::Receiver<()>>) -> Arc<RecordingConnector> {
    Arc::new(RecordingConnector {
        tunnels: Mutex::new(Vec::new()),
        gate: Mutex::new(gate),
    })
}

fn ssh_cfg(auth: SshAuth) -> ConnectionConfig {
    let mut c = ConnectionConfig::new("t", "mongodb://db.internal:27018/app");
    c.ssh = Some(SshConfig {
        host: "bastion".into(),
        port: 22,
        username: "u".into(),
        auth,
    });
    c
}

fn runtime(
    secrets: Arc<MemorySecretStore>,
    connector: Arc<RecordingConnector>,
    opener: Arc<FakeOpener>,
) -> RuntimeHandle {
    let config = RuntimeConfig {
        tunnel_opener: opener,
        ..RuntimeConfig::default()
    };
    let repaint: Repaint = Arc::new(|| {});
    spawn_runtime_with_connector(Some(secrets), None, connector, repaint, config)
}

async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("event in time")
        .expect("channel open")
}

// ---- runtime behaviour ----

#[tokio::test]
async fn connect_with_password_ssh_opens_tunnel_with_target_and_secret() {
    let secrets = Arc::new(MemorySecretStore::new());
    let cfg = ssh_cfg(SshAuth::Password);
    secrets
        .set(&secret_key(cfg.id, SecretKind::SshPassword), "sshpw")
        .unwrap();
    secrets
        .set(&secret_key(cfg.id, SecretKind::SshPassphrase), "wrong-kind")
        .unwrap();
    let (connector, opener) = (recording(None), FakeOpener::new(false));
    let mut h = runtime(secrets, connector.clone(), opener.clone());
    h.tx.send(Command::Connect(cfg)).unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    assert_eq!(
        opener.seen.lock().unwrap().clone(),
        vec![Seen {
            ssh_host: "bastion".into(),
            target_host: "db.internal".into(),
            target_port: 27018,
            secret: Some("sshpw".into()),
            key_auth: false,
        }]
    );
    let expected: SocketAddr = "127.0.0.1:50999".parse().unwrap();
    assert_eq!(*connector.tunnels.lock().unwrap(), vec![Some(expected)]);
    assert_eq!(opener.drops(), 0, "tunnel stays alive while connected");
}

#[tokio::test]
async fn private_key_auth_reads_the_passphrase_and_absent_secret_is_none() {
    let secrets = Arc::new(MemorySecretStore::new());
    let auth = SshAuth::PrivateKey {
        path: "/k/id".into(),
    };
    let cfg = ssh_cfg(auth.clone());
    secrets
        .set(&secret_key(cfg.id, SecretKind::SshPassphrase), "phrase")
        .unwrap();
    let opener = FakeOpener::new(false);
    let mut h = runtime(secrets, recording(None), opener.clone());
    h.tx.send(Command::Connect(cfg)).unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    let seen = opener.seen.lock().unwrap().clone();
    assert!(seen[0].key_auth);
    assert_eq!(seen[0].secret.as_deref(), Some("phrase"));

    let opener = FakeOpener::new(false);
    let mut h = runtime(
        Arc::new(MemorySecretStore::new()),
        recording(None),
        opener.clone(),
    );
    h.tx.send(Command::Connect(ssh_cfg(auth))).unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    assert_eq!(opener.seen.lock().unwrap()[0].secret, None);
}

#[tokio::test]
async fn opener_error_becomes_failed_connect() {
    let (connector, opener) = (recording(None), FakeOpener::new(true));
    let mut h = runtime(
        Arc::new(MemorySecretStore::new()),
        connector.clone(),
        opener,
    );
    h.tx.send(Command::Connect(ssh_cfg(SshAuth::Password)))
        .unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::ConnectFailed {
            error: CoreError::Ssh(_),
            ..
        }
    ));
    assert!(connector.tunnels.lock().unwrap().is_empty());
}

#[tokio::test]
async fn multi_host_uri_with_ssh_is_unsupported_and_never_opens() {
    let mut cfg = ssh_cfg(SshAuth::Password);
    cfg.uri = "mongodb://a:1,b:2/?replicaSet=rs".into();
    let opener = FakeOpener::new(false);
    let mut h = runtime(
        Arc::new(MemorySecretStore::new()),
        recording(None),
        opener.clone(),
    );
    h.tx.send(Command::Connect(cfg)).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::ConnectFailed {
            error: CoreError::Unsupported(_),
            ..
        }
    ));
    assert!(opener.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn disconnect_closes_the_tunnel() {
    let opener = FakeOpener::new(false);
    let mut h = runtime(
        Arc::new(MemorySecretStore::new()),
        recording(None),
        opener.clone(),
    );
    h.tx.send(Command::Connect(ssh_cfg(SshAuth::Password)))
        .unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    assert_eq!(opener.drops(), 0);
    h.tx.send(Command::Disconnect).unwrap();
    assert!(matches!(next(&mut h).await, Event::Disconnected { .. }));
    assert_eq!(opener.drops(), 1);
}

#[tokio::test]
async fn a_new_connect_closes_the_previous_tunnel() {
    let opener = FakeOpener::new(false);
    let mut h = runtime(
        Arc::new(MemorySecretStore::new()),
        recording(None),
        opener.clone(),
    );
    h.tx.send(Command::Connect(ssh_cfg(SshAuth::Password)))
        .unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    h.tx.send(Command::Connect(ssh_cfg(SshAuth::Password)))
        .unwrap();
    assert!(matches!(next(&mut h).await, Event::Connected { .. }));
    assert_eq!(opener.drops(), 1, "only the first tunnel was closed");
}

#[tokio::test]
async fn superseded_connect_drops_its_tunnel() {
    let (release, gate) = oneshot::channel();
    let (connector, opener) = (recording(Some(gate)), FakeOpener::new(false));
    let mut h = runtime(
        Arc::new(MemorySecretStore::new()),
        connector.clone(),
        opener.clone(),
    );
    h.tx.send(Command::Connect(ssh_cfg(SshAuth::Password)))
        .unwrap();
    // Wait until the connect is parked in the connector with its tunnel open.
    while connector.tunnels.lock().unwrap().is_empty() {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    h.tx.send(Command::Disconnect).unwrap();
    assert!(matches!(next(&mut h).await, Event::Disconnected { .. }));
    release.send(()).unwrap();
    assert!(matches!(
        next(&mut h).await,
        Event::Failed {
            context: "connect",
            error: CoreError::Superseded
        }
    ));
    assert_eq!(opener.drops(), 1);
}

#[tokio::test]
async fn failed_driver_connect_drops_the_tunnel() {
    struct Failing;
    #[async_trait]
    impl Connector for Failing {
        async fn connect(
            &self,
            _: &ConnectionConfig,
            _: Option<&str>,
            _: Option<SocketAddr>,
        ) -> Opened {
            Err(CoreError::Database("down".into()))
        }
    }
    let opener = FakeOpener::new(false);
    let config = RuntimeConfig {
        tunnel_opener: opener.clone(),
        ..RuntimeConfig::default()
    };
    let repaint: Repaint = Arc::new(|| {});
    let mut h = spawn_runtime_with_connector(None, None, Arc::new(Failing), repaint, config);
    h.tx.send(Command::Connect(ssh_cfg(SshAuth::Password)))
        .unwrap();
    assert!(matches!(next(&mut h).await, Event::ConnectFailed { .. }));
    assert_eq!(opener.drops(), 1);
}
