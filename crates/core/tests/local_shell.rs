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
use curlybrace_core::tools::{ToolChoice, ToolError, ToolKind, resolve};
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

/// The `mongosh` on PATH. A missing optional tool is a skip (printed), not a failure; one that is
/// there but cannot be used (not executable, ...) fails the test.
fn mongosh(test: &str) -> Option<std::path::PathBuf> {
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

    // One block: only the last value is echoed, so the ping is printed.
    let events = run_script(
        &mut h,
        id,
        exe,
        "printjson(db.runCommand({ping:1}))\n1+1\n\"hello-cbit\"",
    )
    .await;
    let out = text(&events, Stream::Stdout);
    assert!(out.contains("ok: 1"), "{out:?}");
    assert!(out.contains("hello-cbit"), "{out:?}");
    assert!(!out.contains('2'), "an earlier value was echoed: {out:?}");
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

/// Runs `script` on a fresh session and returns its events.
async fn run_on_local(test: &str, script: &str) -> Option<Vec<Event>> {
    if !local_enabled(test) {
        return None;
    }
    let exe = mongosh(test)?;
    let (mut h, id) = local_session().await;
    Some(run_script(&mut h, id, exe, script).await)
}

/// Line by line, `else {` on its own line after `}` was a statement of its own: a syntax error,
/// and then its body ran unconditionally.
#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn an_else_on_its_own_line_stays_with_its_if() {
    let script =
        "const c = true\nif (c) {\n  print(\"then-cbit\")\n}\nelse {\n  print(\"else-cbit\")\n}";
    let Some(events) = run_on_local("an_else_on_its_own_line_stays_with_its_if", script).await
    else {
        return;
    };
    assert_eq!(outcome(&events), Outcome::Exited(0), "{events:?}");
    let out = text(&events, Stream::Stdout);
    assert!(out.contains("then-cbit"), "{out:?}");
    assert!(!out.contains("else-cbit"), "the else body ran: {out:?}");
    assert_no_prompt_leak(&out);
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_chained_call_on_the_next_line_continues_the_expression() {
    let script = "[3, 1, 2]\n  .map(v => v * 2)\n  .sort()";
    let test = "a_chained_call_on_the_next_line_continues_the_expression";
    let Some(events) = run_on_local(test, script).await else {
        return;
    };
    assert_eq!(outcome(&events), Outcome::Exited(0), "{events:?}");
    let out = text(&events, Stream::Stdout);
    assert_eq!(out, "[ 2, 4, 6 ]\n");
}

/// `mongosh` evaluates a block as soon as it cannot parse it: what was read so far fails, and
/// the lines after the error must never reach it (they would run one by one).
#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_syntax_error_part_way_runs_none_of_the_script() {
    let script = "print(\"before-cbit\")\nvar = = 3\nprint(\"after-cbit\")";
    let test = "a_syntax_error_part_way_runs_none_of_the_script";
    let Some(events) = run_on_local(test, script).await else {
        return;
    };
    assert_eq!(outcome(&events), Outcome::ScriptErrors, "{events:?}");
    let err = text(&events, Stream::Stderr);
    assert!(err.starts_with("Uncaught:\nSyntaxError"), "{err:?}");
    assert!(
        err.contains("> 2 | var = = 3"),
        "editor line numbers: {err:?}"
    );
    let out = text(&events, Stream::Stdout);
    assert!(
        !out.contains("before-cbit"),
        "part of the script ran: {out:?}"
    );
    assert!(
        !out.contains("after-cbit"),
        "a line after the error ran: {out:?}"
    );
    assert_no_prompt_leak(&out);
}

