//! `ms-it-*` docker containers with guaranteed cleanup.
use crate::gate::external_uri;
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::Duration;

pub const MONGO_IMAGE: &str = "mongo:7";
const NAME_PREFIX: &str = "ms-it-";
const TCP_READY: Duration = Duration::from_secs(90);

/// Runs `docker <args>`; `Ok(stdout)` on success, `Err(stderr)` otherwise.
pub fn docker(args: &[&str]) -> Result<String, String> {
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

/// An unused loopback port (best effort: it is released before docker binds it).
pub fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    listener.local_addr().expect("local addr").port()
}

/// A running `ms-it-*` container, force-removed on drop.
pub struct Container {
    pub name: String,
    pub port: u16,
}

impl Container {
    /// Starts a MongoDB container on a random loopback port. `env` entries are `KEY=VALUE` and
    /// `args` are passed to `mongod`. Returns once the TCP port accepts connections; callers
    /// that need authentication or a driver ping poll on top of that.
    pub fn start_mongo(env: &[String], args: &[&str]) -> Self {
        let name = format!("{NAME_PREFIX}{}", uuid::Uuid::new_v4().simple());
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

    /// Waits until the published port accepts a TCP connection (not proof mongod is ready).
    pub fn wait_tcp(&self) {
        crate::poll(TCP_READY, "container port", || {
            std::net::TcpStream::connect(("127.0.0.1", self.port)).is_ok()
        });
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        // Exact name, and only names this crate generated.
        if self.name.starts_with(NAME_PREFIX) {
            let _ = docker(&["rm", "-f", "-v", &self.name]);
        }
    }
}

/// A plain (no auth) server: an external one from the environment, or a fresh container.
pub enum MongoServer {
    External(String),
    Owned(Container),
}

impl MongoServer {
    pub fn uri(&self) -> String {
        match self {
            Self::External(uri) => uri.clone(),
            Self::Owned(container) => container.uri(),
        }
    }

    /// The container, when this test owns one (so it may stop/start it).
    pub fn container(&self) -> Option<&Container> {
        match self {
            Self::Owned(c) => Some(c),
            Self::External(_) => None,
        }
    }
}

/// Uses `MONGOSTUDIO_IT_MONGO_URI` when set, otherwise starts a `mongo:7` container.
pub fn plain_server() -> MongoServer {
    match external_uri() {
        Some(uri) => MongoServer::External(uri),
        None => {
            let container = Container::start_mongo(&[], &[]);
            container.wait_tcp();
            MongoServer::Owned(container)
        }
    }
}
