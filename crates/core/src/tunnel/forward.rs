//! Local listener and byte pump. Generic over how the far side is reached so
//! it can be tested without an SSH server.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

use crate::error::{CoreError, Result};

/// Bound for opening the far-side channel of one forwarded connection.
pub(crate) const DIAL_TIMEOUT: Duration = Duration::from_secs(10);

/// Opens the far side of one forwarded connection.
pub(crate) trait Dialer: Send + Sync + 'static {
    type Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static;

    fn dial(&self, originator: SocketAddr) -> impl Future<Output = Result<Self::Stream>> + Send;
}

/// Binds loopback only (never `0.0.0.0`) on an ephemeral port.
pub(crate) async fn bind_loopback() -> Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|_| CoreError::Ssh("could not open a local tunnel port".into()))
}

/// Pause after a failed `accept` before trying again (no busy loop on resource exhaustion).
pub(crate) const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
/// Consecutive non-transient `accept` failures after which the tunnel gives up.
pub(crate) const MAX_CONSECUTIVE_ACCEPT_FAILURES: u32 = 20;

/// What to do after a failed `accept`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum AcceptOutcome {
    /// Wait this long, then accept again.
    Continue(Duration),
    /// Give up: the listener is unusable. The accept task ends, which `Tunnel::is_running`
    /// reports.
    Stop,
}

/// Pure policy. Per-connection hiccups (aborted/reset handshakes, interrupts) and descriptor
/// exhaustion (EMFILE 24 / ENFILE 23) are transient and never end the tunnel; any other error
/// does only after `MAX_CONSECUTIVE_ACCEPT_FAILURES` failures in a row.
pub(crate) fn classify_accept_error(
    error: &std::io::Error,
    consecutive_failures: u32,
    backoff: Duration,
) -> AcceptOutcome {
    use std::io::ErrorKind::{
        ConnectionAborted, ConnectionReset, Interrupted, TimedOut, WouldBlock,
    };
    let transient = matches!(
        error.kind(),
        ConnectionAborted | ConnectionReset | Interrupted | WouldBlock | TimedOut
    ) || matches!(error.raw_os_error(), Some(23 | 24));
    if transient || consecutive_failures < MAX_CONSECUTIVE_ACCEPT_FAILURES {
        AcceptOutcome::Continue(backoff)
    } else {
        AcceptOutcome::Stop
    }
}

/// Source of inbound local connections (a `TcpListener`; scripted in tests).
pub(crate) trait Acceptor: Send + 'static {
    fn accept(&self) -> impl Future<Output = std::io::Result<(TcpStream, SocketAddr)>> + Send;
}

impl Acceptor for TcpListener {
    async fn accept(&self) -> std::io::Result<(TcpStream, SocketAddr)> {
        TcpListener::accept(self).await
    }
}

/// Accepts connections until aborted. The dialer and every per-connection
/// task are owned by the returned task, so aborting it releases the listener,
/// the dialer (and with it the SSH session) and all running copies.
pub(crate) fn spawn_accept_loop<D: Dialer>(listener: TcpListener, dialer: D) -> JoinHandle<()> {
    tokio::spawn(accept_loop(listener, dialer, ACCEPT_BACKOFF))
}

