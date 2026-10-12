//! End-to-end SSH tunnel test. It starts its own `ms-it-*` containers (an
//! openssh-server and a MongoDB on a private network, see `support::SshStack`),
//! removes them on drop, and is gated on `MONGOSTUDIO_IT=1`.

mod support;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use mongostudio_core::client;
use mongostudio_core::connection::{ConnectionConfig, SshAuth, SshConfig};
use mongostudio_core::error::CoreError;
use mongostudio_core::tunnel::{
    HostKeyDecision, HostKeyPrompt, RejectUnknownHostKeys, open_tunnel,
};
use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::{PrivateKey, known_hosts};

struct CountingPrompt {
    answer: HostKeyDecision,
    calls: AtomicUsize,
}

impl HostKeyPrompt for CountingPrompt {
    fn decide(&self, _host: &str, fingerprint: &str) -> HostKeyDecision {
        assert!(fingerprint.starts_with("SHA256:"));
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.answer
    }
}

#[tokio::test]
#[ignore = "needs docker; set MONGOSTUDIO_IT=1"]
async fn tunnel_end_to_end() {
    if !support::it_enabled("tunnel_end_to_end") {
        return;
    }
    let stack = support::SshStack::start();
    let password = support::SSH_PASSWORD.to_string();
    let ssh = SshConfig {
        host: "127.0.0.1".into(),
        port: stack.ssh_port,
        username: support::SSH_USER.into(),
        auth: SshAuth::Password,
    };
    let target = stack.target();
    let (thost, tport) = target.split_once(':').unwrap();
    let tport: u16 = tport.parse().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let known = dir.path().join("known_hosts");

    // Unknown host + reject-by-default prompt: refused, nothing recorded.
    let r = open_tunnel(
        &ssh,
        thost,
        tport,
        Some(&password),
        &known,
        &RejectUnknownHostKeys,
    )
    .await;
    assert!(matches!(r, Err(CoreError::Ssh(_))));
    assert!(!known.exists());

    // Wrong password with a trusted key: typed error, no secret in the text.
    let trust = CountingPrompt {
        answer: HostKeyDecision::TrustAndRemember,
        calls: AtomicUsize::new(0),
    };
    let r = open_tunnel(&ssh, thost, tport, Some("definitely-wrong"), &known, &trust).await;
    let Err(CoreError::Ssh(text)) = r else {
        panic!("wrong password must fail");
    };
    assert!(!text.contains("definitely-wrong"));

    // Known now: right password connects without prompting, MongoDB works.
    let reject = CountingPrompt {
        answer: HostKeyDecision::Reject,
        calls: AtomicUsize::new(0),
    };
    let tunnel = open_tunnel(&ssh, thost, tport, Some(&password), &known, &reject)
        .await
        .expect("tunnel opens");
    assert_eq!(reject.calls.load(Ordering::SeqCst), 0);
    assert!(tunnel.local_addr.ip().is_loopback());
    let cfg = ConnectionConfig::new("it", &format!("mongodb://{target}"));
    let driver = client::connect(&cfg, None, Some(tunnel.local_addr))
        .await
        .expect("mongo reachable through the tunnel");
    drop(driver);

    // Drop closes the local port.
    let addr: SocketAddr = tunnel.local_addr;
    drop(tunnel);
    let mut closed = false;
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(addr).await.is_err() {
            closed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(closed, "port still open after drop");

    // A changed key is refused without ever asking.
    let other = PrivateKey::from(Ed25519Keypair::from_seed(&[7; 32]));
    let changed = dir.path().join("changed_known_hosts");
    known_hosts::learn_known_hosts_path(&ssh.host, ssh.port, other.public_key(), &changed).unwrap();
    let r = open_tunnel(&ssh, thost, tport, Some(&password), &changed, &trust).await;
    assert!(matches!(r, Err(CoreError::SshHostKeyChanged)));
    assert_eq!(
        trust.calls.load(Ordering::SeqCst),
        1,
        "only the first connect asked"
    );
}
