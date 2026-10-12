//! SSH local-forward tunnel: binds `127.0.0.1:<ephemeral>` and forwards each
//! accepted connection through an SSH `direct-tcpip` channel to the target.
//!
//! * Host keys are checked against an app-owned `known_hosts`; a CHANGED key
//!   is always refused without asking ([`check_host_key`]).
//! * Connecting and authenticating is bounded by 10 s (prompt time excluded).
//! * Dropping a [`Tunnel`] closes the listener, all forwards and the session.
//! * Errors are static [`CoreError::Ssh`] texts: no secrets, paths or hosts.
//!
//! TLS note: the MongoDB driver (rustls) verifies the server certificate
//! against the address it dials, which is `127.0.0.1` for a tunnel, and has
//! no server-name override; see `client::build_client_options`.

mod forward;
mod host_key;
mod ssh;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::task::JoinHandle;

pub use host_key::{HostKeyDecision, HostKeyPrompt, RejectUnknownHostKeys, check_host_key};
pub use russh::keys::PublicKey;

use crate::connection::SshConfig;
use crate::error::{CoreError, Result};

/// A live tunnel. Dropping it tears everything down; the local port stops
/// accepting as soon as the aborted accept task is released by the runtime.
pub struct Tunnel {
    pub local_addr: SocketAddr,
    task: JoinHandle<()>,
}

impl Tunnel {
    /// False once the accept loop has ended (the local listener failed repeatedly), after which
    /// the tunnel no longer serves connections.
    pub fn is_running(&self) -> bool {
        !self.task.is_finished()
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        // The task owns the listener, the SSH session and every forward.
        self.task.abort();
    }
}

impl std::fmt::Debug for Tunnel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tunnel")
            .field("local_addr", &self.local_addr)
            .finish_non_exhaustive()
    }
}

/// Opens a tunnel, consulting `prompt` on the calling task. A blocking prompt
/// therefore blocks this future's thread; async callers should prefer
/// [`open_tunnel_shared`], which isolates it with `spawn_blocking`.
pub async fn open_tunnel(
    ssh: &SshConfig,
    target_host: &str,
    target_port: u16,
    secret: Option<&str>,
    known_hosts: &Path,
    prompt: &dyn HostKeyPrompt,
) -> Result<Tunnel> {
    let verify = |key: PublicKey| {
        let result = check_host_key(known_hosts, &ssh.host, ssh.port, &key, prompt);
        async move { result }
    };
    let dialer = ssh::establish(ssh, target_host, target_port, secret, verify).await?;
    start(dialer).await
}

/// Like [`open_tunnel`], but the (possibly blocking) prompt and the
/// `known_hosts` file I/O run on the blocking pool.
pub async fn open_tunnel_shared(
    ssh: &SshConfig,
    target_host: &str,
    target_port: u16,
    secret: Option<&str>,
    known_hosts: &Path,
    prompt: Arc<dyn HostKeyPrompt>,
) -> Result<Tunnel> {
    let known_hosts: PathBuf = known_hosts.to_path_buf();
    let (host, port) = (ssh.host.clone(), ssh.port);
    let verify = move |key: PublicKey| {
        let (known_hosts, host, prompt) = (known_hosts.clone(), host.clone(), prompt.clone());
        async move {
            tokio::task::spawn_blocking(move || {
                check_host_key(&known_hosts, &host, port, &key, prompt.as_ref())
            })
            .await
            .map_err(|_| CoreError::Ssh("the ssh host key check failed".into()))?
        }
    };
    let dialer = ssh::establish(ssh, target_host, target_port, secret, verify).await?;
    start(dialer).await
}

async fn start(dialer: ssh::SshDialer) -> Result<Tunnel> {
    let listener = forward::bind_loopback().await?;
    let local_addr = listener
        .local_addr()
        .map_err(|_| CoreError::Ssh("could not open a local tunnel port".into()))?;
    let task = forward::spawn_accept_loop(listener, dialer);
    Ok(Tunnel { local_addr, task })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::SshAuth;
    use std::time::{Duration, Instant};
    use tokio::net::{TcpListener, TcpStream};

    fn ssh_to(port: u16, auth: SshAuth) -> SshConfig {
        SshConfig {
            host: "127.0.0.1".into(),
            port,
            username: "user".into(),
            auth,
        }
    }

    async fn closed_port() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        l.local_addr().unwrap().port()
    }

    #[tokio::test]
    async fn closed_ssh_port_fails_with_ssh_error_within_ten_seconds() {
        let dir = tempfile::tempdir().unwrap();
        let ssh = ssh_to(closed_port().await, SshAuth::Password);
        let start = Instant::now();
        let r = open_tunnel(
            &ssh,
            "db.internal",
            27017,
            Some("pw"),
            &dir.path().join("kh"),
            &RejectUnknownHostKeys,
        )
        .await;
        assert!(matches!(r, Err(CoreError::Ssh(_))));
        assert!(start.elapsed() < Duration::from_secs(10));
    }

    #[tokio::test]
    async fn shared_variant_also_reports_closed_port_as_ssh_error() {
        let dir = tempfile::tempdir().unwrap();
        let ssh = ssh_to(closed_port().await, SshAuth::Password);
        let r = open_tunnel_shared(
            &ssh,
            "db.internal",
            27017,
            None,
            &dir.path().join("kh"),
            Arc::new(RejectUnknownHostKeys),
        )
        .await;
        assert!(matches!(r, Err(CoreError::Ssh(_))));
    }

    struct NeverDial;
    impl forward::Dialer for NeverDial {
        type Stream = TcpStream;
        async fn dial(&self, _: SocketAddr) -> Result<TcpStream> {
            Err(CoreError::Ssh("no".into()))
        }
    }

    #[tokio::test]
    async fn dropping_the_tunnel_closes_the_local_port() {
        let listener = forward::bind_loopback().await.unwrap();
        let local_addr = listener.local_addr().unwrap();
        assert!(local_addr.ip().is_loopback());
        let task = forward::spawn_accept_loop(listener, NeverDial);
        let tunnel = Tunnel { local_addr, task };
        assert!(TcpStream::connect(tunnel.local_addr).await.is_ok());
        drop(tunnel);
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if TcpStream::connect(local_addr).await.is_err() {
                break;
            }
            assert!(Instant::now() < deadline, "port still accepting after drop");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
