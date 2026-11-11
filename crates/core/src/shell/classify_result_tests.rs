//! The payload window after the script: the epilogue's answer line is lifted out of the text.
use super::*;
use Stream::{Stderr, Stdout};

const READY: &str = "> __CB_READY__\n\n";
const P: &str = PROMPT;
const TOKEN: &str = "0123456789abcdef";
const MAX: usize = 1024;
const PAYLOAD: &str = r#"{"k":"scalar","v":{"$numberInt":"42"},"u":false}"#;
/// A one-line script block: `{a` (one continuation prompt) and `}`.
const BLOCK: [&str; 3] = [READY, PROMPT, "| a\n"];

fn marker(token: &str) -> String {
    format!("\u{1}CBJ-{token} ")
}

fn classifier() -> Classifier {
    Classifier::with_result(1, TOKEN, MAX)
}

/// Feeds the block, then `after`, then finishes.
fn feed_after_block(classifier: &mut Classifier, after: &[&str]) -> Vec<Piece> {
    let mut pieces = Vec::new();
    for chunk in BLOCK.iter().chain(after) {
        pieces.extend(classifier.push(chunk));
    }
    pieces.extend(classifier.finish());
    pieces
}

fn shown(pieces: &[Piece], stream: Stream) -> String {
    pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Out(s, text) if *s == stream => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn results(pieces: &[Piece]) -> Vec<&str> {
    pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Result(payload) => Some(payload.as_str()),
            _ => None,
        })
        .collect()
}

fn answer(payload: &str) -> String {
    format!("{}{payload}\n\n", marker(TOKEN))
}

#[test]
fn the_payload_line_is_lifted_out_of_the_text() {
    let mut c = classifier();
    let pieces = feed_after_block(&mut c, &[P, &answer(PAYLOAD), P]);
    assert!(pieces.contains(&Piece::Evaluated { epilogue: true }));
    assert_eq!(results(&pieces), [PAYLOAD]);
    assert_eq!(shown(&pieces, Stdout), "a\n");
    assert_eq!(shown(&pieces, Stderr), "");
}

// Real `mongosh` 2.14.1 bytes (`--nodb --quiet`, piped stdin, the shipped preamble) for the
// block `{db.c.insertOne({_id: 1, a: 1, when: new Date(0)}); db.c.find()` / `}`, then the
// epilogue with the token `0123456789abcdef`.
const REAL_FIND: &str = "\u{1}CBP> | [ { _id: 1, a: 1, when: ISODate('1970-01-01T00:00:00.000Z') } ]\n\u{1}CBP> \u{1}CBJ-0123456789abcdef {\"k\":\"cursor\",\"t\":\"Cursor\",\"more\":false,\"v\":[{\"_id\":{\"$numberInt\":\"1\"},\"a\":{\"$numberInt\":\"1\"},\"when\":{\"$date\":{\"$numberLong\":\"0\"}}}]}\n\n\u{1}CBP> ";

#[test]
fn real_mongosh_bytes_for_a_find_lift_the_cursor_batch() {
    let mut c = classifier();
    let mut pieces = c.push(READY);
    pieces.extend(c.push(REAL_FIND));
    pieces.extend(c.finish());
    assert_eq!(
        results(&pieces),
        [
            r#"{"k":"cursor","t":"Cursor","more":false,"v":[{"_id":{"$numberInt":"1"},"a":{"$numberInt":"1"},"when":{"$date":{"$numberLong":"0"}}}]}"#
        ]
    );
    assert_eq!(
        shown(&pieces, Stdout),
        "[ { _id: 1, a: 1, when: ISODate('1970-01-01T00:00:00.000Z') } ]\n"
    );
}

