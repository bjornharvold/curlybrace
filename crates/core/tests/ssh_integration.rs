//! End-to-end SSH tunnel test against real containers. Ignored by default and
//! gated on `MONGOSTUDIO_IT`. Setup (any sshd that allows TCP forwarding and a
//! MongoDB reachable from it):
//!
//! ```text
//! docker network create ms-it-net
//! docker run -d --name ms-it-sshd --network ms-it-net -p 127.0.0.1:2299:2222 \
//!   -e PASSWORD_ACCESS=true -e USER_NAME=tester -e USER_PASSWORD=<pw> \
//!   lscr.io/linuxserver/openssh-server   # then set AllowTcpForwarding yes
//! docker run -d --name ms-it-mongo --network ms-it-net mongo:7
//! MONGOSTUDIO_IT=1 MONGOSTUDIO_IT_SSH_PASSWORD=<pw> \
//!   cargo test -p mongostudio-core --test ssh_integration -- --ignored
//! ```
//!
//! Optional overrides: `MONGOSTUDIO_IT_SSH_HOST` (127.0.0.1),
//! `MONGOSTUDIO_IT_SSH_PORT` (2299), `MONGOSTUDIO_IT_SSH_USER` (tester),
//! `MONGOSTUDIO_IT_TARGET` (ms-it-mongo:27017, as seen from the sshd).

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

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

#[tokio::test]
#[ignore = "needs docker containers; set MONGOSTUDIO_IT=1"]
async fn tunnel_end_to_end() {
    if std::env::var("MONGOSTUDIO_IT").is_err() {
        return;
    }
    let password = std::env::var("MONGOSTUDIO_IT_SSH_PASSWORD").expect("ssh password env");
    let ssh = SshConfig {
        host: env_or("MONGOSTUDIO_IT_SSH_HOST", "127.0.0.1"),
        port: env_or("MONGOSTUDIO_IT_SSH_PORT", "2299").parse().unwrap(),
        username: env_or("MONGOSTUDIO_IT_SSH_USER", "tester"),
        auth: SshAuth::Password,
    };
    let target = env_or("MONGOSTUDIO_IT_TARGET", "ms-it-mongo:27017");
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
    assert!(matches!(r, Err(CoreError::Ssh(_))));
    assert_eq!(
        trust.calls.load(Ordering::SeqCst),
        1,
        "only the first connect asked"
    );
}
