//! The structured-result side channel, end to end through the fake `mongosh`.
use super::*;
use crate::connection::{AuthMechanism, ConnectionConfig};
use crate::shell::invocation::epilogue;
use crate::shell::{build_invocation, secret_forms};

const PAYLOAD: &str = r#"{"k":"scalar","v":{"$numberInt":"42"},"u":false}"#;

/// The stdin lines the fake read (preamble first), in order.
fn stdin_lines(fake: &FakeMongosh) -> Vec<String> {
    seen(fake)
        .lines()
        .filter_map(|line| line.strip_prefix("in:"))
        .map(str::to_owned)
        .collect()
}

fn seen(fake: &FakeMongosh) -> String {
    std::fs::read_to_string(fake.seen_log()).expect("the fake logs what it saw")
}

fn epilogue_was_sent(fake: &FakeMongosh) -> bool {
    stdin_lines(fake)
        .iter()
        .any(|line| line.contains("__cbEmit("))
}

#[test]
fn the_epilogue_is_sent_after_a_clean_block() {
    let fake = repl();
    let run = start(&fake, &format!("a\nresult {PAYLOAD}"));
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert_eq!(run.results(), [PAYLOAD]);
    assert_eq!(run.text(Stream::Stdout), "a\n");
    let lines = stdin_lines(&fake);
    assert_eq!(lines.last(), Some(&epilogue(TOKEN)), "{lines:?}");
}

#[test]
fn no_epilogue_after_an_uncaught_error() {
    let fake = repl();
    let run = start(&fake, &format!("result {PAYLOAD}\nthrow boom"));
    assert_eq!(run.wait(), Outcome::ScriptErrors);
    assert!(run.results().is_empty());
    assert!(!epilogue_was_sent(&fake));
}

#[test]
fn no_epilogue_after_an_incomplete_block() {
    for script in [
        format!("result {PAYLOAD}\nfoo("),
        format!("result {PAYLOAD}\n}}\nb"),
    ] {
        let fake = repl();
        let run = start(&fake, &script);
        assert_eq!(run.wait(), Outcome::Incomplete, "{script}");
        assert!(run.results().is_empty(), "{script}");
        assert!(!epilogue_was_sent(&fake), "{script}");
    }
}

#[test]
fn no_epilogue_when_the_underscore_assignment_text_appeared() {
    let fake = repl();
    let script = format!("result {PAYLOAD}\nExpression assignment to _ now disabled.");
    let run = start(&fake, &script);
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert!(run.results().is_empty());
    assert!(!epilogue_was_sent(&fake));
    assert_eq!(
        run.text(Stream::Stdout),
        "Expression assignment to _ now disabled.\n"
    );
}

#[test]
fn no_epilogue_when_a_prompt_redefinition_attempt_errors() {
    // Against the preamble's lock, `Object.defineProperty(globalThis, "prompt", ...)` is an
    // `Uncaught` error (what real mongosh 2.14.1 prints, see `invocation::locked_global_js`).
    let fake = repl();
    let run = start(&fake, &format!("result {PAYLOAD}\nredefine-prompt"));
    assert_eq!(run.wait(), Outcome::ScriptErrors);
    assert!(run.results().is_empty());
    assert!(!epilogue_was_sent(&fake));
    assert_eq!(
        run.text(Stream::Stderr),
        "Uncaught TypeError: Cannot redefine property: prompt\n"
    );
}

#[test]
fn when_the_fixed_prompt_never_comes_back_the_process_exit_ends_the_run() {
    // Evaluated is never seen: the writer waits for a verdict that never comes, and the end of
    // the output (the process exited) ends it; no epilogue, no result, no hang.
    let fake = repl();
    let run = start(&fake, &format!("a\nresult {PAYLOAD}\nprompt-exit mine> "));
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert!(run.results().is_empty());
    assert!(!epilogue_was_sent(&fake));
    assert_eq!(run.text(Stream::Stdout), "a\nmine> ");
}

