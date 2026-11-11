//! Tier 1 (local server): the structured result of a Shell run (Table / Tree data) with the REAL
//! `mongosh` on PATH. Scripts write only to throwaway `cbit_*` databases, dropped afterwards. Run
//! with `CURLYBRACE_IT=1 cargo test -p curlybrace-core --test local_shell_result -- --ignored
//! --nocapture` (point `CURLYBRACE_IT_MONGODB_URI` at a throwaway server).
//!
//! The first test is also the canary for a `mongosh` upgrade: the lift reads `mongosh` internals,
//! and an upgrade that renames them turns the feature into "no table".

mod support;

use std::sync::Arc;
use std::time::Duration;

use bson::Bson;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::runtime::{Command, Event, spawn_runtime};
use curlybrace_core::secrets::{MemorySecretStore, SecretKind, SecretStore, secret_key};
use curlybrace_core::shell::{DocKind, MASK, Outcome, ShellResult, Stream, secret_forms};
use support::shell::*;
use support::{local_enabled, next_event, no_repaint};

/// A password with a quote, a backslash and a non-ASCII character.
const TRICKY: &str = "p\"a\\ss:w\u{f6}rd%#cbit";

fn result_of(events: &[Event]) -> Option<&ShellResult> {
    events.iter().find_map(|e| match e {
        Event::ShellResult { result, .. } => Some(result),
        _ => None,
    })
}

/// Everything the shell printed, both streams.
fn all_text(events: &[Event]) -> String {
    text(events, Stream::Stdout) + &text(events, Stream::Stderr)
}

/// Runs `script(db_name)` against a fresh throwaway database, then drops it.
async fn in_scratch_db(
    test: &str,
    password: Option<&str>,
    script: impl Fn(&str) -> String,
) -> Option<Vec<Event>> {
    if !local_enabled(test) {
        return None;
    }
    let exe = mongosh(test)?;
    let cfg = ConnectionConfig::new("local", &curlybrace_testkit::local_uri());
    let secrets = Arc::new(MemorySecretStore::new());
    if let Some(pw) = password {
        secrets
            .set(&secret_key(cfg.id, SecretKind::Password), pw)
            .unwrap();
    }
    let rt = spawn_runtime(secrets, no_repaint());
    let (mut h, id) = connected(rt, cfg).await;
    let name = curlybrace_testkit::unique_db_name("shell");
    let events = run_script(&mut h, id, exe.clone(), &script(&name)).await;
    let drop = format!("db.getSiblingDB(\"{name}\").dropDatabase()");
    let cleanup = run_script(&mut h, id, exe, &drop).await;
    assert_eq!(outcome(&cleanup), Outcome::Exited(0), "{cleanup:?}");
    Some(events)
}

