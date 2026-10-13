//! The differences report through the runtime: what the CSV holds, the clipboard text, the
//! refusals and the shared rig. In-memory stores; temp directories are removed by each test.

use std::path::Path;

use bson::{Bson, Document, doc};

use super::apply_tests::{ApplyStore, within};
use super::export_tests::{
    ExRig, SRC_NAME, SRC_URI, TGT_NAME, TGT_URI, collect_export, outcome_of, rig_with,
};
use super::tests::{next_compare, ns, pair, same_pair};
use crate::compare::key::KeyTuple;
use crate::compare::protocol::{
    CompareCommand, CompareEvent, ExportOutcome, PairId, ReportRequest, ReportTarget,
};
use crate::compare::results::Category;
use crate::error::CoreError;
use crate::runtime::Command;
use crate::store::DocumentStore;
use crate::test_support::InMemoryStore;

pub(super) const SENTINEL: &str = "VALUE-SENTINEL-93";
const EVIL: &str = "=evil";

/// A small RFC 4180 reader: rows of cells; quoted cells may hold commas, quotes and line breaks.
pub(super) fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                chars.next();
                cell.push('"');
            }
            (true, '"') => quoted = false,
            (true, c) => cell.push(c),
            (false, '"') => quoted = true,
            (false, ',') => row.push(std::mem::take(&mut cell)),
            (false, '\r') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            (false, c) => cell.push(c),
        }
    }
    assert!(!quoted, "an unterminated quoted cell");
    rows
}

fn text_doc(id: &str, rest: Document) -> Document {
    let mut d = doc! { "_id": id };
    d.extend(rest);
    d
}

fn int_doc(id: i32, a: i32) -> Document {
    doc! { "_id": id, "a": a }
}

/// Pair 1 (`d.c`): 5 different, 2 only in source, 1 only in target, 1 equal; keys that are
/// hostile in CSV. Pair 2 (`d.=evil`): 1 each of the three categories and 1 equal. Pair 3
/// (`d.same`): nothing differs.
pub(super) fn source_data() -> InMemoryStore {
    InMemoryStore::new()
        .with_docs(
            &ns("d", "c"),
            vec![
                text_doc("a,b", doc! { "n": { "m": 1 }, "v": SENTINEL }),
                text_doc("q\"uote", doc! { "v": 1 }),
                text_doc("new\nline", doc! { "v": 1 }),
                text_doc("-1", doc! { "list": [1, 2] }),
                text_doc("semi;colon", doc! { "v": 1 }),
                text_doc("same", doc! { "v": 0 }),
                text_doc("=cmd()", doc! { "v": SENTINEL }),
                text_doc("@x", doc! { "v": SENTINEL }),
            ],
        )
        .with_docs(
            &ns("d", EVIL),
            vec![int_doc(1, 1), int_doc(2, 1), int_doc(3, 1)],
        )
        .with_docs(&ns("d", "same"), vec![int_doc(1, 1), int_doc(2, 2)])
}

pub(super) fn target_data() -> InMemoryStore {
    InMemoryStore::new()
        .with_docs(
            &ns("d", "c"),
            vec![
                text_doc("a,b", doc! { "n": { "m": 2 }, "v": "x" }),
                text_doc("q\"uote", doc! { "v": 2 }),
                text_doc("new\nline", doc! { "v": 2 }),
                text_doc("-1", doc! { "list": [1, 3] }),
                text_doc("semi;colon", doc! { "v": 2 }),
                text_doc("same", doc! { "v": 0 }),
                text_doc("+1", doc! { "v": 1 }),
            ],
        )
        .with_docs(
            &ns("d", EVIL),
            vec![int_doc(1, 2), int_doc(3, 1), int_doc(9, 1)],
        )
        .with_docs(&ns("d", "same"), vec![int_doc(1, 1), int_doc(2, 2)])
}

pub(super) async fn report_rig() -> ExRig {
    rig_with(
        ApplyStore::new(source_data()),
        ApplyStore::new(target_data()),
        vec![
            same_pair(1),
            pair(2, ns("d", EVIL), ns("d", EVIL)),
            pair(3, ns("d", "same"), ns("d", "same")),
        ],
    )
    .await
}

pub(super) fn report_request(pairs: Vec<PairId>, include_paths: bool) -> ReportRequest {
    ReportRequest {
        pairs,
        include_paths,
        label: "LABEL-1".into(),
    }
}

