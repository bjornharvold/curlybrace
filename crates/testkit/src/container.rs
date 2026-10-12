//! `cb-it-*` docker containers with guaranteed cleanup.
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::Duration;

pub const MONGO_IMAGE: &str = "mongo:7";
const NAME_PREFIX: &str = "cb-it-";
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
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    listener.local_addr().expect("local addr").port()
}

/// The host port docker assigned to `container_port` (e.g. `27017/tcp`) of our own container.
pub(crate) fn published_port(name: &str, container_port: &str) -> u16 {
    let out = docker(&["port", name, container_port]).expect("docker port");
    out.lines()
        .next()
        .and_then(|line| line.rsplit(':').next())
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or_else(|| panic!("cannot parse docker port output {out:?}"))
}

/// A running `cb-it-*` container, force-removed on drop.
pub struct Container {
    pub name: String,
    pub port: u16,
}

impl Container {
    /// Starts a MongoDB container; docker picks the loopback host port (`-p 127.0.0.1::27017`),
    /// which is then read back, so there is no bind-then-drop race. The port may change if the
    /// container is stopped and started again: use [`Container::start_mongo_restartable`] then.
    pub fn start_mongo(env: &[String], args: &[&str]) -> Self {
        Self::run_mongo(env, args, "127.0.0.1::27017")
    }

    /// Like [`Container::start_mongo`] but with a fixed host port that survives stop/start.
    /// Retries with a new port if the chosen one was taken in the meantime.
    pub fn start_mongo_restartable(env: &[String], args: &[&str]) -> Self {
        for attempt in 1..=3 {
            let publish = format!("127.0.0.1:{}:27017", free_port());
            if let Some(container) = Self::try_run_mongo(env, args, &publish) {
                return container;
            }
            eprintln!("fixed-port start attempt {attempt} failed, retrying with another port");
        }
        panic!("could not start a restartable mongo container");
    }

    fn run_mongo(env: &[String], args: &[&str], publish: &str) -> Self {
        Self::try_run_mongo(env, args, publish).expect("docker run failed")
    }

    fn try_run_mongo(env: &[String], args: &[&str], publish: &str) -> Option<Self> {
        let name = format!("{NAME_PREFIX}{}", uuid::Uuid::new_v4().simple());
        let mut cmd: Vec<&str> = vec!["run", "-d", "--name", &name, "-p", publish];
        for e in env {
            cmd.push("-e");
            cmd.push(e);
        }
        cmd.push(MONGO_IMAGE);
        cmd.extend_from_slice(args);
        // Create the guard first so a failed start is still cleaned up.
        let mut guard = Self {
            name: name.clone(),
            port: 0,
        };
        if let Err(e) = docker(&cmd) {
            eprintln!("docker run failed: {e}");
            return None;
        }
        guard.port = published_port(&name, "27017/tcp");
        Some(guard)
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

/// The server a test talks to: the already-running local one (tier 1) or a container this
/// test owns (tier 2).
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
