//! A host-key question from a replaced connect must neither be asked nor be able to record trust.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Receiver as StdReceiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use russh::keys::PublicKey;
use russh::keys::ssh_key::{PrivateKey, private::Ed25519Keypair};
use tokio::sync::{Notify, oneshot};

use super::tunneling::{TunnelHandle, TunnelOpener, TunnelRequest};
use super::*;
use crate::connection::{SshAuth, SshConfig};
use crate::test_support::InMemoryStore;
use crate::tunnel::{HostKeyDecision, HostKeyPrompt, check_host_key};

const WAIT: Duration = Duration::from_secs(5);

struct NoTunnel;

impl TunnelHandle for NoTunnel {
    fn local_addr(&self) -> SocketAddr {
        "127.0.0.1:50998".parse().unwrap()
    }
}

/// Runs the REAL host-key policy (`check_host_key`, which writes known_hosts on
/// TrustAndRemember) with the prompt the runtime hands to openers.
struct PolicyOpener {
    key: PublicKey,
    entered: Arc<Notify>,
    gate: Mutex<Option<oneshot::Receiver<()>>>,
}

#[async_trait]
impl TunnelOpener for PolicyOpener {
    async fn open(&self, request: TunnelRequest<'_>) -> Result<Box<dyn TunnelHandle>> {
        self.entered.notify_one();
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
        Ok(Box::new(NoTunnel))
    }
}

/// Counts calls; optionally blocks until the test releases it, then answers `answer`.
struct InnerPrompt {
    answer: HostKeyDecision,
    calls: AtomicUsize,
    block: Mutex<Option<StdReceiver<()>>>,
}

impl HostKeyPrompt for InnerPrompt {
    fn decide(&self, _host: &str, _fingerprint: &str) -> HostKeyDecision {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let block = self.block.lock().unwrap().take();
        if let Some(block) = block {
            let _ = block.recv();
        }
        self.answer
    }
}

fn inner(answer: HostKeyDecision, block: Option<StdReceiver<()>>) -> Arc<InnerPrompt> {
    Arc::new(InnerPrompt {
        answer,
        calls: AtomicUsize::new(0),
        block: Mutex::new(block),
    })
}

fn ssh_cfg() -> ConnectionConfig {
    let mut c = embedded_config("ssh", "mongodb://db.internal:27018/app");
    c.ssh = Some(SshConfig {
        host: "bastion".into(),
        port: 22,
        username: "u".into(),
        auth: SshAuth::Password,
    });
    c
}

fn plain_cfg() -> ConnectionConfig {
    embedded_config("plain", "mongodb://127.0.0.1:27017")
}

struct Connector0;

#[async_trait]
impl Connector for Connector0 {
    async fn connect(
        &self,
        _cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        Ok(Arc::new(InMemoryStore::new()))
    }
}

struct Rig {
    h: RuntimeHandle,
    prompt: Arc<InnerPrompt>,
    entered: Arc<Notify>,
    known_hosts: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

fn rig(
    answer: HostKeyDecision,
    prompt_block: Option<StdReceiver<()>>,
    opener_gate: Option<oneshot::Receiver<()>>,
) -> Rig {
    rig_with_timeout(answer, prompt_block, opener_gate, None)
}

fn rig_with_timeout(
    answer: HostKeyDecision,
    prompt_block: Option<StdReceiver<()>>,
    opener_gate: Option<oneshot::Receiver<()>>,
    connect_timeout: Option<Duration>,
) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let known_hosts = dir.path().join("known_hosts");
    let prompt = inner(answer, prompt_block);
    let entered = Arc::new(Notify::new());
    let key = PrivateKey::from(Ed25519Keypair::from_seed(&[9; 32]))
        .public_key()
        .clone();
    let config = RuntimeConfig {
        known_hosts_path: Some(known_hosts.clone()),
        host_key_prompt: prompt.clone(),
        tunnel_opener: Arc::new(PolicyOpener {
            key,
            entered: entered.clone(),
            gate: Mutex::new(opener_gate),
        }),
        connect_timeout: connect_timeout.unwrap_or(RuntimeConfig::default().connect_timeout),
        ..RuntimeConfig::default()
    };
    let repaint: Repaint = Arc::new(|| {});
    let h = spawn_runtime_with_connector(None, None, Arc::new(Connector0), repaint, config);
    Rig {
        h,
        prompt,
        entered,
        known_hosts,
        _dir: dir,
    }
}

async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("event in time")
        .expect("channel open")
}

fn superseded() -> Event {
    Event::Failed {
        session: EMBEDDED_SESSION,
        context: "connect",
        error: CoreError::Superseded,
    }
}

async fn until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

