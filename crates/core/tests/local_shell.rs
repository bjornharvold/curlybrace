//! Tier 1 (local server): the Shell tab's runtime commands with the REAL `mongosh` on PATH
//! against the local server. Nothing is written: the scripts only ping and print. Run with
//! `CURLYBRACE_IT=1 cargo test -p curlybrace-core --test local_shell -- --ignored --nocapture`
//! (point `CURLYBRACE_IT_MONGODB_URI` at a throwaway server).

mod support;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use curlybrace_core::connection::{AuthMechanism, ConnectionConfig};
use curlybrace_core::runtime::{
    Command, Connector, Event, RuntimeConfig, RuntimeHandle, SessionId, spawn_runtime,
    spawn_runtime_with_connector,
};
use curlybrace_core::secrets::{MemorySecretStore, SecretKind, SecretStore, secret_key};
use curlybrace_core::shell::{Outcome, Stream, percent_encode_component};
use curlybrace_core::store::DocumentStore;
use curlybrace_core::test_support::InMemoryStore;
use curlybrace_core::tools::{ToolChoice, ToolKind, resolve};
use support::{local_enabled, next_event, no_repaint};

/// A shell run on this machine can take a while (mongosh starts slowly, a failing connect waits
/// for server selection).
const LIMIT: Duration = Duration::from_secs(60);
const PASSWORD: &str = "p@ss:w/rd%#?cbit";
const RUN: u64 = 1;

/// Accepts every connection without touching a server, so the shell is the only thing that
/// talks to MongoDB (and can fail to authenticate).
struct AcceptAll;

#[async_trait]
impl Connector for AcceptAll {
    async fn connect(
        &self,
        _cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<std::net::SocketAddr>,
    ) -> curlybrace_core::error::Result<Arc<dyn DocumentStore>> {
        Ok(Arc::new(InMemoryStore::new()))
    }
}

/// The `mongosh` on PATH. A missing optional tool is a skip (printed), not a failure.
fn mongosh(test: &str) -> Option<std::path::PathBuf> {
    let found = resolve(
        ToolKind::Mongosh,
        &ToolChoice::Path,
        std::env::var_os("PATH").as_deref(),
    );
    if found.is_err() {
        eprintln!("SKIPPED {test}: mongosh not found on PATH");
    }
    found.ok()
}

async fn connected(mut h: RuntimeHandle, cfg: ConnectionConfig) -> (RuntimeHandle, SessionId) {
    let id = cfg.id;
    h.tx.send(Command::Connect(cfg)).unwrap();
    let event = next_event(&mut h.rx, LIMIT, "connect").await;
    assert!(matches!(event, Event::Connected { .. }), "{event:?}");
    (h, id)
}