pub(super) fn send_report(
    tx: &super::tests::Tx,
    run: u64,
    request: ReportRequest,
    target: ReportTarget,
) {
    tx.send(Command::Compare(CompareCommand::ExportReport {
        session: 1,
        run,
        request,
        target,
    }))
    .unwrap();
}

/// Runs one report to its end and returns its events.
pub(super) async fn report(
    r: &mut ExRig,
    run: u64,
    request: ReportRequest,
    target: ReportTarget,
) -> Vec<CompareEvent> {
    send_report(&r.tx, run, request, target);
    within(collect_export(&mut r.h)).await
}

pub(super) fn file_target(r: &ExRig, name: &str) -> (ReportTarget, std::path::PathBuf) {
    let path = r.path(name);
    (ReportTarget::File(path.clone()), path)
}

pub(super) fn text_of(events: &[CompareEvent]) -> String {
    let texts: Vec<&String> = events
        .iter()
        .filter_map(|e| match e {
            CompareEvent::ExportText { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), 1, "exactly one ExportText");
    texts[0].clone()
}

pub(super) fn failure_of(events: &[CompareEvent]) -> CoreError {
    match events {
        [
            CompareEvent::Failed {
                context: "compare_export",
                error,
                ..
            },
        ] => error.clone(),
        other => panic!("{other:?}"),
    }
}

/// The stored keys of a pair per category, in stored order (the `Rows` command is the oracle).
async fn stored_keys(r: &mut ExRig, pair: PairId, category: Category) -> Vec<KeyTuple> {
    super::tests::send_rows(&r.tx, pair, category);
    loop {
        if let CompareEvent::Rows { rows, total, .. } = next_compare(&mut r.h).await {
            assert_eq!(rows.len(), total, "one page holds the rows of this test");
            return rows.into_iter().map(|row| row.key).collect();
        }
    }
}

fn key_json(key: &KeyTuple) -> String {
    let parts: Vec<serde_json::Value> = key
        .0
        .iter()
        .map(|b| match b {
            Bson::String(s) => serde_json::Value::String(s.clone()),
            Bson::Int32(n) => serde_json::Value::from(*n),
            other => panic!("{other:?}"),
        })
        .collect();
    serde_json::Value::Array(parts).to_string()
}

const CATEGORIES: [(Category, &str); 3] = [
    (Category::OnlyInSource, "only_in_source"),
    (Category::OnlyInTarget, "only_in_target"),
    (Category::Different, "different"),
];

/// The names of a pair as the report shows them.
fn names(pair: PairId) -> (&'static str, &'static str) {
    match pair {
        1 => ("d.c", "d.c"),
        2 => ("d.=evil", "d.=evil"),
        _ => ("d.same", "d.same"),
    }
}

/// The data rows (after the header row) of a report.
pub(super) fn data_rows(text: &str) -> Vec<Vec<String>> {
    let rows = parse_csv(text);
    let header = rows
        .iter()
        .position(|r| r[0] == "pair")
        .expect("the header row");
    rows[header + 1..].to_vec()
}

pub(super) fn comment_lines(text: &str) -> Vec<String> {
    let rows = parse_csv(text);
    let header = rows.iter().position(|r| r[0] == "pair").unwrap();
    rows[..header]
        .iter()
        .map(|r| {
            assert_eq!(r.len(), 1, "a summary line must be ONE cell: {r:?}");
            assert!(r[0].starts_with('#'));
            r[0].clone()
        })
        .collect()
}

/// The report as written to a file.
pub(super) async fn file_report(
    r: &mut ExRig,
    run: u64,
    pairs: Vec<PairId>,
    paths: bool,
) -> (String, ExportOutcome) {
    let (target, path) = file_target(r, &format!("report-{run}.csv"));
    let events = report(r, run, report_request(pairs, paths), target).await;
    let outcome = outcome_of(&events);
    (std::fs::read_to_string(&path).unwrap(), outcome)
}

#[tokio::test]
async fn rows_equal_the_stored_differences_of_every_category_for_two_pairs_in_request_order() {
    let mut r = report_rig().await;
    let (text, outcome) = file_report(&mut r, 5, vec![2, 1], false).await;
    let mut expected: Vec<Vec<String>> = Vec::new();
    for (position, pair) in [(1, 2), (2, 1)] {
        let (source, target) = names(pair);
        for (category, name) in CATEGORIES {
            for key in stored_keys(&mut r, pair, category).await {
                expected.push(vec![
                    position.to_string(),
                    source.into(),
                    target.into(),
                    name.into(),
                    key_json(&key),
                    String::new(),
                ]);
            }
        }
    }
    assert_eq!(expected.len(), 3 + 8, "3 rows of pair 2 and 8 of pair 1");
    assert_eq!(data_rows(&text), expected);
    assert_eq!(
        (
            outcome.written,
            outcome.skipped,
            outcome.failed,
            outcome.not_run
        ),
        (11, 0, 0, 0)
    );
    assert_eq!(outcome.bytes, text.len() as u64);
    assert!(!outcome.cancelled && !outcome.file_left && !outcome.partial_removed);
}

#[tokio::test]
async fn the_summary_block_matches_the_stored_counts_and_each_line_is_one_cell() {
    let mut r = report_rig().await;
    let (text, _) = file_report(&mut r, 5, vec![2, 1, 3], false).await;
    let lines = comment_lines(&text);
    let counts = |os, ot, d, eq, ss, st| {
        format!(
            "only_in_source={os}, only_in_target={ot}, different={d}, identical={eq}, \
             scanned_source={ss}, scanned_target={st}"
        )
    };
    assert_eq!(
        lines,
        vec![
            "# curlybrace data compare report".to_string(),
            "# label: LABEL-1".to_string(),
            "# pairs: 3".to_string(),
            format!("# pair 1: d.=evil -> d.=evil: {}", counts(1, 1, 1, 1, 3, 3)),
            format!("# pair 2: d.c -> d.c: {}", counts(2, 1, 5, 1, 8, 7)),
            format!("# pair 3: d.same -> d.same: {}", counts(0, 0, 0, 2, 2, 2)),
            format!("# total: {}", counts(3, 2, 6, 4, 13, 12)),
        ]
    );
    // The counts printed are the ones the Rows command reports for the stored results.
    for (pair, category, n) in [(1, Category::Different, 5), (2, Category::Different, 1)] {
        assert_eq!(stored_keys(&mut r, pair, category).await.len(), n);
    }
}

#[tokio::test]
async fn a_pair_without_differences_yields_the_summary_and_the_header_only() {
    let mut r = report_rig().await;
    let (text, outcome) = file_report(&mut r, 5, vec![3], true).await;
    assert!(data_rows(&text).is_empty());
    assert_eq!(comment_lines(&text).len(), 5);
    assert_eq!(
        (
            outcome.written,
            outcome.skipped,
            outcome.failed,
            outcome.not_run
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(outcome.bytes, text.len() as u64);
}

#[tokio::test]
async fn include_paths_fills_different_rows_with_display_paths_and_never_values() {
    let mut r = report_rig().await;
    let (text, outcome) = file_report(&mut r, 5, vec![1, 2], true).await;
    assert!(!text.contains(SENTINEL), "a value reached the report");
    for forbidden in ["\"x\"", "SENTINEL"] {
        assert!(!text.contains(forbidden));
    }
    let paths_of = |pair_pos: &str, key: &str| -> Vec<String> {
        let rows = data_rows(&text);
        let row = rows
            .iter()
            .find(|r| r[0] == pair_pos && r[4] == key && r[3] == "different")
            .unwrap_or_else(|| panic!("no row for {key}"));
        let mut paths: Vec<String> = if row[5].is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(&row[5]).unwrap()
        };
        paths.sort();
        paths
    };
    assert_eq!(paths_of("1", "[\"a,b\"]"), ["n.m", "v"]);
    assert_eq!(paths_of("1", "[\"q\\\"uote\"]"), ["v"]);
    assert_eq!(paths_of("1", "[\"-1\"]"), ["list[1]"]);
    assert_eq!(paths_of("2", "[1]"), ["a"]);
    let filled: Vec<_> = data_rows(&text)
        .into_iter()
        .filter(|r| !r[5].is_empty())
        .collect();
    assert_eq!(filled.len(), 6, "exactly the different rows");
    assert!(filled.iter().all(|r| r[3] == "different"));
    assert_eq!(
        (outcome.written, outcome.skipped, outcome.failed),
        (11, 0, 0)
    );
}

#[tokio::test]
async fn documents_that_changed_or_vanished_leave_the_cell_empty_and_are_counted() {
    let mut r = report_rig().await;
    let changed = text_doc("a,b", doc! { "v": "moved" });
    r.source
        .replace(&ns("d", "c"), &Bson::String("a,b".into()), changed)
        .await
        .unwrap();
    r.target
        .delete(&ns("d", "c"), &Bson::String("-1".into()))
        .await
        .unwrap();
    let (text, outcome) = file_report(&mut r, 5, vec![1], true).await;
    let rows = data_rows(&text);
    let empty: Vec<&str> = rows
        .iter()
        .filter(|r| r[3] == "different" && r[5].is_empty())
        .map(|r| r[4].as_str())
        .collect();
    assert_eq!(empty.len(), 2, "{empty:?}");
    assert!(empty.contains(&"[\"a,b\"]") && empty.contains(&"[\"-1\"]"));
    assert_eq!(rows.len(), 8, "the rows are still listed");
    assert_eq!(
        (outcome.written, outcome.skipped, outcome.failed),
        (8, 2, 0)
    );
}

#[tokio::test]
async fn unreadable_documents_are_counted_as_failed_and_the_rows_stay() {
    let mut r = report_rig().await;
    r.source
        .fail_reads
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (text, outcome) = file_report(&mut r, 5, vec![1], true).await;
    let rows = data_rows(&text);
    assert_eq!(rows.len(), 8);
    assert!(rows.iter().all(|r| r[5].is_empty()));
    assert_eq!(
        (outcome.written, outcome.skipped, outcome.failed),
        (8, 0, 5)
    );
}

#[tokio::test]
async fn the_clipboard_text_equals_the_file_and_arrives_before_the_one_done() {
    let mut r = report_rig().await;
    let (file, _) = file_report(&mut r, 5, vec![1, 2], true).await;
    let events = report(
        &mut r,
        6,
        report_request(vec![1, 2], true),
        ReportTarget::Clipboard,
    )
    .await;
    let text = text_of(&events);
    assert_eq!(text, file);
    let outcome = outcome_of(&events);
    assert_eq!((outcome.written, outcome.bytes), (11, text.len() as u64));
    assert!(!outcome.partial_removed && !outcome.file_left);
    assert_event_order(&events);
}

/// `ExportProgress`* then at most one `ExportText` then exactly one `ExportDone`, last.
pub(super) fn assert_event_order(events: &[CompareEvent]) {
    let (last, rest) = events.split_last().unwrap();
    assert!(matches!(last, CompareEvent::ExportDone { .. }), "{last:?}");
    let mut seen_text = false;
    let mut done = 0;
    for event in rest {
        match event {
            CompareEvent::ExportProgress { done: d, total, .. } => {
                assert!(!seen_text && d <= total);
                done = done.max(*d);
            }
            CompareEvent::ExportText { .. } => {
                assert!(!seen_text, "one ExportText");
                seen_text = true;
            }
            other => panic!("{other:?}"),
        }
    }
    let _ = done;
}

#[tokio::test]
async fn file_reports_emit_progress_then_one_done_and_no_text() {
    let mut r = report_rig().await;
    let (target, _) = file_target(&r, "events.csv");
    let events = report(&mut r, 5, report_request(vec![1, 2], true), target).await;
    assert_event_order(&events);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, CompareEvent::ExportText { .. }))
    );
    let totals: Vec<(u64, u64)> = events
        .iter()
        .filter_map(|e| match e {
            CompareEvent::ExportProgress { done, total, .. } => Some((*done, *total)),
            _ => None,
        })
        .collect();
    assert!(totals.iter().all(|(_, t)| *t == 11));
    assert_eq!(totals.last(), Some(&(11, 11)));
    assert!(totals.windows(2).all(|w| w[0].0 <= w[1].0));
}

