//! Shared helpers for the container-backed integration tests.
//!
//! Containers are driven through the `docker` CLI and owned by a [`Container`]
//! guard whose `Drop` runs `docker rm -f`, so they are removed even when an
//! assertion panics. Every container is named `ms-it-*`; nothing else is ever
//! touched. Tests are `#[ignore]` and additionally gated on `MONGOSTUDIO_IT=1`
//! via [`it_enabled`], which prints why a test was skipped.
#![allow(dead_code)]

use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use mongodb::Client;
use mongostudio_core::client;
use mongostudio_core::connection::ConnectionConfig;
use mongostudio_core::mongo_store::MongoStore;

pub const MONGO_IMAGE: &str = "mongo:7";
const READY_TIMEOUT: Duration = Duration::from_secs(90);

/// True when integration tests should run; otherwise prints the reason.
pub fn it_enabled(test: &str) -> bool {
    if std::env::var("MONGOSTUDIO_IT").as_deref() == Ok("1") {
        return true;
    }
    eprintln!("SKIPPED {test}: set MONGOSTUDIO_IT=1 to run container-backed tests");
    false
}

fn docker(args: &[&str]) -> Result<String, String> {
    let out = Command::new("docker")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run docker: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    listener.local_addr().expect("local addr").port()
}

/// A running `ms-it-*` container, force-removed on drop.
pub struct Container {
    pub name: String,
    pub port: u16,
}

impl Container {
    /// Starts a MongoDB container on a random loopback port. `env` entries are
    /// `KEY=VALUE` and `args` are passed to `mongod`.
    pub fn start_mongo(env: &[String], args: &[&str]) -> Self {
        let name = format!("ms-it-{}", uuid::Uuid::new_v4().simple());
        let port = free_port();
        let publish = format!("127.0.0.1:{port}:27017");
        let mut cmd: Vec<&str> = vec!["run", "-d", "--name", &name, "-p", &publish];
        for e in env {
            cmd.push("-e");
            cmd.push(e);
        }
        cmd.push(MONGO_IMAGE);
        cmd.extend_from_slice(args);
        // Create the guard first so a failed start is still cleaned up.
        let guard = Self {
            name: name.clone(),
            port,
        };
        if let Err(e) = docker(&cmd) {
            panic!("docker run failed: {e}");
        }
        guard
    }

    pub fn stop(&self) {
        docker(&["stop", "-t", "1", &self.name]).expect("docker stop");
    }

    pub fn start(&self) {
        docker(&["start", &self.name]).expect("docker start");
    }

    pub fn uri(&self) -> String {
        format!("mongodb://127.0.0.1:{}", self.port)
    }

    pub fn config(&self) -> ConnectionConfig {
        ConnectionConfig::new("it", &self.uri())
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        let _ = docker(&["rm", "-f", "-v", &self.name]);
    }
}

/// Polls until the server answers a ping (and authenticates, if configured).
pub async fn wait_ready(cfg: &ConnectionConfig, password: Option<&str>) -> Client {
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        match client::connect(cfg, password, None).await {
            Ok(client) => return client,
            Err(e) if Instant::now() > deadline => panic!("mongod never became ready: {e}"),
            Err(_) => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
}

/// A plain (no auth) server plus a connected client and store.
pub async fn plain_mongo() -> (Container, Client, MongoStore) {
    let container = Container::start_mongo(&[], &[]);
    let client = wait_ready(&container.config(), None).await;
    let store = MongoStore::new(client.clone());
    (container, client, store)
}

/// Waits up to `limit` for the next runtime event; panics with context on timeout.
pub async fn next_event(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<mongostudio_core::runtime::Event>,
    limit: Duration,
    what: &str,
) -> mongostudio_core::runtime::Event {
    match tokio::time::timeout(limit, rx.recv()).await {
        Ok(Some(event)) => event,
        Ok(None) => panic!("runtime closed while waiting for {what}"),
        Err(_) => panic!("no event for {what} within {limit:?}"),
    }
}

/// A repaint callback that does nothing.
pub fn no_repaint() -> mongostudio_core::runtime::Repaint {
    std::sync::Arc::new(|| {})
}

pub const SSH_USER: &str = "tester";
pub const SSH_PASSWORD: &str = "it-ssh-pw-1";
const SSH_IMAGE: &str = "lscr.io/linuxserver/openssh-server";

/// An sshd container plus a MongoDB reachable only from it, on a private
/// network. Everything is removed on drop (containers first, then the network).
pub struct SshStack {
    network: String,
    mongo: String,
    sshd: String,
    pub ssh_port: u16,
}

impl SshStack {
    pub fn start() -> Self {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let stack = Self {
            network: format!("ms-it-net-{id}"),
            mongo: format!("ms-it-mongo-{id}"),
            sshd: format!("ms-it-sshd-{id}"),
            ssh_port: free_port(),
        };
        docker(&["network", "create", &stack.network]).expect("docker network create");
        docker(&[
            "run",
            "-d",
            "--name",
            &stack.mongo,
            "--network",
            &stack.network,
            MONGO_IMAGE,
        ])
        .expect("start mongo");
        let publish = format!("127.0.0.1:{}:2222", stack.ssh_port);
        let user = format!("USER_NAME={SSH_USER}");
        let pw = format!("USER_PASSWORD={SSH_PASSWORD}");
        docker(&[
            "run",
            "-d",
            "--name",
            &stack.sshd,
            "--network",
            &stack.network,
            "-p",
            &publish,
            "-e",
            "PASSWORD_ACCESS=true",
            "-e",
            &user,
            "-e",
            &pw,
            SSH_IMAGE,
        ])
        .expect("start sshd");
        stack.enable_forwarding();
        stack.wait_mongo();
        stack
    }

    /// The image ships with `AllowTcpForwarding no`; enable it and reload sshd.
    fn enable_forwarding(&self) {
        let script = "test -s /config/sshd.pid && \
            sed -i 's/^AllowTcpForwarding no/AllowTcpForwarding yes/' /config/sshd/sshd_config && \
            kill -HUP $(cat /config/sshd.pid)";
        poll(Duration::from_secs(60), "sshd config", || {
            docker(&["exec", &self.sshd, "sh", "-c", script]).is_ok()
        });
        // Give the reloaded daemon a moment to re-bind before the first dial.
        poll(Duration::from_secs(30), "sshd port", || {
            std::net::TcpStream::connect(("127.0.0.1", self.ssh_port)).is_ok()
        });
    }

    fn wait_mongo(&self) {
        let ping = "db.runCommand({ping:1}).ok";
        poll(Duration::from_secs(90), "mongo in ssh stack", || {
            docker(&["exec", &self.mongo, "mongosh", "--quiet", "--eval", ping])
                .is_ok_and(|out| out.ends_with('1'))
        });
    }

    /// The Mongo address as the sshd container sees it.
    pub fn target(&self) -> String {
        format!("{}:27017", self.mongo)
    }
}

fn poll(limit: Duration, what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(500));
    }
}

impl Drop for SshStack {
    fn drop(&mut self) {
        for name in [&self.sshd, &self.mongo] {
            let _ = docker(&["rm", "-f", "-v", name]);
        }
        let _ = docker(&["network", "rm", &self.network]);
    }
}