/// Runs `script` and returns every event up to the first `ShellFinished` (or a refusal).
async fn run_script(
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

fn text(events: &[Event], wanted: Stream) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ShellOutput { stream, text, .. } if *stream == wanted => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn outcome(events: &[Event]) -> Outcome {
    match events.last() {
        Some(Event::ShellFinished { outcome, .. }) => *outcome,
        other => panic!("expected ShellFinished, got {other:?}"),
    }
}

/// No prompt, sentinel or continuation prompt (`| `) in `out`. Not for syntax errors: their code
/// frame has `| ` of its own.
fn assert_no_prompt_leak(out: &str) {
    assert!(!out.contains("CBP"), "prompt marker leaked: {out:?}");
    assert!(!out.contains('\u{1}'), "control byte leaked: {out:?}");
    assert!(!out.contains("__CB_"), "sentinel leaked: {out:?}");
    assert!(!out.contains("| "), "continuation prompt leaked: {out:?}");
}

/// A runtime on the local server without auth, connected.
async fn local_session() -> (RuntimeHandle, SessionId) {
    let cfg = ConnectionConfig::new("local", &curlybrace_testkit::local_uri());
    let rt = spawn_runtime(Arc::new(MemorySecretStore::new()), no_repaint());
    connected(rt, cfg).await
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn an_error_inside_a_multi_line_statement_is_a_script_error() {
    let test = "an_error_inside_a_multi_line_statement_is_a_script_error";
    if !local_enabled(test) {
        return;
    }
    let Some(exe) = mongosh(test) else {
        return;
    };
    let (mut h, id) = local_session().await;

    let script = "var x = 1\nif (x) {\n  throw new Error(\"multi-cbit\")\n}";
    let events = run_script(&mut h, id, exe, script).await;
    assert_eq!(outcome(&events), Outcome::ScriptErrors, "{events:?}");
    let err = text(&events, Stream::Stderr);
    assert!(err.starts_with("Uncaught Error: multi-cbit"), "{err:?}");
    assert_no_prompt_leak(&err);
    assert_no_prompt_leak(&text(&events, Stream::Stdout));
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn ping_and_bare_expressions_stream_clean_output() {
    if !local_enabled("ping_and_bare_expressions_stream_clean_output") {
        return;
    }
    let Some(exe) = mongosh("ping_and_bare_expressions_stream_clean_output") else {
        return;
    };
    let cfg = ConnectionConfig::new("local", &curlybrace_testkit::local_uri());
    let rt = spawn_runtime(Arc::new(MemorySecretStore::new()), no_repaint());
    let (mut h, id) = connected(rt, cfg).await;

    let events = run_script(
        &mut h,
        id,
        exe,
        "db.runCommand({ping:1})\n1+1\n\"hello-cbit\"",
    )
    .await;
    let out = text(&events, Stream::Stdout);
    assert!(out.contains("ok: 1"), "{out:?}");
    assert!(out.contains("hello-cbit"), "{out:?}");
    assert_no_prompt_leak(&out);
    assert!(
        !text(&events, Stream::Stderr).contains("Uncaught"),
        "{events:?}"
    );
    assert_eq!(outcome(&events), Outcome::Exited(0), "{events:?}");
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_thrown_error_is_a_script_error_on_stderr() {
    if !local_enabled("a_thrown_error_is_a_script_error_on_stderr") {
        return;
    }
    let Some(exe) = mongosh("a_thrown_error_is_a_script_error_on_stderr") else {
        return;
    };
    let cfg = ConnectionConfig::new("local", &curlybrace_testkit::local_uri());
    let rt = spawn_runtime(Arc::new(MemorySecretStore::new()), no_repaint());
    let (mut h, id) = connected(rt, cfg).await;

    let events = run_script(&mut h, id, exe, "throw new Error(\"boom-cbit\")").await;
    assert_eq!(outcome(&events), Outcome::ScriptErrors, "{events:?}");
    let err = text(&events, Stream::Stderr);
    assert!(err.contains("boom-cbit"), "{err:?}");
    assert_no_prompt_leak(&err);
    assert_no_prompt_leak(&text(&events, Stream::Stdout));
}

/// Unblocked, `passwordPrompt()` on piped stdin prints "Enter password", reads the rest of the
/// script as the password and exits 0, so nothing after it runs and the run looks successful.
#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn password_prompt_is_a_script_error() {
    let test = "password_prompt_is_a_script_error";
    if !local_enabled(test) {
        return;
    }
    let Some(exe) = mongosh(test) else {
        return;
    };
    let (mut h, id) = local_session().await;

    let script = "print(\"before-cbit\")\nconst pw = passwordPrompt()\nprint(\"after-cbit\")";
    let events = run_script(&mut h, id, exe, script).await;
    assert_eq!(outcome(&events), Outcome::ScriptErrors, "{events:?}");
    let err = text(&events, Stream::Stderr);
    assert!(
        err.contains("passwordPrompt() is not available in the shell tab"),
        "{err:?}"
    );
    let out = text(&events, Stream::Stdout);
    assert!(!out.contains("Enter password"), "{out:?}");
    assert_no_prompt_leak(&out);
}

/// The wrong password for a user the server does not know (the server needs no `--auth`: an
/// unknown user fails to authenticate either way). The runtime connects through [`AcceptAll`], so
/// only `mongosh` meets the server. Lighter than a second `--auth` server.
#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_failed_login_is_connect_failed_and_never_leaks_the_password() {
    if !local_enabled("a_failed_login_is_connect_failed_and_never_leaks_the_password") {
        return;
    }
    let Some(exe) = mongosh("a_failed_login_is_connect_failed_and_never_leaks_the_password") else {
        return;
    };
    let mut cfg = ConnectionConfig::new("local", &curlybrace_testkit::local_uri());
    cfg.auth = AuthMechanism::ScramSha256;
    cfg.username = Some("cbit_nobody".into());
    cfg.auth_source = Some("admin".into());
    let secrets = Arc::new(MemorySecretStore::new());
    secrets
        .set(&secret_key(cfg.id, SecretKind::Password), PASSWORD)
        .unwrap();
    let rt = spawn_runtime_with_connector(
        Some(secrets),
        None,
        Arc::new(AcceptAll),
        no_repaint(),
        RuntimeConfig::default(),
    );
    let (mut h, id) = connected(rt, cfg).await;

    let events = run_script(&mut h, id, exe, "print(\"script-ran-cbit\")").await;
    assert_eq!(outcome(&events), Outcome::ConnectFailed, "{events:?}");
    let encoded = percent_encode_component(PASSWORD);
    let all = format!("{events:?}");
    assert!(!all.contains(PASSWORD), "password in an event: {all}");
    assert!(
        !all.contains(&encoded),
        "encoded password in an event: {all}"
    );
    let err = text(&events, Stream::Stderr).to_lowercase();
    assert!(err.contains("authentication failed"), "{err:?}");
    assert!(!all.contains("script-ran-cbit"), "the script ran: {all}");
}