#[tokio::test]
async fn no_connection_name_uri_or_host_reaches_the_file_or_the_text() {
    let mut r = report_rig().await;
    let (file, _) = file_report(&mut r, 5, vec![1, 2, 3], true).await;
    let events = report(
        &mut r,
        6,
        report_request(vec![1, 2, 3], true),
        ReportTarget::Clipboard,
    )
    .await;
    for text in [file, text_of(&events)] {
        for secret in [
            SRC_NAME,
            TGT_NAME,
            SRC_URI,
            TGT_URI,
            "src-host",
            "tgt-host",
            "srcuser",
            "tgtuser",
            "mongodb://",
        ] {
            assert!(!text.contains(secret), "{secret} leaked");
        }
    }
}

#[tokio::test]
async fn a_hostile_label_cannot_add_lines_or_cells() {
    let mut r = report_rig().await;
    let request = ReportRequest {
        pairs: vec![3],
        include_paths: false,
        label: "x,=HYPERLINK(\"http://e\")\r\n=cmd|' /C calc'!A0\u{2028};y".into(),
    };
    let (target, path) = file_target(&r, "label.csv");
    let events = report(&mut r, 5, request, target).await;
    outcome_of(&events);
    let text = std::fs::read_to_string(path).unwrap();
    let lines = comment_lines(&text);
    assert_eq!(lines.len(), 5, "{lines:?}");
    assert!(lines[1].starts_with("# label: x,=HYPERLINK("));
    assert!(!lines[1].contains(['\r', '\n', '\u{2028}']));
}

