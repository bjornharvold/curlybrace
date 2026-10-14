//! Runs a generated script with the REAL `mongosh` inside a throwaway container of our own.
//!
//! Safety rules (the user's own containers must never be touched):
//! * the container name comes from `CURLYBRACE_IT_MONGOSH_CONTAINER` and must start with
//!   `cb-it-dev-` (letters, digits, `-`, `_` only); anything else is refused with a panic;
//! * the only docker verbs used are `docker port <name>` and `docker exec -i <name> mongosh ...`
//!   on that validated name;
//! * the container's published port must be the address of `CURLYBRACE_IT_MONGODB_URI`, so the
//!   script runs against the very server the test seeded and checks;
//! * the connection string inside the container is always the container's own
//!   `127.0.0.1:27017`, and the default database is a `cbit_*` database.
#![allow(dead_code)]

use std::io::Write;
use std::process::{Command, Stdio};

pub const ENV_MONGOSH_CONTAINER: &str = "CURLYBRACE_IT_MONGOSH_CONTAINER";
pub const CONTAINER_PREFIX: &str = "cb-it-dev-";

/// Why a container name is refused (None = accepted).
pub fn container_name_problem(name: &str) -> Option<&'static str> {
    if !name.starts_with(CONTAINER_PREFIX) || name.len() == CONTAINER_PREFIX.len() {
        return Some("the container name must start with cb-it-dev-");
    }
    let ok = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        None
    } else {
        Some("the container name may only hold letters, digits, - and _")
    }
}

/// The container to run `mongosh` in, or None (with a message) when the variable is unset.
/// An invalid name is a misconfiguration and panics.
pub fn mongosh_container(test: &str) -> Option<String> {
    let name = std::env::var(ENV_MONGOSH_CONTAINER).unwrap_or_default();
    let name = name.trim();
    if name.is_empty() {
        eprintln!("SKIPPED {test}: set {ENV_MONGOSH_CONTAINER} to your own cb-it-dev-* container");
        return None;
    }
    if let Some(problem) = container_name_problem(name) {
        panic!("{ENV_MONGOSH_CONTAINER}: {problem}");
    }
    Some(name.to_string())
}

/// `host:port` of a `mongodb://host:port[/...]` URI (no credentials expected).
pub fn authority(uri: &str) -> String {
    let rest = uri.split("://").nth(1).unwrap_or(uri);
    let rest = rest.rsplit('@').next().unwrap_or(rest);
    rest.split(['/', '?']).next().unwrap_or(rest).to_string()
}

fn assert_container_serves_the_test_server(container: &str) {
    let uri = curlybrace_testkit::local_uri();
    let out = Command::new("docker")
        .args(["port", container, "27017/tcp"])
        .output()
        .expect("docker is not available");
    let published = String::from_utf8_lossy(&out.stdout);
    let want = authority(&uri);
    assert!(
        out.status.success() && published.lines().any(|l| l.trim() == want),
        "container {container} does not publish 27017 as {want} (it publishes {:?}); \
         {ENV_MONGOSH_CONTAINER} must be the container behind CURLYBRACE_IT_MONGODB_URI",
        published.trim()
    );
}

/// What a `mongosh` run produced. `mongosh` reading a script from stdin keeps going after a
/// failed statement (it is a REPL), so judge by the database and by [`Self::output`], not only
/// by the exit status.
#[derive(Debug)]
pub struct MongoshRun {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl MongoshRun {
    pub fn output(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }

    pub fn succeeded(&self) -> bool {
        self.code == Some(0)
    }

    /// True when the shell printed an error (MongoServerError, SyntaxError, ...).
    pub fn printed_error(&self) -> bool {
        let out = self.output();
        out.contains("Error") || out.contains("error:")
    }
}

/// `docker exec -i <container> mongosh --quiet <own uri> < script`, with `default_db` (a `cbit_*`
/// database) as the shell's current database.
pub fn run_script(container: &str, default_db: &str, script: &str) -> MongoshRun {
    if let Some(problem) = container_name_problem(container) {
        panic!("refusing to exec into {container:?}: {problem}");
    }
    assert!(
        default_db.starts_with("cbit_")
            && default_db
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_'),
        "the shell's default database must be a plain cbit_* name"
    );
    assert_container_serves_the_test_server(container);
    let uri = format!("mongodb://127.0.0.1:27017/{default_db}?directConnection=true");
    let mut child = Command::new("docker")
        .args(["exec", "-i", container, "mongosh", "--quiet", &uri])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("could not start docker exec");
    let mut stdin = child.stdin.take().expect("stdin");
    let bytes = script.as_bytes().to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&bytes);
    });
    let out = child.wait_with_output().expect("docker exec failed");
    let _ = writer.join();
    MongoshRun {
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}
