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

/// Accepts connections until aborted. The dialer and every per-connection
/// task are owned by the returned task, so aborting it releases the listener,
/// the dialer (and with it the SSH session) and all running copies.
pub(crate) fn spawn_accept_loop<D: Dialer>(listener: TcpListener, dialer: D) -> JoinHandle<()> {
    let dialer = Arc::new(dialer);
    tokio::spawn(async move {
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let Ok((socket, peer)) = accepted else { break };
                    connections.spawn(pump(socket, peer, dialer.clone()));
                }
                // Reap finished copies so the set does not grow unbounded.
                Some(_) = connections.join_next(), if !connections.is_empty() => {}
            }
        }
    })
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