#[test]
fn a_payload_split_across_pushes_is_lifted_whole() {
    let mut c = classifier();
    let line = answer(PAYLOAD);
    let chunks: Vec<String> = line.chars().map(String::from).collect();
    let mut after: Vec<&str> = vec![P];
    after.extend(chunks.iter().map(String::as_str));
    after.push(P);
    let pieces = feed_after_block(&mut c, &after);
    assert_eq!(results(&pieces), [PAYLOAD]);
    assert_eq!(shown(&pieces, Stdout), "a\n");
}

#[test]
fn a_marker_with_the_wrong_token_is_ordinary_text() {
    let mut c = classifier();
    let spoof = format!("{}{PAYLOAD}\n", marker("ffffffffffffffff"));
    let pieces = feed_after_block(&mut c, &[P, &spoof, P]);
    assert!(results(&pieces).is_empty());
    assert_eq!(shown(&pieces, Stdout), format!("a\n{spoof}"));
}

#[test]
fn a_marker_before_evaluated_is_ordinary_text() {
    let mut c = classifier();
    let spoof = format!("{}{PAYLOAD}\n", marker(TOKEN));
    let pieces = feed_after_block(&mut c, &[&spoof, P, P]);
    assert!(results(&pieces).is_empty());
    assert_eq!(shown(&pieces, Stdout), format!("a\n{spoof}"));
}

#[test]
fn the_blank_line_after_the_payload_is_dropped() {
    let mut c = classifier();
    let line = format!("{}{PAYLOAD}\n", marker(TOKEN));
    // The blank `print` leaves comes in its own read; one more blank line is real output.
    let pieces = feed_after_block(&mut c, &[P, &line, "\n", "\n", P]);
    assert_eq!(results(&pieces), [PAYLOAD]);
    assert_eq!(shown(&pieces, Stdout), "a\n\n");
}

#[test]
fn late_async_output_in_the_window_stays_text() {
    let mut c = classifier();
    let after = [P, "late-1\n", &answer(PAYLOAD), "late-2\n", P, "late-3\n"];
    let pieces = feed_after_block(&mut c, &after);
    assert_eq!(results(&pieces), [PAYLOAD]);
    assert_eq!(shown(&pieces, Stdout), "a\nlate-1\nlate-2\nlate-3\n");
}

#[test]
fn a_marker_after_the_window_closed_is_ordinary_text() {
    let mut c = classifier();
    let late = format!("{}{PAYLOAD}\n", marker(TOKEN));
    let pieces = feed_after_block(&mut c, &[P, P, &late]);
    assert!(results(&pieces).is_empty());
    assert_eq!(shown(&pieces, Stdout), format!("a\n{late}"));
}

#[test]
fn an_oversize_payload_is_dropped_and_reported() {
    let mut c = Classifier::with_result(1, TOKEN, 16);
    let big = "x".repeat(1000);
    let m = marker(TOKEN);
    let mut after = vec![P, m.as_str()];
    after.extend(std::iter::repeat_n(big.as_str(), 10));
    after.extend(["\n", "\n", "after\n", P]);
    let mut pieces = Vec::new();
    for chunk in BLOCK.iter().chain(&after) {
        pieces.extend(c.push(chunk));
        assert!(
            c.result_buffered() <= 16,
            "buffered {}",
            c.result_buffered()
        );
    }
    pieces.extend(c.finish());
    assert!(results(&pieces).is_empty());
    assert_eq!(count(&pieces, &Piece::ResultTooLarge), 1);
    assert_eq!(shown(&pieces, Stdout), "a\nafter\n");
}

#[test]
fn a_payload_of_exactly_the_cap_is_kept() {
    let payload = "y".repeat(16);
    let mut c = Classifier::with_result(1, TOKEN, 16);
    let pieces = feed_after_block(&mut c, &[P, &answer(&payload), P]);
    assert_eq!(results(&pieces), [payload.as_str()]);
}