fn seed_30(name: &str) -> String {
    format!(
        "const t = db.getSiblingDB(\"{name}\").items\n\
         t.insertMany(Array.from({{length: 30}}, (x, i) => ({{_id: i, n: i}})))\n"
    )
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_cursor_result_has_the_cursor_shape() {
    let test = "a_cursor_result_has_the_cursor_shape";
    let Some(events) = in_scratch_db(test, None, |n| seed_30(n) + "t.find()").await else {
        return;
    };
    assert_eq!(outcome(&events), Outcome::Exited(0), "{events:?}");
    let canary = "mongosh may have renamed the internals the result lift reads \
                  (_currentIterationResult / cursor shape): update the emitter";
    let Some(ShellResult::Documents {
        kind,
        docs,
        more,
        total,
    }) = result_of(&events)
    else {
        panic!("{canary}: no cursor result in {events:?}");
    };
    assert_eq!(*kind, DocKind::Cursor, "{canary}");
    assert!(
        *more,
        "{canary}: 30 documents, a batch of 20, more expected"
    );
    assert_eq!(
        docs.len(),
        20,
        "{canary}: the echoed batch is 20 by default"
    );
    assert_eq!(*total, None, "{canary}");
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn find_to_array_is_documents() {
    let test = "find_to_array_is_documents";
    let Some(events) = in_scratch_db(test, None, |n| seed_30(n) + "t.find().toArray()").await
    else {
        return;
    };
    let Some(ShellResult::Documents {
        kind, docs, total, ..
    }) = result_of(&events)
    else {
        panic!("no array result in {events:?}");
    };
    assert_eq!(*kind, DocKind::Array);
    assert_eq!(docs.len(), 30);
    assert_eq!(*total, Some(30));
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn findone_is_an_object() {
    let test = "findone_is_an_object";
    let Some(events) = in_scratch_db(test, None, |n| seed_30(n) + "t.findOne({_id: 7})").await
    else {
        return;
    };
    let Some(ShellResult::Object { doc, .. }) = result_of(&events) else {
        panic!("no object result in {events:?}");
    };
    assert_eq!(doc.get_i32("n").ok(), Some(7), "{doc:?}");
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_scalar_is_a_scalar() {
    let test = "a_scalar_is_a_scalar";
    let Some(events) = in_scratch_db(test, None, |n| seed_30(n) + "t.countDocuments({})").await
    else {
        return;
    };
    let Some(ShellResult::Scalar { value, undefined }) = result_of(&events) else {
        panic!("no scalar result in {events:?}");
    };
    assert!(!*undefined);
    let count = match value {
        Bson::Int32(n) => i64::from(*n),
        Bson::Int64(n) => *n,
        other => panic!("not a number: {other:?}"),
    };
    assert_eq!(count, 30);
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn an_error_sends_no_result() {
    let test = "an_error_sends_no_result";
    let Some(events) = in_scratch_db(test, None, |n| {
        seed_30(n) + "t.find()\nthrow new Error(\"boom-cbit\")"
    })
    .await
    else {
        return;
    };
    assert_eq!(outcome(&events), Outcome::ScriptErrors, "{events:?}");
    assert!(result_of(&events).is_none(), "{events:?}");
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_password_with_quote_and_backslash_never_appears_in_text_or_result() {
    let test = "a_password_with_quote_and_backslash_never_appears_in_text_or_result";
    let literal = serde_json::to_string(TRICKY).unwrap();
    let Some(events) = in_scratch_db(test, Some(TRICKY), |n| {
        format!(
            "const t = db.getSiblingDB(\"{n}\").secrets\n\
             t.insertOne({{_id: 1, pw: {literal}}})\n\
             print({literal})\n\
             printjson({{pw: {literal}}})\n\
             t.find()"
        )
    })
    .await
    else {
        return;
    };
    assert_eq!(outcome(&events), Outcome::Exited(0), "{events:?}");
    let shown = all_text(&events);
    let Some(ShellResult::Documents { docs, .. }) = result_of(&events) else {
        panic!("no result in {events:?}");
    };
    let parsed = format!("{docs:?}");
    assert!(shown.contains(MASK), "the mask is expected: {shown:?}");
    assert_eq!(docs[0].get_str("pw").ok(), Some(MASK), "{parsed}");
    let extra = [
        "a\\ss".to_owned(),
        "a\\\\ss".to_owned(),
        "w\u{f6}rd".to_owned(),
    ];
    for form in secret_forms(TRICKY).into_iter().chain(extra) {
        assert!(!shown.contains(&form), "{form:?} in the text: {shown:?}");
        assert!(!parsed.contains(&form), "{form:?} in the result: {parsed}");
    }
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_spoofed_marker_is_plain_text() {
    let test = "a_spoofed_marker_is_plain_text";
    let script = "print(\"\\u0001CBJ-deadbeefdeadbeef {\\\"k\\\":\\\"fake\\\"}\")\n\
                  print(\"CBJ-0123456789abcdef {\\\"k\\\":\\\"guess\\\"}\")";
    let Some(events) = run_on_local(test, script).await else {
        return;
    };
    assert_eq!(outcome(&events), Outcome::Exited(0), "{events:?}");
    let out = text(&events, Stream::Stdout);
    assert!(out.contains("\"k\":\"fake\""), "{out:?}");
    assert!(out.contains("\"k\":\"guess\""), "{out:?}");
    // The only result is the real one: `print` returned undefined. Neither fake line became it.
    let results = events
        .iter()
        .filter(|e| matches!(e, Event::ShellResult { .. }))
        .count();
    assert_eq!(results, 1, "{events:?}");
    assert!(
        matches!(
            result_of(&events),
            Some(ShellResult::Scalar {
                undefined: true,
                ..
            })
        ),
        "{events:?}"
    );
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn assigning_underscore_sends_no_result() {
    let test = "assigning_underscore_sends_no_result";
    let Some(events) = run_on_local(test, "var _ = 1").await else {
        return;
    };
    assert_eq!(outcome(&events), Outcome::Exited(0), "{events:?}");
    let out = text(&events, Stream::Stdout);
    assert!(out.contains("assignment to _"), "{out:?}");
    assert!(result_of(&events).is_none(), "{events:?}");
}

#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn ordinary_scripts_mentioning_prompt_still_get_a_result() {
    let test = "ordinary_scripts_mentioning_prompt_still_get_a_result";
    let Some(events) = in_scratch_db(test, None, |n| {
        format!(
            "const p = db.getSiblingDB(\"{n}\").prompt\n\
             p.insertOne({{_id: 1, prompt: \"hello\"}})\n\
             db.getSiblingDB(\"{n}\").prompt.find()"
        )
    })
    .await
    else {
        return;
    };
    assert_eq!(outcome(&events), Outcome::Exited(0), "{events:?}");
    let Some(ShellResult::Documents { docs, .. }) = result_of(&events) else {
        panic!("no result in {events:?}");
    };
    assert_eq!(docs.len(), 1);
}

/// A startup file (as `~/.mongoshrc.js` is, also on piped stdin) that declares `prompt` makes it
/// a non-configurable global: the preamble's lock must not throw on it, and must still connect.
/// The file is run with `--shell <file>`, which `mongosh` loads in the same context just before
/// its rc files (the real HOME is not overridden).
#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_startup_file_that_declares_prompt_does_not_stop_the_connect() {
    let test = "a_startup_file_that_declares_prompt_does_not_stop_the_connect";
    if !local_enabled(test) {
        return;
    }
    let Some(exe) = mongosh(test) else {
        return;
    };
    for rc in [
        "var prompt = \"theirs> \";\n",
        "function prompt() { return \"theirs> \" }\n",
        "var prompt = 1;\nvar __cbEmit = 2;\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("rc.js");
        std::fs::write(&file, rc).unwrap();
        let run = run_raw(
            &exe,
            "print(\"ran-cbit\")\n1 + 1",
            &[std::ffi::OsStr::new("--shell"), file.as_os_str()],
        );
        assert_eq!(run.outcome, Outcome::Exited(0), "{rc}: {run:?}");
        assert_eq!(run.stdout, "ran-cbit\n2\n", "{rc}: {run:?}");
        assert!(!run.stderr.contains("redefine"), "{rc}: {run:?}");
        assert_eq!(run.results.len(), 1, "{rc}: {run:?}");
        let result = curlybrace_core::shell::parse_result(&run.results[0]).unwrap();
        assert!(
            matches!(
                result,
                ShellResult::Scalar {
                    value: Bson::Int32(2),
                    ..
                }
            ),
            "{rc}: {result:?}"
        );
    }
}

/// A script that silences stdout leaves the runner waiting for the end of the block: only Cancel
/// ends it. Bounded: a short wait for "no end by itself", then a cancel that must end the run.
#[tokio::test]
#[ignore = "needs a local MongoDB; see scripts/integration-tests.sh"]
async fn a_script_that_silences_stdout_is_ended_only_by_cancel() {
    let test = "a_script_that_silences_stdout_is_ended_only_by_cancel";
    if !local_enabled(test) {
        return;
    }
    let Some(exe) = mongosh(test) else {
        return;
    };
    let (mut h, id) = local_session().await;
    h.tx.send(Command::ShellRun {
        session: id,
        run_id: RUN,
        exe,
        script: "process.stdout.write = () => true\ndb.runCommand({ping: 1})".into(),
    })
    .unwrap();
    // Drain for a while: the run must not end by itself.
    let window = tokio::time::sleep(Duration::from_secs(8));
    tokio::pin!(window);
    let early = loop {
        tokio::select! {
            () = &mut window => break None,
            event = h.rx.recv() => match event {
                Some(Event::ShellFinished { outcome, .. }) => {
                    break Some(format!("ended by itself with {outcome:?}"))
                }
                Some(_) => {}
                None => break Some("runtime closed".to_owned()),
            },
        }
    };
    // Cancel before any failure, so no `mongosh` is left running whatever happened.
    let cancel = Command::ShellCancel {
        session: id,
        run_id: RUN,
    };
    let sent = h.tx.send(cancel);
    if let Some(why) = early {
        panic!("{why}");
    }
    sent.unwrap();
    loop {
        match next_event(&mut h.rx, LIMIT, "the cancelled run").await {
            Event::ShellFinished { outcome, .. } => {
                assert_eq!(outcome, Outcome::Cancelled);
                break;
            }
            Event::ShellResult { .. } => panic!("a cancelled run sent a result"),
            _ => {}
        }
    }
}