#[tokio::test]
async fn a_prompt_requested_after_a_newer_connect_is_not_asked_and_records_nothing() {
    let (release, gate) = oneshot::channel();
    let mut r = rig(HostKeyDecision::TrustAndRemember, None, Some(gate));
    r.h.tx.send(Command::Connect(ssh_cfg())).unwrap(); // epoch 1, stuck in the opener
    r.entered.notified().await;
    r.h.tx.send(Command::Connect(plain_cfg())).unwrap(); // epoch 2 replaces it
    assert_eq!(
        next(&mut r.h).await,
        Event::Connected {
            session: EMBEDDED_SESSION,
            epoch: 2,
            topology: None,
        }
    );
    release.send(()).unwrap(); // epoch 1 now reaches the host-key policy
    assert_eq!(next(&mut r.h).await, superseded());
    assert_eq!(
        r.prompt.calls.load(Ordering::SeqCst),
        0,
        "inner prompt not asked"
    );
    assert!(!r.known_hosts.exists(), "known_hosts untouched");
}

#[tokio::test]
async fn an_answer_arriving_after_supersession_is_turned_into_a_reject() {
    let (unblock, block) = std::sync::mpsc::channel();
    let mut r = rig(HostKeyDecision::TrustAndRemember, Some(block), None);
    r.h.tx.send(Command::Connect(ssh_cfg())).unwrap(); // epoch 1 asks the user
    let prompt = r.prompt.clone();
    until("the question is asked", || {
        prompt.calls.load(Ordering::SeqCst) == 1
    })
    .await;
    r.h.tx.send(Command::Connect(plain_cfg())).unwrap(); // epoch 2 replaces it
    assert_eq!(
        next(&mut r.h).await,
        Event::Connected {
            session: EMBEDDED_SESSION,
            epoch: 2,
            topology: None,
        }
    );
    unblock.send(()).unwrap(); // the user "answers" Trust and remember, too late
    assert_eq!(next(&mut r.h).await, superseded());
    assert!(!r.known_hosts.exists(), "nothing was remembered");
}

#[tokio::test]
async fn a_current_connect_can_still_trust_and_remember() {
    let mut r = rig(HostKeyDecision::TrustAndRemember, None, None);
    r.h.tx.send(Command::Connect(ssh_cfg())).unwrap();
    assert_eq!(
        next(&mut r.h).await,
        Event::Connected {
            session: EMBEDDED_SESSION,
            epoch: 1,
            topology: None,
        }
    );
    assert_eq!(r.prompt.calls.load(Ordering::SeqCst), 1);
    let saved = std::fs::read_to_string(&r.known_hosts).unwrap();
    assert!(
        saved.trim_start().starts_with("bastion ssh-ed25519 "),
        "{saved}"
    );
}

// The same gate protects compare sessions: a question of a closed session is rejected.

fn open_compare() -> Command {
    Command::Compare(crate::compare::protocol::CompareCommand::Open {
        session: 1,
        source: ssh_cfg(),
        target: plain_cfg(),
    })
}

fn close_compare() -> Command {
    Command::Compare(crate::compare::protocol::CompareCommand::Close { session: 1 })
}

/// No event may arrive: a closed session's open task ends silently.
async fn assert_silent(h: &mut RuntimeHandle) {
    let waited = tokio::time::timeout(Duration::from_millis(300), h.rx.recv()).await;
    assert!(waited.is_err(), "unexpected event: {waited:?}");
}

#[tokio::test]
async fn a_closed_compare_session_never_asks_the_host_key_question() {
    let (release, gate) = oneshot::channel();
    let mut r = rig(HostKeyDecision::TrustAndRemember, None, Some(gate));
    r.h.tx.send(open_compare()).unwrap(); // stuck in the opener
    r.entered.notified().await;
    r.h.tx.send(close_compare()).unwrap();
    // Commands are dispatched in order, so this answer proves the Close was processed.
    r.h.tx
        .send(Command::ListDatabases {
            session: EMBEDDED_SESSION,
        })
        .unwrap();
    assert!(matches!(
        next(&mut r.h).await,
        Event::Failed {
            session: EMBEDDED_SESSION,
            ..
        }
    ));
    release.send(()).unwrap(); // the opener now reaches the host-key policy
    assert_silent(&mut r.h).await;
    assert_eq!(r.prompt.calls.load(Ordering::SeqCst), 0, "not asked");
    assert!(!r.known_hosts.exists(), "known_hosts untouched");
}