/// A `}` with no `{` closes the block early: what came before it ran, the rest is never sent.
#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_stray_closing_brace_ends_the_script_as_incomplete() {
    let script = "print(\"one-cbit\") }\nprint(\"escaped-cbit\")";
    let test = "a_stray_closing_brace_ends_the_script_as_incomplete";
    let Some(events) = run_on_local(test, script).await else {
        return;
    };
    assert_eq!(outcome(&events), Outcome::Incomplete, "{events:?}");
    let out = text(&events, Stream::Stdout);
    assert!(out.contains("one-cbit"), "{out:?}");
    assert!(!out.contains("escaped-cbit"), "the rest ran: {out:?}");
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn an_unclosed_template_literal_is_incomplete() {
    let script = "print(\"never-cbit\")\nconst t = `open";
    let test = "an_unclosed_template_literal_is_incomplete";
    let Some(events) = run_on_local(test, script).await else {
        return;
    };
    assert_eq!(outcome(&events), Outcome::Incomplete, "{events:?}");
    let out = text(&events, Stream::Stdout);
    assert!(!out.contains("never-cbit"), "{out:?}");
    assert_no_prompt_leak(&out);
}

/// Shell helpers are REPL commands, not JavaScript: inside the block `use` is a syntax error,
/// and nothing runs. `db.getSiblingDB` is the way.
#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn shell_helpers_are_syntax_errors_and_their_javascript_forms_work() {
    let test = "shell_helpers_are_syntax_errors_and_their_javascript_forms_work";
    let Some(events) = run_on_local(test, "use cbit_other\nprint(\"ran-cbit\")").await else {
        return;
    };
    assert_eq!(outcome(&events), Outcome::ScriptErrors, "{events:?}");
    assert!(!text(&events, Stream::Stdout).contains("ran-cbit"));

    let script = "const other = db.getSiblingDB(\"cbit_other\")\nother.getName()";
    let Some(events) = run_on_local(test, script).await else {
        return;
    };
    assert_eq!(outcome(&events), Outcome::Exited(0), "{events:?}");
    assert_eq!(text(&events, Stream::Stdout), "cbit_other\n");
}

/// The preamble locks `prompt` and `__cbEmit`: a script that tries to redefine them is ignored
/// or gets an `Uncaught` error, the fixed prompt keeps coming back (no leaked user prompt), and
/// the run ends (a lost prompt would leave the runner waiting for the end of the block).
#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn prompt_and_emitter_redefinitions_are_locked_out_and_the_run_ends() {
    let test = "prompt_and_emitter_redefinitions_are_locked_out_and_the_run_ends";
    let ignored = [
        "prompt = () => \"x> \"",
        "globalThis.prompt = () => \"x> \"",
        "globalThis[\"pro\" + \"mpt\"] = () => \"x> \"",
        "var prompt = () => \"x> \"",
        "__cbEmit = null",
        "db.prompt.find({prompt: 1, k: \"prompt\"})",
    ];
    for attempt in ignored {
        let script = format!("{attempt}\nprint(\"after-cbit\")");
        let Some(events) = run_on_local(test, &script).await else {
            return;
        };
        assert_eq!(
            outcome(&events),
            Outcome::Exited(0),
            "{attempt}: {events:?}"
        );
        let out = text(&events, Stream::Stdout);
        assert!(out.contains("after-cbit"), "{attempt}: {out:?}");
        assert!(!out.contains("x> "), "{attempt}: {out:?}");
        assert_no_prompt_leak(&out);
    }
    let script = "Object.defineProperty(globalThis, \"prompt\", {value: () => \"x> \"})\nprint(\"after-cbit\")";
    let Some(events) = run_on_local(test, script).await else {
        return;
    };
    assert_eq!(outcome(&events), Outcome::ScriptErrors, "{events:?}");
    let err = text(&events, Stream::Stderr);
    assert!(err.contains("Cannot redefine property: prompt"), "{err:?}");
    let out = text(&events, Stream::Stdout);
    assert!(
        !out.contains("after-cbit") && !out.contains("x> "),
        "{out:?}"
    );
    assert_no_prompt_leak(&out);
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
