//! The structured shell result through the runtime: `Event::ShellResult` is sent once, only for a
//! run that exited 0, and always right before that run's `ShellFinished`. A real runtime and the
//! fake `mongosh`; every wait is on an event, never on time.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use super::shell_handlers::result_to_send;
use super::*;
use crate::secrets::{MemorySecretStore, SecretKind, SecretStore, secret_key};
use crate::shell::fake::{Behaviour, FakeMongosh};
use crate::shell::{DocKind, MASK, Outcome, ShellResult};
use crate::test_support::InMemoryStore;

const WAIT: Duration = Duration::from_secs(10);
const QUIET: Duration = Duration::from_millis(300);
const ONE_DOC: &str = r#"{"k":"cursor","t":"FindCursor","v":[{"name":"ada"}],"more":false}"#;

struct StoreConnector;

#[async_trait]
impl Connector for StoreConnector {
    async fn connect(
        &self,
        _cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        Ok(Arc::new(InMemoryStore::new()))
    }
}

struct Fixture {
    h: RuntimeHandle,
    id: SessionId,
    fake: FakeMongosh,
}

impl Fixture {
    async fn connected(cfg: ConnectionConfig, password: Option<&str>) -> Self {
        let secrets = Arc::new(MemorySecretStore::new());
        if let Some(password) = password {
            secrets
                .set(&secret_key(cfg.id, SecretKind::Password), password)
                .unwrap();
        }
        let repaint: Repaint = Arc::new(|| {});
        let mut h = spawn_runtime_with_connector(
            Some(secrets),
            None,
            Arc::new(StoreConnector),
            repaint,
            RuntimeConfig::default(),
        );
        let id = cfg.id;
        h.tx.send(Command::Connect(cfg)).unwrap();
        assert!(matches!(next(&mut h).await, Event::Connected { .. }));
        Self {
            h,
            id,
            fake: FakeMongosh::new(Behaviour::Repl),
        }
    }

    async fn plain() -> Self {
        let cfg = ConnectionConfig::new("t", "mongodb://localhost:27017/app");
        Self::connected(cfg, None).await
    }

    fn run(&self, run_id: u64, script: &str) {
        self.h
            .tx
            .send(Command::ShellRun {
                session: self.id,
                run_id,
                exe: self.fake.exe.clone(),
                script: script.into(),
            })
            .unwrap();
    }

    /// Events up to and including the first `ShellFinished` or `ShellRefused`.
    async fn until_done(&mut self) -> Vec<Event> {
        let mut seen = Vec::new();
        loop {
            let event = next(&mut self.h).await;
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

    /// Nothing arrives for a while.
    async fn assert_quiet(&mut self) {
        if let Ok(event) = tokio::time::timeout(QUIET, self.h.rx.recv()).await {
            panic!("unexpected event {event:?}");
        }
    }
}

async fn next(h: &mut RuntimeHandle) -> Event {
    tokio::time::timeout(WAIT, h.rx.recv())
        .await
        .expect("event in time")
        .expect("channel open")
}

fn results(events: &[Event]) -> Vec<&ShellResult> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::ShellResult { result, .. } => Some(result),
            _ => None,
        })
        .collect()
}

fn finished_outcome(events: &[Event]) -> Outcome {
    match events.last() {
        Some(Event::ShellFinished { outcome, .. }) => *outcome,
        other => panic!("expected ShellFinished last, got {other:?}"),
    }
}

#[tokio::test]
async fn a_run_that_returns_documents_sends_shell_result_before_finished() {
    let mut f = Fixture::plain().await;
    f.run(7, &format!("result {ONE_DOC}"));
    let events = f.until_done().await;
    let at = events
        .iter()
        .position(|e| matches!(e, Event::ShellResult { .. }))
        .expect("a ShellResult");
    assert_eq!(
        at,
        events.len() - 2,
        "right before ShellFinished: {events:?}"
    );
    assert_eq!(finished_outcome(&events), Outcome::Exited(0));
    match &events[at] {
        Event::ShellResult {
            session,
            run_id,
            result: ShellResult::Documents {
                kind, docs, more, ..
            },
        } => {
            assert_eq!((*session, *run_id), (f.id, 7));
            assert_eq!(*kind, DocKind::Cursor);
            assert!(!more);
            assert_eq!(docs.len(), 1);
            assert_eq!(docs[0].get_str("name").unwrap(), "ada");
        }
        other => panic!("expected documents, got {other:?}"),
    }
    assert_eq!(events[at].session(), Some(f.id));
}

#[tokio::test]
async fn a_run_with_no_epilogue_answer_sends_no_result() {
    let mut f = Fixture::plain().await;
    f.run(1, "just text");
    let events = f.until_done().await;
    assert!(results(&events).is_empty(), "{events:?}");
    assert_eq!(finished_outcome(&events), Outcome::Exited(0));
}

#[tokio::test]
async fn an_unparseable_payload_sends_no_result() {
    let mut f = Fixture::plain().await;
    f.run(1, "result {\"k\":\"nonsense\"}");
    let events = f.until_done().await;
    assert!(results(&events).is_empty(), "{events:?}");
    assert_eq!(finished_outcome(&events), Outcome::Exited(0));
}

#[tokio::test]
async fn a_cancelled_run_sends_no_result() {
    let mut f = Fixture::plain().await;
    f.run(1, &format!("result {ONE_DOC}\nstarted\nhang"));
    loop {
        if matches!(next(&mut f.h).await, Event::ShellOutput { .. }) {
            break;
        }
    }
    let session = f.id;
    f.h.tx
        .send(Command::ShellCancel { session, run_id: 1 })
        .unwrap();
    let events = f.until_done().await;
    assert!(results(&events).is_empty(), "{events:?}");
    assert_eq!(finished_outcome(&events), Outcome::Cancelled);
}

