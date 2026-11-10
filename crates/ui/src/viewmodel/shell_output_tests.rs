use super::*;

fn push(log: &OutputLog, stream: Stream, text: &str) -> OutputLog {
    log.appended(stream, text).0
}

fn texts(log: &OutputLog) -> Vec<String> {
    log.iter().map(|l| l.text.clone()).collect()
}

#[test]
fn empty_chunk_changes_nothing() {
    let log = push(&OutputLog::default(), Stream::Stdout, "a");
    assert_eq!(push(&log, Stream::Stdout, ""), log);
}

#[test]
fn a_lone_newline_is_an_empty_line() {
    let log = push(&OutputLog::default(), Stream::Stdout, "\n");
    assert_eq!(texts(&log), vec![""]);
    let log = push(&log, Stream::Stdout, "x\n");
    assert_eq!(texts(&log), vec!["", "x"]);
}

#[test]
fn a_newline_closes_an_open_line_without_adding_one() {
    let log = push(&OutputLog::default(), Stream::Stdout, "abc");
    assert_eq!(texts(&push(&log, Stream::Stdout, "\n")), vec!["abc"]);
}

#[test]
fn crlf_is_one_line_ending_whole_or_split() {
    let log = push(&OutputLog::default(), Stream::Stdout, "a\r\nb\r\n");
    assert_eq!(texts(&log), vec!["a", "b"]);
    let log = push(&OutputLog::default(), Stream::Stdout, "a\r");
    let log = push(&log, Stream::Stdout, "\nb");
    assert_eq!(texts(&log), vec!["a", "b"]);
    // Only one trailing CR is stripped, and only when the line closes.
    let log = push(&OutputLog::default(), Stream::Stdout, "a\r\r\n");
    assert_eq!(texts(&log), vec!["a\r"]);
    let open = push(&OutputLog::default(), Stream::Stdout, "a\r");
    assert_eq!(texts(&open), vec!["a\r"]);
}

#[test]
fn interleaved_streams_make_three_lines() {
    let log = push(&OutputLog::default(), Stream::Stdout, "out");
    let log = push(&log, Stream::Stderr, "err\n");
    let log = push(&log, Stream::Stdout, "more");
    let streams: Vec<_> = log.iter().map(|l| l.stream).collect();
    assert_eq!(texts(&log), vec!["out", "err", "more"]);
    assert_eq!(
        streams,
        vec![Stream::Stdout, Stream::Stderr, Stream::Stdout]
    );
}

#[test]
fn a_closed_log_does_not_continue_its_last_line() {
    let log = push(&OutputLog::default(), Stream::Stdout, "a");
    let log = push(&log.closed(), Stream::Stdout, "b");
    assert_eq!(texts(&log), vec!["a", "b"]);
}

#[test]
fn a_one_mebibyte_line_in_chunks_is_cut_at_the_cap() {
    let chunk = "x".repeat(8 * 1024);
    let mut log = OutputLog::default();
    for _ in 0..128 {
        log = push(&log, Stream::Stdout, &chunk);
    }
    let log = push(&log, Stream::Stdout, "\nnext\n");
    assert_eq!(log.len(), 2);
    let first = log.get(0).unwrap();
    assert_eq!(first.text.len(), MAX_LINE_BYTES);
    assert!(first.truncated);
    let second = log.get(1).unwrap();
    assert_eq!(second.text, "next");
    assert!(!second.truncated);
}

#[test]
fn an_exactly_full_line_is_not_truncated() {
    let log = push(
        &OutputLog::default(),
        Stream::Stdout,
        &"y".repeat(MAX_LINE_BYTES),
    );
    let log = push(&log, Stream::Stdout, "\n");
    assert!(!log.get(0).unwrap().truncated);
}

#[test]
fn a_multibyte_char_straddling_the_cap_is_dropped_whole() {
    let mut text = "x".repeat(MAX_LINE_BYTES - 1);
    text.push('é');
    let log = push(&OutputLog::default(), Stream::Stdout, &text);
    let line = log.get(0).unwrap();
    assert_eq!(line.text.len(), MAX_LINE_BYTES - 1);
    assert!(line.truncated);
}

#[test]
fn the_cap_holds_across_chunks_with_an_open_line_at_the_boundary() {
    let log = push(
        &OutputLog::default(),
        Stream::Stdout,
        &"a".repeat(MAX_LINE_BYTES - 2),
    );
    let log = push(&log, Stream::Stdout, "bcd");
    assert_eq!(log.get(0).unwrap().text.len(), MAX_LINE_BYTES);
    assert!(log.get(0).unwrap().truncated);
    // At the cap, further text is swallowed until the newline.
    let log = push(&log, Stream::Stdout, "zzz");
    assert_eq!(log.len(), 1);
    assert_eq!(log.get(0).unwrap().text.len(), MAX_LINE_BYTES);
    let log = push(&log, Stream::Stdout, "zz\nok\n");
    assert_eq!(log.len(), 2);
    assert_eq!(log.get(1).unwrap().text, "ok");
}

#[test]
fn dropping_the_oldest_lines_spans_segments() {
    let text: String = (0..MAX_OUTPUT_LINES + 700)
        .map(|i| format!("{i}\n"))
        .collect();
    let (log, dropped) = OutputLog::default().appended(Stream::Stdout, &text);
    assert_eq!(dropped, 700);
    assert_eq!(log.len(), MAX_OUTPUT_LINES);
    assert_eq!(log.get(0).unwrap().text, "700");
    assert_eq!(log.get(MAX_OUTPUT_LINES), None);
    // Continues to index correctly after a partial leading segment.
    let (log, dropped) = log.appended(Stream::Stdout, "a\nb\nc\n");
    assert_eq!(dropped, 3);
    assert_eq!(log.get(0).unwrap().text, "703");
    assert_eq!(log.get(MAX_OUTPUT_LINES - 1).unwrap().text, "c");
}
