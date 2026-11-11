//! A bare `|` / `| ` at a read boundary right after the last script line, end to end through the
//! fake `mongosh`: output that starts with `|` keeps its result, an unfinished block still ends
//! soon (bounded by the grace), and Cancel during the grace ends the run.
use super::*;

const PAYLOAD: &str = r#"{"k":"scalar","v":{"$numberInt":"42"},"u":false}"#;

/// The fake prints `first`, pauses (a separate read), then `second`; both are `printf` formats
/// without spaces (`\040` is a space).
fn split_first_line(first: &str, second: &str) -> String {
    format!("split {first} {second}\nresult {PAYLOAD}")
}

#[test]
fn output_starting_with_a_bar_split_after_the_bar_still_gets_its_result() {
    for (first, second) in [("|", "\\040a\\040|\\n"), ("|\\040", "a\\040|\\n")] {
        let fake = repl();
        let run = start(&fake, &split_first_line(first, second));
        assert_eq!(run.wait(), Outcome::Exited(0), "{first:?}");
        assert_eq!(run.text(Stream::Stdout), "| a |\n", "{first:?}");
        assert_eq!(run.results(), [PAYLOAD], "{first:?}");
    }
}

#[test]
fn output_starting_with_a_bar_split_elsewhere_still_gets_its_result() {
    let fake = repl();
    let run = start(&fake, &split_first_line("|\\040a", "\\040|\\n"));
    assert_eq!(run.wait(), Outcome::Exited(0));
    assert_eq!(run.text(Stream::Stdout), "| a |\n");
    assert_eq!(run.results(), [PAYLOAD]);
}

#[test]
fn an_unfinished_block_still_ends_within_the_grace() {
    let fake = repl();
    let limits = RunLimits::default();
    let started = Instant::now();
    let run = start_with(
        &fake.exe,
        invocation(&format!("result {PAYLOAD}\nfoo(")),
        limits,
    );
    assert_eq!(run.wait(), Outcome::Incomplete);
    assert!(run.results().is_empty());
    let bound = limits.continuation_grace + Duration::from_secs(5);
    assert!(started.elapsed() < bound, "took {:?}", started.elapsed());
}

#[test]
fn cancel_during_the_grace_ends_the_run_cancelled() {
    // A long grace: the run can only end before it by the cancel.
    let fake = repl();
    let limits = RunLimits {
        continuation_grace: Duration::from_secs(60),
        ..RunLimits::default()
    };
    let run = start_with(&fake.exe, invocation("bar-hang"), limits);
    // `bar-out` comes after the `|` that started the grace.
    run.wait_for_text(Stream::Stderr, "bar-out\n");
    let cancelled_at = Instant::now();
    run.handle().cancel();
    assert_eq!(run.wait(), Outcome::Cancelled);
    assert!(cancelled_at.elapsed() < Duration::from_secs(5));
    assert!(run.results().is_empty());
}