#[tokio::test]
async fn a_compare_answer_arriving_after_close_is_turned_into_a_reject() {
    let (unblock, block) = std::sync::mpsc::channel();
    let mut r = rig(HostKeyDecision::TrustAndRemember, Some(block), None);
    r.h.tx.send(open_compare()).unwrap(); // asks the user
    let prompt = r.prompt.clone();
    until("the question is asked", || {
        prompt.calls.load(Ordering::SeqCst) == 1
    })
    .await;
    r.h.tx.send(close_compare()).unwrap();
    // Commands are dispatched in order, so this answer proves the Close was processed.
    r.h.tx
        .send(Command::ListDatabases {
            session: EMBEDDED_SESSION,
        })
        .unwrap();
    assert!(matches!(
        next(&mut r.h).await,
        Event::Failed {
            session: EMBEDDED_SESSION,
            ..
        }
    ));
    unblock.send(()).unwrap(); // Trust and remember, too late
    assert_silent(&mut r.h).await;
    assert!(!r.known_hosts.exists(), "nothing was remembered");
}

#[tokio::test]
async fn an_open_compare_session_can_still_trust_and_remember() {
    let mut r = rig(HostKeyDecision::TrustAndRemember, None, None);
    r.h.tx.send(open_compare()).unwrap();
    assert_eq!(
        next(&mut r.h).await,
        Event::Compare(crate::compare::protocol::CompareEvent::Opened { session: 1 })
    );
    assert_eq!(r.prompt.calls.load(Ordering::SeqCst), 1);
    assert!(r.known_hosts.exists(), "trust was remembered");
}

// A connect or a connection test that ended (timeout, failure, replacement) must not leave a
// question behind that can still record trust.

fn test_cmd(request_id: u64) -> Command {
    Command::TestConnection {
        request_id,
        cfg: ssh_cfg(),
        secrets: TestSecrets::default(),
    }
}

const SHORT: Duration = Duration::from_millis(300);

fn rejected() -> CoreError {
    CoreError::Ssh("the ssh host key was rejected".into())
}

#[tokio::test]
async fn a_test_answered_after_its_timeout_records_nothing() {
    let (unblock, block) = std::sync::mpsc::channel();
    let mut r = rig_with_timeout(
        HostKeyDecision::TrustAndRemember,
        Some(block),
        None,
        Some(SHORT),
    );
    r.h.tx.send(test_cmd(1)).unwrap();
    let prompt = r.prompt.clone();
    until("the question is asked", || {
        prompt.calls.load(Ordering::SeqCst) == 1
    })
    .await;
    assert_eq!(
        next(&mut r.h).await,
        Event::TestConnection {
            request_id: 1,
            outcome: Err(CoreError::Database("timeout".into())),
        }
    );
    unblock.send(()).unwrap(); // Trust and remember, after the test already ended
    tokio::time::sleep(SHORT).await;
    assert!(!r.known_hosts.exists(), "nothing was remembered");
}

#[tokio::test]
async fn a_connect_answered_after_its_timeout_records_nothing() {
    let (unblock, block) = std::sync::mpsc::channel();
    let mut r = rig_with_timeout(
        HostKeyDecision::TrustAndRemember,
        Some(block),
        None,
        Some(SHORT),
    );
    r.h.tx.send(Command::Connect(ssh_cfg())).unwrap();
    let prompt = r.prompt.clone();
    until("the question is asked", || {
        prompt.calls.load(Ordering::SeqCst) == 1
    })
    .await;
    assert!(matches!(
        next(&mut r.h).await,
        Event::ConnectFailed { epoch: 1, .. }
    ));
    unblock.send(()).unwrap(); // Trust and remember, after the connect already failed
    tokio::time::sleep(SHORT).await;
    assert!(!r.known_hosts.exists(), "nothing was remembered");
}

#[tokio::test]
async fn a_newer_test_rejects_the_question_of_the_one_before() {
    let (release, gate) = oneshot::channel();
    // Trust once, so test 2 leaves nothing in known_hosts that would excuse test 1 from asking.
    let mut r = rig(HostKeyDecision::Trust, None, Some(gate));
    r.h.tx.send(test_cmd(1)).unwrap(); // parked in the opener
    r.entered.notified().await;
    r.h.tx.send(test_cmd(2)).unwrap(); // supersedes test 1, runs normally
    assert!(matches!(
        next(&mut r.h).await,
        Event::TestConnection {
            request_id: 2,
            outcome: Ok(_)
        }
    ));
    assert_eq!(r.prompt.calls.load(Ordering::SeqCst), 1, "test 2 asked");
    release.send(()).unwrap(); // test 1 now reaches the host-key policy
    assert_eq!(
        next(&mut r.h).await,
        Event::TestConnection {
            request_id: 1,
            outcome: Err(rejected()),
        }
    );
    assert_eq!(
        r.prompt.calls.load(Ordering::SeqCst),
        1,
        "test 1 was never asked"
    );
    assert_silent(&mut r.h).await;
}
