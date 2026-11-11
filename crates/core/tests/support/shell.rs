//! Shared by the gated real-`mongosh` Shell suites (`local_shell`, `local_shell_result`): a
//! session on the local server, a script run collected into events, and event readers.
#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;

use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::runtime::{Command, Event, RuntimeHandle, SessionId, spawn_runtime};
use curlybrace_core::secrets::MemorySecretStore;
use curlybrace_core::shell::{Outcome, Stream};
use curlybrace_core::tools::{ToolChoice, ToolError, ToolKind, resolve};

use super::{local_enabled, next_event, no_repaint};

/// A shell run on this machine can take a while (mongosh starts slowly, a failing connect waits
/// for server selection).
pub const LIMIT: Duration = Duration::from_secs(60);
pub const RUN: u64 = 1;

/// The `mongosh` on PATH. A missing optional tool is a skip (printed), not a failure; one that is
/// there but cannot be used (not executable, ...) fails the test.
pub fn mongosh(test: &str) -> Option<std::path::PathBuf> {
    let found = resolve(
        ToolKind::Mongosh,
        &ToolChoice::Path,
        std::env::var_os("PATH").as_deref(),
    );
    match found {
        Ok(exe) => Some(exe),
        Err(ToolError::NotOnPath | ToolError::Missing) => {
            eprintln!("SKIPPED {test}: mongosh not found on PATH");
            None
        }
        Err(error) => panic!("{test}: the mongosh on PATH cannot be used: {error:?}"),
    }
}

pub async fn connected(mut h: RuntimeHandle, cfg: ConnectionConfig) -> (RuntimeHandle, SessionId) {
    let id = cfg.id;
    h.tx.send(Command::Connect(cfg)).unwrap();
    let event = next_event(&mut h.rx, LIMIT, "connect").await;
    assert!(matches!(event, Event::Connected { .. }), "{event:?}");
    (h, id)
}

/// Runs `script` and returns every event up to the first `ShellFinished` (or a refusal).
pub async fn run_script(
    h: &mut RuntimeHandle,
    id: SessionId,
    exe: std::path::PathBuf,
    script: &str,
) -> Vec<Event> {
    h.tx.send(Command::ShellRun {
        session: id,
        run_id: RUN,
        exe,
        script: script.into(),
    })
    .unwrap();
    let mut seen = Vec::new();
    loop {
        let event = next_event(&mut h.rx, LIMIT, "shell run").await;
        let last = matches!(
            event,
            Event::ShellFinished { .. } | Event::ShellRefused { .. }
        );
        seen.push(event);
        if last {
            return seen;
        }
    }
}

pub fn text(events: &[Event], wanted: Stream) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ShellOutput { stream, text, .. } if *stream == wanted => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

pub fn outcome(events: &[Event]) -> Outcome {
    match events.last() {
        Some(Event::ShellFinished { outcome, .. }) => *outcome,
        other => panic!("expected ShellFinished, got {other:?}"),
    }
}

/// A runtime on the local server without auth, connected.
pub async fn local_session() -> (RuntimeHandle, SessionId) {
    let cfg = ConnectionConfig::new("local", &curlybrace_testkit::local_uri());
    let rt = spawn_runtime(Arc::new(MemorySecretStore::new()), no_repaint());
    connected(rt, cfg).await
}

/// Runs `script` on a fresh session and returns its events.
pub async fn run_on_local(test: &str, script: &str) -> Option<Vec<Event>> {
    if !local_enabled(test) {
        return None;
    }
    let exe = mongosh(test)?;
    let (mut h, id) = local_session().await;
    Some(run_script(&mut h, id, exe, script).await)
}

/// What a run straight through `spawn_shell` (no runtime) produced.
#[derive(Debug)]
pub struct RawRun {
    pub outcome: Outcome,
    pub stdout: String,
    pub stderr: String,
    /// Every `on_result` payload, in order.
    pub results: Vec<String>,
}

/// Runs `script` on the local server through `spawn_shell` itself, with `extra_args` after the
/// usual `--nodb --quiet` (`--shell <file>` runs a file first, as `~/.mongoshrc.js` would be).
/// Bounded: a run that does not end within [`LIMIT`] is cancelled and the test fails.
pub fn run_raw(exe: &std::path::Path, script: &str, extra_args: &[&std::ffi::OsStr]) -> RawRun {
    use curlybrace_core::shell::{RunLimits, build_invocation, spawn_shell};
    use std::sync::Mutex;
    let cfg = ConnectionConfig::new("local", &curlybrace_testkit::local_uri());
    let mut inv = build_invocation(&cfg, None, None, script).expect("an invocation");
    inv.args
        .extend(extra_args.iter().map(|arg| arg.to_os_string()));
    let text = Arc::new(Mutex::new((String::new(), String::new())));
    let results = Arc::new(Mutex::new(Vec::new()));
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let (text_sink, result_sink) = (Arc::clone(&text), Arc::clone(&results));
    let handle = spawn_shell(
        exe,
        inv,
        Vec::new(),
        std::env::var_os("PATH").as_deref(),
        RunLimits::default(),
        move |stream, chunk| {
            let mut text = text_sink.lock().unwrap();
            match stream {
                Stream::Stdout => text.0.push_str(&chunk),
                Stream::Stderr => text.1.push_str(&chunk),
            }
        },
        // No secrets are passed, so masking never changes a payload here.
        move |payload, _masked| result_sink.lock().unwrap().push(payload),
        move |outcome| {
            let _ = done_tx.send(outcome);
        },
    );
    let outcome = match done_rx.recv_timeout(LIMIT) {
        Ok(outcome) => outcome,
        Err(_) => {
            handle.cancel();
            panic!(
                "the run did not end within {LIMIT:?}: {:?}",
                text.lock().unwrap()
            );
        }
    };
    let (stdout, stderr) = text.lock().unwrap().clone();
    let results = results.lock().unwrap().clone();
    RawRun {
        outcome,
        stdout,
        stderr,
        results,
    }
}

/// No prompt, sentinel or continuation prompt (`| `) in `out`. Not for syntax errors: their code
/// frame has `| ` of its own.
pub fn assert_no_prompt_leak(out: &str) {
    assert!(!out.contains("CBP"), "prompt marker leaked: {out:?}");
    assert!(!out.contains('\u{1}'), "control byte leaked: {out:?}");
    assert!(!out.contains("__CB_"), "sentinel leaked: {out:?}");
    assert!(!out.contains("| "), "continuation prompt leaked: {out:?}");
}