#[tokio::test]
async fn refusals_leave_no_file_and_free_the_slot() {
    let mut r = report_rig().await;
    let (target, path) = file_target(&r, "never.csv");
    let cases: Vec<(Vec<PairId>, &str)> = vec![
        (vec![], "nothing to export"),
        (vec![1, 99], "no results for this pair"),
        (vec![99], "no results for this pair"),
        (vec![1, 1], "duplicate pair"),
    ];
    for (run, (pairs, message)) in cases.into_iter().enumerate() {
        let events = report(
            &mut r,
            run as u64 + 1,
            report_request(pairs, false),
            target.clone(),
        )
        .await;
        assert_eq!(failure_of(&events), CoreError::Unsupported(message.into()));
        assert!(!path.exists());
    }
    // A discarded pair has no results either.
    r.tx.send(Command::Compare(CompareCommand::Discard {
        session: 1,
        pair: 2,
    }))
    .unwrap();
    super::apply_tests::barrier(&r.tx, &mut r.h).await;
    let events = report(&mut r, 9, report_request(vec![2], false), target.clone()).await;
    assert_eq!(
        failure_of(&events),
        CoreError::Unsupported("no results for this pair".into())
    );
    let events = report(&mut r, 10, report_request(vec![1], false), target).await;
    assert_eq!(outcome_of(&events).written, 8, "the slot was free");
}

