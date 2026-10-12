//! The SSH side: connect with host-key verification, authenticate, and open
//! `direct-tcpip` channels. Every error is a static, secret-free message.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use russh::ChannelStream;
use russh::client::{self, Handle, Msg};
use russh::keys::{PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate, load_secret_key};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use super::forward::Dialer;
use crate::connection::{SshAuth, SshConfig};
use crate::error::{CoreError, Result};

/// Bound for TCP connect + key exchange + authentication. Time spent waiting
/// for the user to answer a host-key prompt is not counted.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const KEEPALIVE_MAX_MISSED: usize = 3;

fn ssh_err(message: &str) -> CoreError {
    CoreError::Ssh(message.into())
}

struct HostKeyRequest {
    key: PublicKey,
    reply: oneshot::Sender<bool>,
}

/// russh client handler: forwards the server key to the connecting task,
/// which owns the policy (`verify`) and the prompt.
struct ClientHandler {
    requests: mpsc::Sender<HostKeyRequest>,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        // Host certificates are not supported; only plain keys can be pinned.
        let PublicKeyOrCertificate::PublicKey { key, .. } = server_public_key else {
            return Ok(false);
        };
        let (reply, answer) = oneshot::channel();
        let request = HostKeyRequest {
            key: key.clone(),
            reply,
        };
        self.requests
            .send(request)
            .await
            .map_err(|_| russh::Error::SendError)?;
        // A dropped reply (cancelled connect) counts as a rejection.
        Ok(answer.await.unwrap_or(false))
    }
}

/// An authenticated session that dials `target` through direct-tcpip.
pub(crate) struct SshDialer {
    handle: Handle<ClientHandler>,
    target_host: String,
    target_port: u32,
}

impl Dialer for SshDialer {
    type Stream = ChannelStream<Msg>;

    async fn dial(&self, originator: SocketAddr) -> Result<Self::Stream> {
        self.handle
            .channel_open_direct_tcpip(
                self.target_host.clone(),
                self.target_port,
                originator.ip().to_string(),
                u32::from(originator.port()),
            )
            .await
            .map(|channel| channel.into_stream())
            .map_err(|_| ssh_err("the ssh server refused the forwarding channel"))
    }
}

/// Connects, verifies the host key via `verify`, authenticates.
pub(crate) async fn establish<V, Fut>(
    ssh: &SshConfig,
    target_host: &str,
    target_port: u16,
    secret: Option<&str>,
    mut verify: V,
) -> Result<SshDialer>
where
    V: FnMut(PublicKey) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let mut deadline = Instant::now() + CONNECT_TIMEOUT;
    let (mut handle, deadline) = connect(ssh, &mut verify, &mut deadline).await?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    tokio::time::timeout(remaining, authenticate(&mut handle, ssh, secret))
        .await
        .map_err(|_| ssh_err("the ssh connection timed out"))??;
    Ok(SshDialer {
        handle,
        target_host: target_host.to_string(),
        target_port: u32::from(target_port),
    })
}

/// Drives the handshake while servicing host-key requests. The deadline is
/// pushed back by the time a request took (the user may be deciding).
async fn connect<V, Fut>(
    ssh: &SshConfig,
    verify: &mut V,
    deadline: &mut Instant,
) -> Result<(Handle<ClientHandler>, Instant)>
where
    V: FnMut(PublicKey) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let (requests, mut incoming) = mpsc::channel::<HostKeyRequest>(1);
    let config = Arc::new(client::Config {
        keepalive_interval: Some(KEEPALIVE_INTERVAL),
        keepalive_max: KEEPALIVE_MAX_MISSED,
        ..Default::default()
    });
    let handshake = client::connect(
        config,
        (ssh.host.as_str(), ssh.port),
        ClientHandler { requests },
    );
    tokio::pin!(handshake);
    let mut host_key_error: Option<CoreError> = None;
    loop {
        tokio::select! {
            outcome = &mut handshake => {
                let handle = outcome.map_err(|_| {
                    host_key_error
                        .take()
                        .unwrap_or_else(|| ssh_err("could not connect to the ssh server"))
                })?;
                return Ok((handle, *deadline));
            }
            Some(request) = incoming.recv() => {
                let started = Instant::now();
                let verdict = verify(request.key).await;
                *deadline += started.elapsed();
                let _ = request.reply.send(verdict.is_ok());
                host_key_error = verdict.err();
            }
            () = tokio::time::sleep_until(*deadline) => {
                return Err(ssh_err("the ssh connection timed out"));
            }
        }
    }
}

async fn authenticate(
    handle: &mut Handle<ClientHandler>,
    ssh: &SshConfig,
    secret: Option<&str>,
) -> Result<()> {
    let outcome = match &ssh.auth {
        SshAuth::Password => {
            let password = secret.ok_or_else(|| ssh_err("the ssh password is missing"))?;
            handle
                .authenticate_password(ssh.username.as_str(), password)
                .await
        }
        SshAuth::PrivateKey { path } => {
            let key = Arc::new(load_key(path, secret).await?);
            let hash = handle
                .best_supported_rsa_hash()
                .await
                .ok()
                .flatten()
                .flatten();
            handle
                .authenticate_publickey(
                    ssh.username.as_str(),
                    PrivateKeyWithHashAlg::new(key, hash),
                )
                .await
        }
    };
    match outcome {
        Ok(result) if result.success() => Ok(()),
        _ => Err(ssh_err("ssh authentication failed")),
    }
}

/// Decoding may run a slow key-derivation function, so it runs off the
/// async workers. Neither the path nor the passphrase appears in errors.
async fn load_key(
    path: &std::path::Path,
    passphrase: Option<&str>,
) -> Result<russh::keys::PrivateKey> {
    let path = path.to_path_buf();
    let passphrase = passphrase.map(str::to_owned);
    tokio::task::spawn_blocking(move || {
        if std::fs::metadata(&path).is_err() {
            return Err(ssh_err("the ssh private key file could not be read"));
        }
        load_secret_key(&path, passphrase.as_deref())
            .map_err(|_| ssh_err("the ssh private key could not be decoded (check the passphrase)"))
    })
    .await
    .map_err(|_| ssh_err("the ssh private key could not be loaded"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_key_file_is_a_typed_error_without_the_path() {
        let err = load_key(std::path::Path::new("/nonexistent/secret-key-name"), None)
            .await
            .unwrap_err();
        assert!(matches!(&err, CoreError::Ssh(m) if !m.contains("secret-key-name")));
    }

    #[tokio::test]
    async fn garbage_key_file_is_a_typed_error_without_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("id_bad");
        std::fs::write(&path, "TOP-SECRET-NOT-A-KEY").unwrap();
        let err = load_key(&path, Some("hunter2")).await.unwrap_err();
        let text = err.to_string();
        assert!(matches!(err, CoreError::Ssh(_)));
        assert!(!text.contains("TOP-SECRET") && !text.contains("hunter2"));
    }
}