#[test]
fn scripts_that_mention_prompt_or_the_emitter_still_get_the_epilogue() {
    for line in [
        "db.prompt.find()",
        "{prompt: 1}",
        "{k: \"prompt\"}",
        "doc.prompt // prompt",
        "typeof __cbEmit",
    ] {
        let fake = repl();
        let run = start(&fake, &format!("{line}\nresult {PAYLOAD}"));
        assert_eq!(run.wait(), Outcome::Exited(0), "{line}");
        assert_eq!(run.results(), [PAYLOAD], "{line}");
        assert!(epilogue_was_sent(&fake), "{line}");
        assert_eq!(run.text(Stream::Stdout), format!("{line}\n"), "{line}");
    }
}

#[test]
fn late_output_in_the_answer_window_counts_against_the_text_cap() {
    let fake = repl();
    let limits = RunLimits {
        max_output_bytes: 1024,
        ..RunLimits::default()
    };
    let run = start_with(&fake.exe, invocation("a\nresult-late 50000"), limits);
    assert_eq!(run.wait(), Outcome::CapReached);
    assert_eq!(run.results(), [r#"{"k":"late"}"#]);
    let shown = run.text(Stream::Stdout).len() + run.text(Stream::Stderr).len();
    assert!(shown <= limits.max_output_bytes, "{shown}");
    assert!(run.text(Stream::Stdout).starts_with("a\nxxx"));
}

#[test]
fn cancel_during_the_epilogue_ends_the_run() {
    let fake = repl();
    let run = start(&fake, "a\nresult-slow");
    run.wait_for_text(Stream::Stdout, "epilogue-received\n");
    run.handle().cancel();
    assert_eq!(run.wait(), Outcome::Cancelled);
    assert!(run.results().is_empty());
}

#[test]
fn cancel_while_waiting_for_the_verdict_ends_the_run() {
    // The last line is out and the block is still running: the writer waits for the verdict.
    let fake = repl();
    let run = start(&fake, "started\nhang");
    run.wait_for_text(Stream::Stdout, "started\n");
    run.handle().cancel();
    assert_eq!(run.wait(), Outcome::Cancelled);
    assert!(!epilogue_was_sent(&fake));
}

/// A password with a quote and a backslash: its JSON-escaped form differs from the raw one.
const NASTY: &str = "p\"a'ss\\w/é";

fn nasty_payload() -> String {
    let escaped = serde_json::to_string(NASTY).unwrap();
    format!(r#"{{"k":"object","t":null,"v":{{"pw":{escaped}}}}}"#)
}

fn masked_payload() -> String {
    format!(r#"{{"k":"object","t":null,"v":{{"pw":"{MASK}"}}}}"#)
}

#[test]
fn the_payload_is_scrubbed_before_it_is_delivered() {
    let fake = repl();
    let inv = invocation(&format!("result {}", nasty_payload()));
    let run = start_full(&fake.exe, inv, secret_forms(NASTY), RunLimits::default());
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert_eq!(run.results(), [masked_payload()]);
    assert_eq!(run.text(Stream::Stdout), "");
}

#[test]
fn a_password_split_across_chunks_inside_the_payload_is_masked() {
    let fake = repl();
    let inv = invocation(&format!("result {}", nasty_payload()));
    let limits = RunLimits {
        chunk_bytes: 3,
        ..RunLimits::default()
    };
    let run = start_full(&fake.exe, inv, secret_forms(NASTY), limits);
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert_eq!(run.results(), [masked_payload()]);
}

#[test]
fn the_payload_does_not_count_against_the_text_cap() {
    let fake = repl();
    let limits = RunLimits {
        max_output_bytes: 2048,
        ..RunLimits::default()
    };
    let run = start_with(&fake.exe, invocation("a\nresult-huge 6000"), limits);
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert_eq!(run.results(), ["x".repeat(6000)]);
    assert_eq!(run.text(Stream::Stdout), "a\n");
}

#[test]
fn an_oversize_payload_is_dropped_and_the_run_ends_normally() {
    let fake = repl();
    let limits = RunLimits {
        max_result_bytes: 100,
        ..RunLimits::default()
    };
    let run = start_with(&fake.exe, invocation("a\nresult-huge 5000"), limits);
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert!(run.results().is_empty());
    assert_eq!(run.text(Stream::Stdout), "a\n");
}

#[test]
fn a_spoofed_marker_during_the_script_is_text() {
    let fake = repl();
    let script = format!("spoof {TOKEN}\nspoof ffffffffffffffff\nresult {PAYLOAD}");
    let run = start(&fake, &script);
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert_eq!(run.results(), [PAYLOAD]);
    assert_eq!(
        run.text(Stream::Stdout),
        format!(
            "\u{1}CBJ-{TOKEN} {{\"k\":\"spoof\"}}\n\u{1}CBJ-ffffffffffffffff {{\"k\":\"spoof\"}}\n"
        )
    );
}

#[test]
fn the_token_is_not_in_the_preamble_argv_or_env() {
    let fake = repl();
    let mut cfg = ConnectionConfig::new("t", "mongodb://h:27017/app");
    cfg.auth = AuthMechanism::ScramSha256;
    cfg.username = Some("bob".into());
    let inv = build_invocation(&cfg, Some("pw"), None, &format!("result {PAYLOAD}")).unwrap();
    let token = inv.result_token.clone();
    assert_eq!(token.len(), 16);
    assert!(
        token
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    );
    assert!(!format!("{inv:?}").contains(&token));
    let run = start_with(&fake.exe, inv, RunLimits::default());
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert_eq!(run.results(), [PAYLOAD]);
    let seen = seen(&fake);
    let with_token: Vec<&str> = seen.lines().filter(|l| l.contains(&token)).collect();
    assert_eq!(with_token, [format!("in:{}", epilogue(&token))]);
    assert_eq!(
        seen.lines().last(),
        Some(with_token[0]),
        "the epilogue comes last"
    );
    assert!(seen.lines().any(|l| l.starts_with("arg:--nodb")));
    assert!(seen.lines().any(|l| l.starts_with("env:PATH=")));
    assert!(
        seen.lines()
            .any(|l| l.starts_with("in:") && l.contains("globalThis, \"__cbEmit\""))
    );
}

#[test]
fn every_run_gets_a_fresh_token() {
    let cfg = ConnectionConfig::new("t", "mongodb://h:27017/app");
    let a = build_invocation(&cfg, None, None, "1").unwrap();
    let b = build_invocation(&cfg, None, None, "1").unwrap();
    assert_ne!(a.result_token, b.result_token);
}

#[test]
fn the_text_output_is_identical_with_and_without_the_epilogue() {
    let fake = repl();
    let with = start(&fake, &format!("a\nstderr e\nresult {PAYLOAD}\nb"));
    assert_eq!(with.wait(), Outcome::Exited(0));
    let without = start(&fake, "a\nstderr e\nb");
    assert_eq!(without.wait(), Outcome::Exited(0));
    assert_eq!(with.results(), [PAYLOAD]);
    assert!(without.results().is_empty());
    for stream in [Stream::Stdout, Stream::Stderr] {
        assert_eq!(with.text(stream), without.text(stream), "{stream:?}");
    }
    assert_eq!(with.text(Stream::Stdout), "a\nb\n");
}

#[test]
fn on_result_is_called_before_on_done() {
    let fake = repl();
    let run = start(&fake, &format!("a\nresult {PAYLOAD}"));
    assert_eq!(run.wait(), Outcome::Exited(0));
    let events = run.events.lock().unwrap().clone();
    let result_at = events
        .iter()
        .position(|e| matches!(e, Event::Result(_)))
        .expect("a result");
    let last_out = events
        .iter()
        .rposition(|e| matches!(e, Event::Out(..)))
        .expect("output");
    assert!(last_out < result_at, "{events:?}");
    assert_eq!(result_at + 1, events.len() - 1, "{events:?}");
}

#[test]
fn on_result_is_called_at_most_once() {
    let fake = repl();
    let run = start(&fake, &format!("result-twice {PAYLOAD}"));
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert_eq!(run.results(), [PAYLOAD]);
    assert_eq!(run.text(Stream::Stdout), "");
}
