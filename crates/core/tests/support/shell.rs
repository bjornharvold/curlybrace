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

/// No prompt, sentinel or continuation prompt (`| `) in `out`. Not for syntax errors: their code
/// frame has `| ` of its own.
pub fn assert_no_prompt_leak(out: &str) {
    assert!(!out.contains("CBP"), "prompt marker leaked: {out:?}");
    assert!(!out.contains('\u{1}'), "control byte leaked: {out:?}");
    assert!(!out.contains("__CB_"), "sentinel leaked: {out:?}");
    assert!(!out.contains("| "), "continuation prompt leaked: {out:?}");
}
