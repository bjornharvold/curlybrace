use bson::Bson;

use super::*;
use crate::compare::diff::PathSegment::{Field, Index};

fn summary(
    equal: u64,
    only_in_source: u64,
    only_in_target: u64,
    different: u64,
    scanned_source: u64,
    scanned_target: u64,
) -> Summary {
    Summary {
        equal,
        only_in_source,
        only_in_target,
        different,
        scanned_source,
        scanned_target,
    }
}

fn header() -> ReportHeader {
    ReportHeader {
        label: "x\n=cmd".to_string(),
        pairs: vec![
            PairSummary {
                source: "shop.items".into(),
                target: "shop2.items".into(),
                summary: summary(4, 1, 2, 3, 5, 6),
            },
            PairSummary {
                source: "=evil.c".into(),
                target: "a\nb".into(),
                summary: summary(10, 0, 0, 2, 12, 12),
            },
        ],
    }
}

fn key(v: Bson) -> KeyTuple {
    KeyTuple(vec![v])
}

fn report() -> String {
    let mut w = CsvWriter::new(Vec::new());
    w.begin(&header()).expect("begin");
    let none: &[Vec<crate::compare::diff::PathSegment>] = &[];
    w.row(&CsvRow {
        pair: 1,
        source: "shop.items",
        target: "shop2.items",
        category: Category::OnlyInSource,
        key: &key(Bson::Int32(1)),
        changed_paths: none,
    })
    .expect("row 1");
    w.row(&CsvRow {
        pair: 1,
        source: "shop.items",
        target: "shop2.items",
        category: Category::Different,
        key: &key(Bson::String("a,b".into())),
        changed_paths: &[
            vec![Field("a".into()), Field("b".into())],
            vec![Field("c".into()), Index(2)],
        ],
    })
    .expect("row 2");
    w.row(&CsvRow {
        pair: 2,
        source: "=evil.c",
        target: "a\nb",
        category: Category::Different,
        key: &key(Bson::String("\u{fc}n\u{ef}".into())),
        changed_paths: &[vec![Field("\u{e9},x".into())]],
    })
    .expect("row 3");
    w.row(&CsvRow {
        pair: 2,
        source: "=evil.c",
        target: "t",
        category: Category::OnlyInTarget,
        key: &key(Bson::String("=1+1\r\n@x".into())),
        changed_paths: none,
    })
    .expect("row 4");
    String::from_utf8(w.finish().expect("finish")).expect("utf8")
}

#[test]
fn golden_report() {
    let expected = "\
# curlybrace data compare report\r
# label: x =cmd\r
# pairs: 2\r
# pair 1: shop.items -> shop2.items: only_in_source=1, only_in_target=2, different=3, identical=4, scanned_source=5, scanned_target=6\r
# pair 2: =evil.c -> a b: only_in_source=0, only_in_target=0, different=2, identical=10, scanned_source=12, scanned_target=12\r
# total: only_in_source=1, only_in_target=2, different=5, identical=14, scanned_source=17, scanned_target=18\r
pair,source,target,category,key,changed_paths\r
1,shop.items,shop2.items,only_in_source,[1],\r
1,shop.items,shop2.items,different,\"[\"\"a,b\"\"]\",\"[\"\"a.b\"\",\"\"c[2]\"\"]\"\r
2,'=evil.c,\"a\nb\",different,\"[\"\"\u{fc}n\u{ef}\"\"]\",\"[\"\"\u{e9},x\"\"]\"\r
2,'=evil.c,t,only_in_target,\"[\"\"=1+1\\r\\n@x\"\"]\",\r
";
    assert_eq!(report(), expected);
}

#[test]
fn output_is_deterministic() {
    assert_eq!(report(), report());
}

#[test]
fn the_summary_has_no_durations() {
    let text = report().to_lowercase();
    for word in ["duration", "elapsed", "seconds", "millis", " ms"] {
        assert!(!text.contains(word), "{word}");
    }
}

#[test]
fn comment_lines_come_first_then_the_header_row() {
    let text = report();
    let lines: Vec<&str> = text.split("\r\n").collect();
    let header_at = lines
        .iter()
        .position(|l| *l == "pair,source,target,category,key,changed_paths")
        .expect("header row");
    assert!(lines[..header_at].iter().all(|l| l.starts_with('#')));
    assert!(lines[header_at + 1..].iter().all(|l| !l.starts_with('#')));
}

#[test]
fn a_report_without_rows_still_has_summary_and_header() {
    let mut w = CsvWriter::new(Vec::new());
    w.begin(&ReportHeader {
        label: String::new(),
        pairs: vec![],
    })
    .expect("begin");
    let text = String::from_utf8(w.finish().expect("finish")).expect("utf8");
    assert!(text.starts_with("# curlybrace data compare report\r\n"));
    assert!(text.contains("# pairs: 0\r\n"));
    assert!(text.contains("# total: only_in_source=0, only_in_target=0, different=0"));
    assert!(text.ends_with("pair,source,target,category,key,changed_paths\r\n"));
}

#[test]
fn a_long_hostile_label_is_one_capped_comment_line() {
    let mut w = CsvWriter::new(Vec::new());
    let label = format!("{}\r\npair,source\r\n=x", "y".repeat(10_000));
    w.begin(&ReportHeader {
        label,
        pairs: vec![],
    })
    .expect("begin");
    let text = String::from_utf8(w.finish().expect("finish")).expect("utf8");
    assert!(text.lines().take(3).all(|l| l.starts_with('#')));
    assert!(text.len() < 1_000);
}

#[test]
fn calling_the_writer_out_of_order_is_a_sequence_error() {
    let none: &[Vec<crate::compare::diff::PathSegment>] = &[];
    let k = key(Bson::Int32(1));
    let row = CsvRow {
        pair: 1,
        source: "a",
        target: "b",
        category: Category::Different,
        key: &k,
        changed_paths: none,
    };
    let mut w = CsvWriter::new(Vec::new());
    assert_eq!(w.row(&row), Err(RenderError::Sequence));
    w.begin(&header()).expect("begin");
    assert_eq!(w.begin(&header()), Err(RenderError::Sequence));
    assert!(matches!(
        CsvWriter::new(Vec::new()).finish(),
        Err(RenderError::Sequence)
    ));
}

#[test]
fn a_nan_key_is_still_a_row() {
    let mut w = CsvWriter::new(Vec::new());
    w.begin(&header()).expect("begin");
    let k = key(Bson::Double(f64::NAN));
    let none: &[Vec<crate::compare::diff::PathSegment>] = &[];
    w.row(&CsvRow {
        pair: 1,
        source: "a",
        target: "b",
        category: Category::Different,
        key: &k,
        changed_paths: none,
    })
    .expect("row");
    let text = String::from_utf8(w.finish().expect("finish")).expect("utf8");
    assert!(text.contains(",different,"));
}