#[tokio::test]
async fn a_run_with_script_errors_sends_no_result() {
    let mut f = Fixture::plain().await;
    f.run(1, &format!("result {ONE_DOC}\nthrow boom"));
    let events = f.until_done().await;
    assert!(results(&events).is_empty(), "{events:?}");
    assert_eq!(finished_outcome(&events), Outcome::ScriptErrors);
}

#[tokio::test]
async fn a_nonzero_exit_sends_no_result() {
    let mut f = Fixture::plain().await;
    f.run(1, &format!("result {ONE_DOC}\nexit 3"));
    let events = f.until_done().await;
    assert!(results(&events).is_empty(), "{events:?}");
    assert_eq!(finished_outcome(&events), Outcome::Exited(3));
}

/// The fake cannot exit non-zero after answering the epilogue, so the rule is also pinned on the
/// function that applies it.
#[test]
fn only_a_clean_exit_delivers_a_parsed_result() {
    let parsed = || crate::shell::parse_result(ONE_DOC);
    assert!(parsed().is_some());
    assert_eq!(result_to_send(Outcome::Exited(0), parsed()), parsed());
    assert_eq!(result_to_send(Outcome::Exited(0), None), None);
    for outcome in [
        Outcome::Exited(1),
        Outcome::Exited(-1),
        Outcome::ScriptErrors,
        Outcome::Incomplete,
        Outcome::ConnectFailed,
        Outcome::Signalled,
        Outcome::Cancelled,
        Outcome::CapReached,
        Outcome::StartFailed,
    ] {
        assert_eq!(result_to_send(outcome, parsed()), None, "{outcome:?}");
    }
}

#[tokio::test]
async fn the_event_debug_does_not_print_documents() {
    let mut f = Fixture::plain().await;
    f.run(
        1,
        r#"result {"k":"cursor","t":"FindCursor","v":[{"field":"SECRET-VALUE"}],"more":false}"#,
    );
    let events = f.until_done().await;
    assert_eq!(results(&events).len(), 1);
    match results(&events)[0] {
        ShellResult::Documents { docs, .. } => {
            assert_eq!(docs[0].get_str("field").unwrap(), "SECRET-VALUE");
        }
        other => panic!("expected documents, got {other:?}"),
    }
    let debug = format!("{events:?}");
    assert!(!debug.contains("SECRET-VALUE"), "{debug}");
    assert!(debug.contains("ShellResult"), "{debug}");
}

#[tokio::test]
async fn a_result_never_carries_the_password() {
    const PASSWORD: &str = r#"pa\ss"w"#;
    let mut cfg = ConnectionConfig::new("t", "mongodb://localhost:27017/app");
    cfg.auth = crate::connection::AuthMechanism::ScramSha256;
    cfg.username = Some("alice".into());
    let mut f = Fixture::connected(cfg, Some(PASSWORD)).await;
    // The payload as `EJSON.stringify` writes the value: backslash and quote escaped.
    f.run(
        1,
        r#"result {"k":"array","n":1,"v":[{"pw":"pa\\ss\"w"}],"more":false}"#,
    );
    let events = f.until_done().await;
    let found = results(&events);
    assert_eq!(found.len(), 1, "{events:?}");
    match found[0] {
        ShellResult::Documents { docs, .. } => {
            let value = docs[0].get_str("pw").unwrap();
            assert!(value.contains(MASK), "{value}");
            assert!(!value.contains("pa\\ss"), "{value}");
            assert!(!value.contains("ss\"w"), "{value}");
        }
        other => panic!("expected documents, got {other:?}"),
    }
}

/// A password equal to a JSON token (`true`) is masked in the raw payload, which then no longer
/// parses: the run says its value is withheld (no content), not that it has none.
#[tokio::test]
async fn a_password_that_breaks_the_payload_withholds_the_result() {
    let mut cfg = ConnectionConfig::new("t", "mongodb://localhost:27017/app");
    cfg.auth = crate::connection::AuthMechanism::ScramSha256;
    cfg.username = Some("alice".into());
    let mut f = Fixture::connected(cfg, Some("true")).await;
    f.run(1, r#"result {"k":"object","t":null,"v":{"ok":true}}"#);
    let events = f.until_done().await;
    assert_eq!(finished_outcome(&events), Outcome::Exited(0));
    assert_eq!(results(&events), vec![&ShellResult::Withheld], "{events:?}");
}

#[tokio::test]
async fn a_refused_run_sends_no_result() {
    let mut cfg = ConnectionConfig::new("t", "mongodb://localhost:27017/app");
    cfg.read_only = true;
    let mut f = Fixture::connected(cfg, None).await;
    f.run(1, &format!("result {ONE_DOC}"));
    let events = f.until_done().await;
    assert!(results(&events).is_empty(), "{events:?}");
    assert!(matches!(events.last(), Some(Event::ShellRefused { .. })));
    f.assert_quiet().await;
}

#[tokio::test]
async fn exactly_one_result_per_run() {
    let mut f = Fixture::plain().await;
    f.run(1, &format!("result-twice {ONE_DOC}"));
    let events = f.until_done().await;
    assert_eq!(results(&events).len(), 1, "{events:?}");
    f.run(2, &format!("result {ONE_DOC}"));
    let events = f.until_done().await;
    assert_eq!(results(&events).len(), 1, "{events:?}");
    f.assert_quiet().await;
}
