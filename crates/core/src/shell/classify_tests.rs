use super::*;
use Stream::{Stderr, Stdout};

/// Feeds the chunks, then finishes; returns everything it produced.
fn feed(chunks: &[&str]) -> (Vec<Piece>, bool) {
    let mut classifier = Classifier::default();
    let mut pieces = Vec::new();
    for chunk in chunks {
        pieces.extend(classifier.push(chunk));
    }
    pieces.extend(classifier.finish());
    (pieces, classifier.saw_uncaught())
}

/// The text shown on `stream`, after the handshake.
fn shown(pieces: &[Piece], stream: Stream) -> String {
    pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Out(s, text) if *s == stream => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// What `mongosh` prints for the preamble: the default prompt, the sentinel and `print`'s blank.
const READY: &str = "> __CB_READY__\n\n";
/// The fixed prompt the preamble sets.
const P: &str = PROMPT;

fn after_ready(body: &str) -> (Vec<Piece>, bool) {
    feed(&[READY, body])
}

#[test]
fn nothing_before_the_ready_sentinel_is_shown() {
    let (pieces, _) = feed(&["noise\n> __CB_", "READY__\n\n\u{1}CBP> 2\n\u{1}CBP> "]);
    assert_eq!(pieces[0], Piece::Ready);
    assert_eq!(pieces[1..], [Piece::Out(Stdout, "2\n".into())]);
}

#[test]
fn the_failed_sentinel_carries_its_message_and_ends_the_output() {
    let (pieces, _) = feed(&["> __CB_FAILED__ Authentication failed.\n\n> more\n"]);
    assert_eq!(pieces, [Piece::Failed("Authentication failed.".into())]);
}

#[test]
fn a_failed_message_containing_a_gt_is_kept_whole() {
    let (pieces, _) = feed(&["> __CB_FAILED__ expected a> b\n"]);
    assert_eq!(pieces, [Piece::Failed("expected a> b".into())]);
}

#[test]
fn a_failed_sentinel_without_a_newline_is_seen_at_the_end() {
    let (pieces, _) = feed(&["> __CB_FAILED__ gone"]);
    assert_eq!(pieces, [Piece::Failed("gone".into())]);
}

#[test]
fn sentinels_after_default_replica_set_mongos_and_atlas_prompts_are_recognised() {
    for prompt in [
        "rs0 [primary] admin> ",
        "rs0 [direct: primary] admin> ",
        "[mongos] admin> ",
        "Atlas atlas-xyz-shard-0 [primary] admin> ",
    ] {
        let (pieces, _) = feed(&[&format!("{prompt}__CB_READY__\n\n{P}1\n{P}")]);
        assert_eq!(pieces[0], Piece::Ready, "{prompt}");
        assert_eq!(shown(&pieces, Stdout), "1\n", "{prompt}");
    }
}

#[test]
fn output_without_a_sentinel_reports_the_last_lines() {
    let (pieces, _) = feed(&["> Error: cannot start\r\n> \n", "at line 2\npartial"]);
    assert_eq!(
        pieces,
        [Piece::NoSentinel(
            "Error: cannot start\nat line 2\npartial\n".into()
        )]
    );
}

#[test]
fn only_the_last_twenty_unready_lines_are_kept() {
    let text: String = (0..30).map(|i| format!("line {i}\n")).collect();
    let (pieces, _) = feed(&[&text]);
    let expected: String = (10..30).map(|i| format!("line {i}\n")).collect();
    assert_eq!(pieces, [Piece::NoSentinel(expected)]);
}

#[test]
fn no_output_at_all_is_an_empty_no_sentinel() {
    let (pieces, _) = feed(&[]);
    assert_eq!(pieces, [Piece::NoSentinel(String::new())]);
}

#[test]
fn the_fixed_prompt_is_stripped() {
    let (pieces, _) = after_ready(&format!("{P}1\n{P}2\n{P}{P}3\n{P}"));
    assert_eq!(shown(&pieces, Stdout), "1\n2\n3\n");
}

#[test]
fn the_fixed_prompt_split_across_pushes_is_stripped() {
    let (pieces, _) = feed(&[READY, "\u{1}C", "BP", ">", " ok\n\u{1}", "CBP> "]);
    assert_eq!(shown(&pieces, Stdout), "ok\n");
}

#[test]
fn the_prompt_after_unterminated_output_is_stripped() {
    let (pieces, _) = after_ready(&format!("{P}abc{P}def\n{P}"));
    assert_eq!(shown(&pieces, Stdout), "abcdef\n");
}

#[test]
fn default_style_prompts_in_output_are_left_alone() {
    // With the fixed prompt, text that looks like a default prompt is the user's output.
    let body = format!(
        "{P}rs0 [primary] admin> x\n[mongos] admin> y\nAtlas a-0 [primary] admin> z\n> w\n{P}"
    );
    let (pieces, _) = after_ready(&body);
    assert_eq!(
        shown(&pieces, Stdout),
        "rs0 [primary] admin> x\n[mongos] admin> y\nAtlas a-0 [primary] admin> z\n> w\n"
    );
}

#[test]
fn a_lone_control_character_is_output() {
    let (pieces, _) = after_ready(&format!("{P}a\u{1}b\n{P}"));
    assert_eq!(shown(&pieces, Stdout), "a\u{1}b\n");
}

#[test]
fn carriage_returns_are_removed() {
    let (pieces, _) = after_ready(&format!("{P}a\r\nb\n{P}"));
    assert_eq!(shown(&pieces, Stdout), "a\nb\n");
}

#[test]
fn print_output_keeps_its_blank_line() {
    let (pieces, _) = after_ready(&format!("{P}x\n\n{P}"));
    assert_eq!(shown(&pieces, Stdout), "x\n\n");
}

#[test]
fn an_uncaught_block_goes_to_stderr_until_the_next_prompt() {
    let (pieces, uncaught) = after_ready(&format!(
        "{P}1\n{P}Uncaught \rError: boom\n  at x\n{P}2\n{P}"
    ));
    assert!(uncaught);
    assert_eq!(shown(&pieces, Stdout), "1\n2\n");
    assert_eq!(shown(&pieces, Stderr), "Uncaught Error: boom\n  at x\n");
}

#[test]
fn a_syntax_error_with_its_code_frame_goes_to_stderr() {
    let raw = format!(
        "{P}Uncaught:\n\rSyntaxError: Unexpected token (1:4)\n\n> 1 | var = = 3\n    |     ^\n  2 |\n\n{P}ok\n{P}"
    );
    let (pieces, uncaught) = after_ready(&raw);
    assert!(uncaught);
    assert_eq!(
        shown(&pieces, Stderr),
        "Uncaught:\nSyntaxError: Unexpected token (1:4)\n\n> 1 | var = = 3\n    |     ^\n  2 |\n\n"
    );
    assert_eq!(shown(&pieces, Stdout), "ok\n");
}

#[test]
fn an_error_marker_split_across_pushes_is_still_an_error() {
    let (pieces, uncaught) = feed(&[READY, "\u{1}CBP> Unc", "aught ", "\rError: x\n\u{1}CBP> "]);
    assert!(uncaught);
    assert_eq!(shown(&pieces, Stderr), "Uncaught Error: x\n");
}

#[test]
fn printed_text_starting_with_uncaught_is_ordinary_output() {
    let (pieces, uncaught) = after_ready(&format!("{P}Uncaught x\n\n{P}Uncaught:\nno\n{P}"));
    assert!(!uncaught);
    assert_eq!(shown(&pieces, Stdout), "Uncaught x\n\nUncaught:\nno\n");
    assert_eq!(shown(&pieces, Stderr), "");
}

#[test]
fn only_a_line_starting_with_the_marker_is_an_error() {
    let (_, uncaught) = after_ready(&format!("{P}not Uncaught \rx\n{P}"));
    assert!(!uncaught);
}

#[test]
fn text_without_a_newline_streams_without_waiting_for_the_line_end() {
    let mut classifier = Classifier::default();
    classifier.push(READY);
    classifier.push(P);
    let long = "x".repeat(200);
    assert_eq!(classifier.push(&long), [Piece::Out(Stdout, long.clone())]);
    assert_eq!(classifier.push("yy"), [Piece::Out(Stdout, "yy".into())]);
}

#[test]
fn a_line_start_is_held_only_while_it_could_be_a_prompt_or_an_error() {
    let mut classifier = Classifier::default();
    classifier.push(READY);
    assert!(classifier.push("Unc").is_empty());
    assert_eq!(
        classifier.push("le\n"),
        [Piece::Out(Stdout, "Uncle\n".into())]
    );
    assert!(classifier.push("\u{1}CB").is_empty());
    assert_eq!(classifier.finish(), [Piece::Out(Stdout, "\u{1}CB".into())]);
}