async fn accept_loop<A: Acceptor, D: Dialer>(acceptor: A, dialer: D, backoff: Duration) {
    let dialer = Arc::new(dialer);
    let mut connections = JoinSet::new();
    let mut failures = 0u32;
    loop {
        tokio::select! {
            accepted = acceptor.accept() => match accepted {
                Ok((socket, peer)) => {
                    failures = 0;
                    connections.spawn(pump(socket, peer, dialer.clone()));
                }
                Err(error) => {
                    failures += 1;
                    match classify_accept_error(&error, failures, backoff) {
                        AcceptOutcome::Continue(pause) => {
                            log::warn!("tunnel accept failed ({:?}); retrying", error.kind());
                            tokio::time::sleep(pause).await;
                        }
                        AcceptOutcome::Stop => {
                            log::error!("tunnel accept failed repeatedly ({:?}); closing", error.kind());
                            break;
                        }
                    }
                }
            },
            // Reap finished copies so the set does not grow unbounded.
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
}

/// A failed dial or copy only closes this one local connection.
async fn pump<D: Dialer>(mut local: TcpStream, peer: SocketAddr, dialer: Arc<D>) {
    let dialed = tokio::time::timeout(DIAL_TIMEOUT, dialer.dial(peer)).await;
    let Ok(Ok(mut remote)) = dialed else { return };
    let _ = tokio::io::copy_bidirectional(&mut local, &mut remote).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use std::collections::VecDeque;
    use std::io;
    use std::sync::Mutex;

    #[test]
    fn transient_and_resource_errors_always_continue_with_a_backoff() {
        let pause = Duration::from_millis(100);
        let transient = [
            io::Error::from(io::ErrorKind::ConnectionAborted),
            io::Error::from(io::ErrorKind::Interrupted),
            io::Error::from(io::ErrorKind::ConnectionReset),
            io::Error::from_raw_os_error(24), // EMFILE
            io::Error::from_raw_os_error(23), // ENFILE
        ];
        for e in &transient {
            assert_eq!(
                classify_accept_error(e, 0, pause),
                AcceptOutcome::Continue(pause)
            );
            assert_eq!(
                classify_accept_error(e, 10_000, pause),
                AcceptOutcome::Continue(pause),
                "{e:?}"
            );
        }
    }

    #[test]
    fn other_errors_stop_only_after_repeated_consecutive_failures() {
        let pause = Duration::from_millis(100);
        let e = io::Error::from(io::ErrorKind::PermissionDenied);
        assert_eq!(
            classify_accept_error(&e, MAX_CONSECUTIVE_ACCEPT_FAILURES - 1, pause),
            AcceptOutcome::Continue(pause)
        );
        assert_eq!(
            classify_accept_error(&e, MAX_CONSECUTIVE_ACCEPT_FAILURES, pause),
            AcceptOutcome::Stop
        );
    }

    /// Plays back a script of accept results, then blocks forever.
    struct Scripted(Mutex<VecDeque<io::Result<(TcpStream, SocketAddr)>>>);

    impl Acceptor for Scripted {
        async fn accept(&self) -> io::Result<(TcpStream, SocketAddr)> {
            let next = self.0.lock().unwrap().pop_front();
            match next {
                Some(result) => result,
                None => std::future::pending().await,
            }
        }
    }

    async fn socket_pair() -> (TcpStream, TcpStream, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).await.unwrap();
        let (server, peer) = listener.accept().await.unwrap();
        (client, server, peer)
    }

    #[tokio::test]
    async fn the_loop_survives_transient_accept_errors_and_still_serves_connections() {
        let echo = echo_server().await;
        let (mut client, server, peer) = socket_pair().await;
        let script = VecDeque::from([
            Err(io::Error::from(io::ErrorKind::ConnectionAborted)),
            Err(io::Error::from_raw_os_error(24)),
            Ok((server, peer)),
        ]);
        let task = accept_loop(
            Scripted(Mutex::new(script)),
            TcpDialer(echo),
            Duration::ZERO,
        );
        let handle = tokio::spawn(task);
        client.write_all(b"ping").await.unwrap();
        let mut buf = [0u8; 4];
        client.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");
        assert!(!handle.is_finished(), "the tunnel is still accepting");
        handle.abort();
    }

    #[tokio::test]
    async fn persistent_non_transient_errors_end_the_loop_observably() {
        let script = (0..=MAX_CONSECUTIVE_ACCEPT_FAILURES)
            .map(|_| Err(io::Error::from(io::ErrorKind::PermissionDenied)))
            .collect();
        let echo = echo_server().await;
        let handle = tokio::spawn(accept_loop(
            Scripted(Mutex::new(script)),
            TcpDialer(echo),
            Duration::ZERO,
        ));
        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("loop ends")
            .unwrap();
    }

    struct TcpDialer(SocketAddr);

    impl Dialer for TcpDialer {
        type Stream = TcpStream;
        async fn dial(&self, _originator: SocketAddr) -> Result<TcpStream> {
            TcpStream::connect(self.0)
                .await
                .map_err(|_| CoreError::Ssh("dial failed".into()))
        }
    }

    async fn echo_server() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let (mut r, mut w) = s.split();
                    let _ = tokio::io::copy(&mut r, &mut w).await;
                });
            }
        });
        addr
    }

    #[tokio::test]
    async fn bytes_flow_both_ways_through_the_forwarder() {
        let echo = echo_server().await;
        let listener = bind_loopback().await.unwrap();
        let local = listener.local_addr().unwrap();
        assert!(local.ip().is_loopback());
        let task = spawn_accept_loop(listener, TcpDialer(echo));

        let mut client = TcpStream::connect(local).await.unwrap();
        client.write_all(b"ping").await.unwrap();
        let mut buf = [0u8; 4];
        client.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"ping");
        task.abort();
    }

    #[tokio::test]
    async fn aborting_the_accept_loop_closes_the_port() {
        let echo = echo_server().await;
        let listener = bind_loopback().await.unwrap();
        let local = listener.local_addr().unwrap();
        let task = spawn_accept_loop(listener, TcpDialer(echo));
        task.abort();
        let _ = task.await;
        assert!(TcpStream::connect(local).await.is_err());
    }

    #[tokio::test]
    async fn failed_dial_closes_only_that_connection() {
        let dead: SocketAddr = {
            let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
            l.local_addr().unwrap()
        };
        let listener = bind_loopback().await.unwrap();
        let local = listener.local_addr().unwrap();
        let task = spawn_accept_loop(listener, TcpDialer(dead));
        let mut client = TcpStream::connect(local).await.unwrap();
        let mut buf = [0u8; 1];
        assert_eq!(client.read(&mut buf).await.unwrap(), 0);
        // The listener keeps serving.
        assert!(TcpStream::connect(local).await.is_ok());
        task.abort();
    }
}
