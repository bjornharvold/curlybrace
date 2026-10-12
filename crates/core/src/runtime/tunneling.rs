//! Runtime side of the SSH tunnel: the injectable opener, URI target
//! derivation, the default `known_hosts` location and the connect-time glue.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use crate::connection::{ConnectionConfig, SshAuth, SshConfig};
use crate::error::{CoreError, Result};
use crate::secrets::{SecretKind, secret_key};
use crate::tunnel::{HostKeyPrompt, Tunnel, open_tunnel_shared};

use super::Shared;

const DEFAULT_MONGO_PORT: u16 = 27017;

/// A live tunnel owned by the runtime. Dropping it closes the tunnel.
pub trait TunnelHandle: Send + Sync {
    fn local_addr(&self) -> SocketAddr;
}

impl TunnelHandle for Tunnel {
    fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

/// Everything needed to open one tunnel.
pub struct TunnelRequest<'a> {
    pub ssh: &'a SshConfig,
    pub target_host: &'a str,
    pub target_port: u16,
    /// SSH password, or the key passphrase for private-key auth.
    pub secret: Option<&'a str>,
    pub known_hosts: &'a Path,
    pub prompt: Arc<dyn HostKeyPrompt>,
}

/// Opens tunnels. Injectable so runtime tests need no SSH server.
#[async_trait]
pub trait TunnelOpener: Send + Sync {
    async fn open(&self, request: TunnelRequest<'_>) -> Result<Box<dyn TunnelHandle>>;
}

/// Default opener: a real SSH session. The host-key prompt runs on the
/// blocking pool, so a UI implementation may block until the user answers.
pub struct SshTunnelOpener;

#[async_trait]
impl TunnelOpener for SshTunnelOpener {
    async fn open(&self, request: TunnelRequest<'_>) -> Result<Box<dyn TunnelHandle>> {
        let tunnel = open_tunnel_shared(
            request.ssh,
            request.target_host,
            request.target_port,
            request.secret,
            request.known_hosts,
            request.prompt,
        )
        .await?;
        Ok(Box::new(tunnel))
    }
}

/// App-owned `known_hosts`, e.g. `~/Library/Application Support/mongostudio/known_hosts`
/// on macOS, never the user's `~/.ssh/known_hosts`. `None` when no config directory can be
/// determined: there is deliberately NO fallback to a relative path (it would resolve against
/// the working directory), and an SSH connect without a location fails closed.
pub fn default_known_hosts_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("mongostudio").join("known_hosts"))
}

/// Derives the single `host:port` a tunnel must reach from the URI authority.
/// Multi-host and `mongodb+srv` URIs cannot be tunnelled through one forward.
pub(super) fn parse_target(uri: &str) -> Result<(String, u16)> {
    if uri.starts_with("mongodb+srv://") {
        return Err(CoreError::Unsupported(
            "ssh tunnels cannot be combined with mongodb+srv uris".into(),
        ));
    }
    let invalid = || CoreError::InvalidUri("the connection uri has no usable host".into());
    let rest = uri.strip_prefix("mongodb://").ok_or_else(invalid)?;
    let authority = rest.split(['/', '?']).next().unwrap_or_default();
    let hosts = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if hosts.contains(',') {
        return Err(CoreError::Unsupported(
            "ssh tunnels support a single host only, not multi-host uris".into(),
        ));
    }
    split_host_port(hosts).ok_or_else(invalid)
}

fn split_host_port(hosts: &str) -> Option<(String, u16)> {
    let (host, port) = if let Some(bracketed) = hosts.strip_prefix('[') {
        let (host, tail) = bracketed.split_once(']')?;
        (host, tail.strip_prefix(':'))
    } else {
        match hosts.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (hosts, None),
        }
    };
    if host.is_empty() {
        return None;
    }
    let port = match port {
        None => DEFAULT_MONGO_PORT,
        Some(p) => p.parse().ok()?,
    };
    Some((host.to_string(), port))
}

/// Opens the configured tunnel, if any. The secret is read from the keychain
/// off the async workers; an absent secret is passed on as `None`.
pub(super) async fn open_tunnel_if_configured(
    shared: &Shared,
    cfg: &ConnectionConfig,
) -> Result<Option<Box<dyn TunnelHandle>>> {
    let Some(ssh) = &cfg.ssh else {
        return Ok(None);
    };
    let (target_host, target_port) = parse_target(&cfg.uri)?;
    let secret = read_ssh_secret(shared, cfg, ssh).await?;
    let config = &shared.config;
    let known_hosts = config.known_hosts_path.as_deref().ok_or_else(|| {
        CoreError::Ssh("the location for known ssh hosts could not be determined".into())
    })?;
    let request = TunnelRequest {
        ssh,
        target_host: &target_host,
        target_port,
        secret: secret.as_deref(),
        known_hosts,
        prompt: config.host_key_prompt.clone(),
    };
    config.tunnel_opener.open(request).await.map(Some)
}

async fn read_ssh_secret(
    shared: &Shared,
    cfg: &ConnectionConfig,
    ssh: &SshConfig,
) -> Result<Option<String>> {
    let Some(secrets) = shared.secrets.clone() else {
        return Ok(None);
    };
    let kind = match ssh.auth {
        SshAuth::Password => SecretKind::SshPassword,
        SshAuth::PrivateKey { .. } => SecretKind::SshPassphrase,
    };
    let key = secret_key(cfg.id, kind);
    tokio::task::spawn_blocking(move || secrets.get(&key))
        .await
        .map_err(|_| CoreError::Secret("secret lookup failed".into()))?
}
