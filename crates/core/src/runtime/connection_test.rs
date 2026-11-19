//! The connection test: one throw-away connection, apart from the epoch machinery.

use std::sync::Arc;

use crate::connection::ConnectionConfig;
use crate::error::{CoreError, Result};

use super::handlers::{bounded, read_password};
use super::job_flag::JobFlag;
use super::tunneling::{Currentness, SshSecretSource, open_tunnel_if_configured};
use super::{Event, Shared};

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

/// Runs one connection test in its own task, apart from the epoch machinery. The work runs in an
/// inner task so a panic still produces exactly one answer.
pub(super) fn spawn(
    shared: Arc<Shared>,
    events: tokio::sync::mpsc::UnboundedSender<Event>,
    repaint: super::Repaint,
    request_id: u64,
    cfg: ConnectionConfig,
    secrets: super::TestSecrets,
) {
    // A newer test ends the previous one's flag, so only one test's question is ever live. A test
    // is never replaced by a Connect/Disconnect, so only this flag decides its currentness.
    let flag = JobFlag::new();
    let previous = shared
        .latest_test
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .replace(flag.clone());
    if let Some(previous) = previous {
        previous.end();
    }
    tokio::spawn(async move {
        let work = tokio::spawn(async move { test_connection(&shared, cfg, secrets, flag).await });
        let outcome = work
            .await
            .unwrap_or_else(|_| Err(CoreError::Database("internal error".into())));
        if events
            .send(Event::TestConnection {
                request_id,
                outcome,
            })
            .is_ok()
        {
            repaint();
        }
    });
}

async fn test_connection(
    shared: &Arc<Shared>,
    cfg: ConnectionConfig,
    secrets: super::TestSecrets,
    flag: JobFlag,
) -> Result<super::TestReport> {
    // Ends the test however it ends (answer, error, timeout, panic, cancellation): a host-key
    // answer arriving later is rejected, so nothing reaches known_hosts.
    let _end = flag.end_on_drop();
    cfg.validate()?;
    let limit = shared.config.connect_timeout;
    bounded(limit, async {
        let password = match &secrets.password {
            super::PasswordChoice::Typed(p) => Some(p.expose().to_owned()),
            super::PasswordChoice::Without => None,
            super::PasswordChoice::Stored => read_password(shared, &cfg).await?,
        };
        let job: Arc<dyn Currentness> = Arc::new(flag.clone());
        let ssh_secret = match &secrets.ssh_secret {
            super::PasswordChoice::Typed(s) => SshSecretSource::Typed(s.expose()),
            super::PasswordChoice::Without => SshSecretSource::Without,
            super::PasswordChoice::Stored => SshSecretSource::Stored,
        };
        let tunnel = open_tunnel_if_configured(shared, job, &cfg, ssh_secret).await?;
        let addr = tunnel.as_ref().map(|t| t.local_addr());
        let store = shared
            .connector
            .connect(&cfg, password.as_deref(), addr)
            .await?;
        let server_version = store.server_version().await?;
        // Dropped here, before the answer: a test leaves no client and no tunnel behind.
        drop(store);
        drop(tunnel);
        Ok(super::TestReport {
            server_version,
            tls: cfg.tls.enabled,
            authenticated: cfg.auth != crate::connection::AuthMechanism::None,
        })
    })
    .await
}