#[tokio::test]
async fn an_invalid_path_is_refused_up_front() {
    let mut r = report_rig().await;
    for bad in [
        Path::new("relative.csv").to_path_buf(),
        r.dir.path().to_path_buf(),
        r.dir.path().join("missing").join("x.csv"),
    ] {
        let events = report(
            &mut r,
            5,
            report_request(vec![1], false),
            ReportTarget::File(bad),
        )
        .await;
        assert_eq!(
            failure_of(&events),
            CoreError::Unsupported("invalid export path".into())
        );
    }
    let (target, _) = file_target(&r, "ok.csv");
    let events = report(&mut r, 6, report_request(vec![1], false), target).await;
    assert_eq!(outcome_of(&events).written, 8);
}

fn big_source(rows: i32) -> InMemoryStore {
    let docs = (0..rows).map(|n| doc! { "_id": n }).collect();
    InMemoryStore::new().with_docs(&ns("d", "big"), docs)
}

async fn big_rig(rows: i32) -> ExRig {
    rig_with(
        ApplyStore::new(big_source(rows)),
        ApplyStore::new(InMemoryStore::new()),
        vec![pair(1, ns("d", "big"), ns("d", "big"))],
    )
    .await
}

#[tokio::test]
async fn a_clipboard_report_over_the_cap_is_refused_before_the_slot_is_taken() {
    let mut r = big_rig(10_001).await;
    let events = report(
        &mut r,
        5,
        report_request(vec![1], false),
        ReportTarget::Clipboard,
    )
    .await;
    assert_eq!(
        failure_of(&events),
        CoreError::Unsupported("report too large for the clipboard".into())
    );
    // The slot was never taken: a file report of the same pair runs at once.
    let (text, outcome) = file_report(&mut r, 6, vec![1], false).await;
    assert_eq!(outcome.written, 10_001);
    assert_eq!(data_rows(&text).len(), 10_001);
}

#[tokio::test]
async fn a_clipboard_report_of_exactly_the_cap_is_accepted() {
    let mut r = big_rig(10_000).await;
    let events = report(
        &mut r,
        5,
        report_request(vec![1], false),
        ReportTarget::Clipboard,
    )
    .await;
    assert_eq!(data_rows(&text_of(&events)).len(), 10_000);
    assert_eq!(outcome_of(&events).written, 10_000);
    assert_event_order(&events);
}
