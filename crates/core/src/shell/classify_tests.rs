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

const READY: &str = "> __CB_READY__\n\n";

#[test]
fn nothing_before_the_ready_sentinel_is_shown() {
    let (pieces, _) = feed(&["noise\n> __CB_", "READY__\n\nadmin> 2\nadmin> "]);
    assert_eq!(pieces[0], Piece::Ready);
    assert_eq!(pieces[1..], [Piece::Out(Stdout, "2\n".into())]);
}

#[test]
fn the_failed_sentinel_carries_its_message_and_ends_the_output() {
    let (pieces, _) = feed(&["> __CB_FAILED__ Authentication failed.\n\n> more\n"]);
    assert_eq!(pieces, [Piece::Failed("Authentication failed.".into())]);
}

#[test]
fn a_failed_sentinel_without_a_newline_is_seen_at_the_end() {
    let (pieces, _) = feed(&["> __CB_FAILED__ gone"]);
    assert_eq!(pieces, [Piece::Failed("gone".into())]);
}

#[test]
fn output_without_a_sentinel_shows_nothing() {
    let (pieces, _) = feed(&["> something\nelse"]);
    assert!(pieces.is_empty(), "{pieces:?}");
}

#[test]
fn prompts_with_and_without_a_database_name_are_stripped() {
    let (pieces, _) = feed(&[READY, "test> 1\nmy-db.x_1> 2\n> 3\n> "]);
    assert_eq!(shown(&pieces, Stdout), "1\n2\n3\n");
}

#[test]
fn a_prompt_split_across_pushes_is_stripped() {
    let (pieces, _) = feed(&[READY, "adm", "in", ">", " ok\nadmin> "]);
    assert_eq!(shown(&pieces, Stdout), "ok\n");
}

#[test]
fn a_bare_trailing_prompt_after_text_is_stripped() {
    let (pieces, _) = feed(&[READY, "a b> "]);
    assert_eq!(shown(&pieces, Stdout), "a b");
}

#[test]
fn the_current_prompt_after_unterminated_output_is_stripped_at_the_end() {
    let (pieces, _) = feed(&[READY, "admin> x yad", "min", "> "]);
    assert_eq!(shown(&pieces, Stdout), "x y");
}

#[test]
fn text_that_only_looks_like_the_prompt_start_is_released() {
    let (pieces, _) = feed(&[READY, "admin> xad", "z\n> "]);
    assert_eq!(shown(&pieces, Stdout), "xadz\n");
    let (pieces, _) = feed(&[READY, "admin> xadm"]);
    assert_eq!(shown(&pieces, Stdout), "xadm");
}

#[test]
fn a_gt_inside_a_line_is_kept() {
    let (pieces, _) = feed(&[READY, "a > b\n1>2\n> "]);
    assert_eq!(shown(&pieces, Stdout), "a > b\n1>2\n");
}

#[test]
fn carriage_returns_are_removed() {
    let (pieces, _) = feed(&[READY, "a\r\nb\n> "]);
    assert_eq!(shown(&pieces, Stdout), "a\nb\n");
}

#[test]
fn print_output_keeps_its_blank_line() {
    let (pieces, _) = feed(&[READY, "x\n\n> "]);
    assert_eq!(shown(&pieces, Stdout), "x\n\n");
}

#[test]
fn an_uncaught_block_goes_to_stderr_until_the_next_prompt() {
    let (pieces, uncaught) = feed(&[READY, "1\n> Uncaught \rError: boom\n  at x\n> 2\n> "]);
    assert!(uncaught);
    assert_eq!(shown(&pieces, Stdout), "1\n2\n");
    assert_eq!(shown(&pieces, Stderr), "Uncaught Error: boom\n  at x\n");
}

#[test]
fn a_syntax_error_code_frame_stays_in_the_error_block() {
    let raw = "> Uncaught:\n\rSyntaxError: Unexpected token (1:4)\n\n> 1 | var = = 3\n    |     ^\n  2 |\n\n> ok\n> ";
    let (pieces, uncaught) = feed(&[READY, raw]);
    assert!(uncaught);
    assert_eq!(
        shown(&pieces, Stderr),
        "Uncaught:\nSyntaxError: Unexpected token (1:4)\n\n> 1 | var = = 3\n    |     ^\n  2 |\n\n"
    );
    assert_eq!(shown(&pieces, Stdout), "ok\n");
}

#[test]
fn a_line_split_inside_uncaught_is_still_an_error() {
    let (pieces, uncaught) = feed(&[READY, "> Unc", "aught \rError: x\n> "]);
    assert!(uncaught);
    assert_eq!(shown(&pieces, Stderr), "Uncaught Error: x\n");
}

#[test]
fn only_a_line_starting_with_uncaught_is_an_error() {
    let (_, uncaught) = feed(&[READY, "not Uncaught\n> "]);
    assert!(!uncaught);
}

#[test]
fn text_without_a_newline_streams_without_waiting_for_the_line_end() {
    let mut classifier = Classifier::default();
    classifier.push(READY);
    let long = "x".repeat(200);
    assert_eq!(classifier.push(&long), [Piece::Out(Stdout, long.clone())]);
    assert_eq!(classifier.push("yy"), [Piece::Out(Stdout, "yy".into())]);
}

#[test]
fn a_short_line_start_is_held_only_while_it_could_be_a_prompt() {
    let mut classifier = Classifier::default();
    classifier.push(READY);
    assert!(classifier.push("abc").is_empty());
    assert_eq!(classifier.push(" d"), [Piece::Out(Stdout, "abc d".into())]);
    assert_eq!(classifier.push("\nxyz"), [Piece::Out(Stdout, "\n".into())]);
    assert_eq!(classifier.finish(), [Piece::Out(Stdout, "xyz".into())]);
}