#[test]
fn an_unterminated_payload_at_the_end_is_dropped() {
    let mut c = classifier();
    let cut = format!("{}{PAYLOAD}", marker(TOKEN));
    let pieces = feed_after_block(&mut c, &[P, &cut]);
    assert!(results(&pieces).is_empty());
    assert_eq!(shown(&pieces, Stdout), "a\n");
}

fn count(pieces: &[Piece], wanted: &Piece) -> usize {
    pieces.iter().filter(|p| *p == wanted).count()
}

/// The verdict given with `Evaluated` for the block's output `body` (after the `| ` prompt).
fn verdict(classifier: &mut Classifier, body: &str) -> Vec<Piece> {
    let mut pieces = Vec::new();
    for chunk in [READY, P, "| ", body, P] {
        pieces.extend(classifier.push(chunk));
    }
    pieces
}

#[test]
fn a_clean_block_asks_for_the_epilogue() {
    let pieces = verdict(&mut classifier(), "a\n");
    assert_eq!(count(&pieces, &Piece::Evaluated { epilogue: true }), 1);
}

#[test]
fn an_uncaught_error_gives_no_epilogue() {
    let pieces = verdict(&mut classifier(), "Uncaught \rError: x\n");
    assert_eq!(count(&pieces, &Piece::Evaluated { epilogue: false }), 1);
}

#[test]
fn an_early_end_gives_no_epilogue() {
    // Two continuations expected, the prompt came back after one line.
    let mut c = Classifier::with_result(2, TOKEN, MAX);
    let pieces = verdict(&mut c, "a\n");
    assert_eq!(count(&pieces, &Piece::Evaluated { epilogue: false }), 1);
}

#[test]
fn the_underscore_assignment_text_gives_no_epilogue_even_split() {
    let mut c = classifier();
    let mut pieces = Vec::new();
    for chunk in [
        READY,
        P,
        "| Expression assign",
        "ment to _ now disa",
        "bled.\nx\n",
        P,
    ] {
        pieces.extend(c.push(chunk));
    }
    assert_eq!(count(&pieces, &Piece::Evaluated { epilogue: false }), 1);
}

#[test]
fn without_a_token_there_is_never_an_epilogue_or_a_payload() {
    let mut c = Classifier::expecting(1);
    let pieces = feed_after_block(&mut c, &[P, &answer(PAYLOAD), P]);
    assert_eq!(count(&pieces, &Piece::Evaluated { epilogue: false }), 1);
    assert!(results(&pieces).is_empty());
}

#[test]
fn a_continuation_prompt_past_the_last_line_gives_no_epilogue_at_once() {
    // A bracket is still open: `mongosh` waits for a line that will never come, so the writer
    // must hear "no epilogue" now (and close stdin), not after the end of the output.
    let mut c = classifier();
    let mut pieces = Vec::new();
    for chunk in [READY, P, "| ", "| "] {
        pieces.extend(c.push(chunk));
    }
    assert_eq!(count(&pieces, &Piece::NoEpilogue), 1);
    pieces.extend(c.finish());
    assert!(c.incomplete());
    assert!(!pieces.iter().any(|p| matches!(p, Piece::Evaluated { .. })));
}

#[test]
fn every_marker_line_in_the_window_is_lifted() {
    // The lane delivers the first only; none of them is ever text.
    let mut c = classifier();
    let twice = answer(PAYLOAD).repeat(2);
    let pieces = feed_after_block(&mut c, &[P, &twice, P]);
    assert_eq!(results(&pieces), [PAYLOAD, PAYLOAD]);
    assert_eq!(shown(&pieces, Stdout), "a\n");
}

#[test]
fn the_window_is_open_from_the_verdict_to_the_next_prompt() {
    let mut c = classifier();
    for chunk in BLOCK {
        c.push(chunk);
    }
    assert!(!c.in_result_window());
    c.push(P);
    assert!(c.in_result_window());
    c.push(&answer(PAYLOAD));
    assert!(c.in_result_window());
    c.push(P);
    assert!(!c.in_result_window());
}
