use bson::{Bson, doc};

use super::*;
use crate::compare::apply::{ApplyKind, Direction, IdPolicy, SkipReason};
use crate::compare::diff::PathSegment::{Field, Index};
use crate::compare::export::{ExportCounts, OpMeta};
use crate::compare::fingerprint::Fingerprint;
use crate::compare::key::KeyTuple;

const FP1: &str = "01010101010101010101010101010101";
const FP2: &str = "02020202020202020202020202020202";
const HOSTILE_DB: &str = "x\"); db.dropDatabase(); //";

fn meta(direction: Direction, key: Bson, source: Option<u8>, target: Option<u8>) -> OpMeta {
    OpMeta {
        direction,
        key: KeyTuple(vec![key]),
        source: source.map(|b| Fingerprint([b; 16])),
        target: target.map(|b| Fingerprint([b; 16])),
    }
}

fn header(label: &str) -> ExportHeader {
    ExportHeader {
        label: label.to_string(),
        counts: ExportCounts {
            insert: 1,
            replace: 1,
            delete: 2,
            update_fields: 1,
            skipped: 1,
        },
        id_policy: IdPolicy::Keep,
    }
}

fn new_writer(label: &str) -> ScriptWriter<Vec<u8>> {
    let mut w = ScriptWriter::new(Vec::new());
    w.begin(&header(label)).expect("begin");
    w
}

fn text(w: ScriptWriter<Vec<u8>>) -> String {
    String::from_utf8(w.finish().expect("finish")).expect("utf8")
}

fn field(name: &str) -> crate::compare::diff::PathSegment {
    Field(name.to_string())
}

/// A full script: two collections, all four op kinds, a skipped op, hostile label and names.
fn full_script() -> String {
    let mut w = new_writer("evil\n); db.dropDatabase(); //");
    w.collection("shop", "items").expect("collection");
    w.op(
        &meta(Direction::SourceToTarget, Bson::Int32(1), Some(1), None),
        &ShellOp::Insert {
            doc: doc! { "_id": 1, "name": "a" },
        },
    )
    .expect("insert");
    w.op(
        &meta(Direction::SourceToTarget, Bson::Int32(2), Some(1), Some(2)),
        &ShellOp::Replace {
            id: Bson::Int32(2),
            doc: doc! { "_id": 2, "name": "b" },
        },
    )
    .expect("replace");
    w.op(
        &meta(Direction::TargetToSource, Bson::Int32(3), None, Some(2)),
        &ShellOp::Delete { id: Bson::Int32(3) },
    )
    .expect("delete");
    w.op(
        &meta(Direction::SourceToTarget, Bson::Int32(4), Some(1), Some(2)),
        &ShellOp::UpdateFields {
            id: Bson::Int32(4),
            set: vec![
                (vec![field("a"), field("b")], Bson::Double(1.5)),
                (vec![field("c"), Index(0)], Bson::String("x".into())),
            ],
            unset: vec![vec![field("d")]],
        },
    )
    .expect("update");
    w.skipped(
        &meta(Direction::SourceToTarget, Bson::Int32(5), Some(1), Some(2)),
        ApplyKind::Replace,
        SkipReason::Stale,
    )
    .expect("skipped");
    w.end_collection().expect("end");
    w.collection(HOSTILE_DB, "c\nd").expect("collection");
    let key = Bson::String("k\"); db.dropDatabase(); //".into());
    w.op(
        &meta(Direction::TargetToSource, key.clone(), None, Some(2)),
        &ShellOp::Delete { id: key },
    )
    .expect("hostile delete");
    w.end_collection().expect("end");
    text(w)
}

#[test]
fn golden_script() {
    let expected = format!(
        "\
// curlybrace data compare: mongosh script
// label: evil ); db.dropDatabase(); //
// operations: insert 1, replace 1, delete 2, updateFields 1, skipped 1
// id policy: keep
//
// Review this script before you run it: it writes to the databases it names.
// It was written from an earlier comparison, and documents may have changed since.
// Skipped operations are listed as comments and are never written.

// collection: shop.items
db.getSiblingDB(\"shop\").getCollection(\"items\").bulkWrite([
  // sourceToTarget insert source={FP1} target=none key=[NumberInt(1)]
  {{ insertOne: {{ document: {{\"_id\": NumberInt(1), \"name\": \"a\"}} }} }},
  // sourceToTarget replace source={FP1} target={FP2} key=[NumberInt(2)]
  {{ replaceOne: {{ filter: {{\"_id\": NumberInt(2)}}, replacement: {{\"_id\": NumberInt(2), \"name\": \"b\"}} }} }},
  // targetToSource delete source=none target={FP2} key=[NumberInt(3)]
  {{ deleteOne: {{ filter: {{\"_id\": NumberInt(3)}} }} }},
  // sourceToTarget updateFields source={FP1} target={FP2} key=[NumberInt(4)]
  {{ updateOne: {{ filter: {{\"_id\": NumberInt(4)}}, update: {{\"$set\": {{\"a.b\": 1.5, \"c.0\": \"x\"}}, \"$unset\": {{\"d\": \"\"}}}} }} }},
  // skipped (stale) sourceToTarget replace source={FP1} target={FP2} key=[NumberInt(5)]
], {{ ordered: false }});

// collection: x\"); db.dropDatabase(); //.c d
db.getSiblingDB(\"x\\\"); db.dropDatabase(); //\").getCollection(\"c\\nd\").bulkWrite([
  // targetToSource delete source=none target={FP2} key=[\"k\\\"); db.dropDatabase(); //\"]
  {{ deleteOne: {{ filter: {{\"_id\": \"k\\\"); db.dropDatabase(); //\"}} }} }},
], {{ ordered: false }});
"
    );
    assert_eq!(full_script(), expected);
}

#[test]
fn output_is_deterministic() {
    assert_eq!(full_script(), full_script());
}

/// The script with string literals blanked and `//` comment lines removed: the code only.
fn code_only(script: &str) -> String {
    let mut out = String::new();
    for line in script.lines() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        let mut in_string = false;
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            match (in_string, c) {
                (true, '\\') => {
                    chars.next();
                }
                (true, '"') => in_string = false,
                (true, _) => {}
                (false, '"') => in_string = true,
                (false, c) => out.push(c),
            }
        }
        assert!(!in_string, "a string literal runs past its line: {line}");
        out.push('\n');
    }
    out
}

#[test]
fn hostile_text_appears_only_in_literals_and_sanitised_comments() {
    let script = full_script();
    assert!(script.contains("dropDatabase"));
    assert!(!code_only(&script).contains("dropDatabase"));
    for line in script.lines() {
        if line.contains("dropDatabase") {
            let trimmed = line.trim_start();
            assert!(
                trimmed.starts_with("//") || trimmed.starts_with('{') || trimmed.starts_with("db."),
                "{line}"
            );
        }
    }
    assert!(!script.contains('\u{2028}') && !script.contains('\r'));
}

#[test]
fn exactly_one_bulk_write_statement_per_collection_with_unordered_writes() {
    let script = full_script();
    let code = code_only(&script);
    assert_eq!(code.matches("bulkWrite(").count(), 2);
    assert_eq!(code.matches("], { ordered: false });").count(), 2);
    assert_eq!(code.matches(".getSiblingDB(").count(), 2);
}

#[test]
fn no_variables_leak_between_statements() {
    let code = code_only(&full_script());
    for keyword in ["const ", "let ", "var ", "function", "=>", "= "] {
        assert!(!code.contains(keyword), "found {keyword:?}");
    }
    assert_eq!(
        code.matches(';').count(),
        2,
        "exactly one terminated statement each"
    );
}

#[test]
fn a_collection_with_only_skipped_operations_has_no_statement() {
    let mut w = new_writer("l");
    w.collection("d", "c").expect("collection");
    w.skipped(
        &meta(Direction::SourceToTarget, Bson::Int32(1), Some(1), Some(2)),
        ApplyKind::Fields,
        SkipReason::NotPlanned,
    )
    .expect("skipped");
    w.end_collection().expect("end");
    let script = text(w);
    assert!(!script.contains("bulkWrite"));
    assert!(script.contains(&format!(
        "// skipped (notPlanned) sourceToTarget updateFields source={FP1} target={FP2} key=[NumberInt(1)]"
    )));
}

#[test]
fn an_empty_collection_has_no_statement() {
    let mut w = new_writer("l");
    w.collection("d", "c").expect("collection");
    w.end_collection().expect("end");
    let script = text(w);
    assert!(!script.contains("bulkWrite"));
    assert!(script.contains("// collection: d.c"));
}

#[test]
fn empty_and_nul_names_are_bad_names() {
    for (db, name) in [("", "c"), ("d", ""), ("d\0", "c"), ("d", "c\0")] {
        let mut w = new_writer("l");
        assert_eq!(
            w.collection(db, name),
            Err(RenderError::BadName),
            "{db:?} {name:?}"
        );
    }
}

#[test]
fn calling_the_writer_out_of_order_is_a_sequence_error() {
    let m = meta(Direction::SourceToTarget, Bson::Int32(1), None, None);
    let delete = ShellOp::Delete { id: Bson::Int32(1) };

    let mut w = ScriptWriter::new(Vec::new());
    assert_eq!(w.collection("d", "c"), Err(RenderError::Sequence));
    assert_eq!(w.op(&m, &delete), Err(RenderError::Sequence));

    let mut w = new_writer("l");
    assert_eq!(w.begin(&header("again")), Err(RenderError::Sequence));
    assert_eq!(w.op(&m, &delete), Err(RenderError::Sequence));
    assert_eq!(w.end_collection(), Err(RenderError::Sequence));
    assert_eq!(
        w.skipped(&m, ApplyKind::Delete, SkipReason::Stale),
        Err(RenderError::Sequence)
    );
    w.collection("d", "c").expect("collection");
    assert_eq!(w.collection("d", "c"), Err(RenderError::Sequence));
}

#[test]
fn finishing_inside_a_collection_is_refused_so_a_statement_is_never_left_open() {
    let mut w = new_writer("l");
    w.collection("d", "c").expect("collection");
    w.op(
        &meta(Direction::SourceToTarget, Bson::Int32(1), None, Some(2)),
        &ShellOp::Delete { id: Bson::Int32(1) },
    )
    .expect("op");
    assert!(matches!(w.finish(), Err(RenderError::Sequence)));
}

#[test]
fn finishing_before_begin_is_refused() {
    let w = ScriptWriter::new(Vec::new());
    assert!(matches!(w.finish(), Err(RenderError::Sequence)));
}

#[test]
fn a_failing_operation_writes_nothing() {
    let mut w = new_writer("l");
    w.collection("d", "c").expect("collection");
    let before = w.inner.len();
    let bad = ShellOp::Insert {
        doc: doc! { "a": Bson::Undefined },
    };
    assert_eq!(
        w.op(
            &meta(Direction::SourceToTarget, Bson::Int32(1), Some(1), None),
            &bad
        ),
        Err(RenderError::Unrenderable)
    );
    assert_eq!(
        w.inner.len(),
        before,
        "nothing, not even the statement opening"
    );
}

fn update(
    set: Vec<(Vec<crate::compare::diff::PathSegment>, Bson)>,
    unset: Vec<Vec<crate::compare::diff::PathSegment>>,
) -> Result<String, RenderError> {
    let mut w = new_writer("l");
    w.collection("d", "c")?;
    w.op(
        &meta(Direction::SourceToTarget, Bson::Int32(1), Some(1), Some(2)),
        &ShellOp::UpdateFields {
            id: Bson::Int32(1),
            set,
            unset,
        },
    )?;
    w.end_collection()?;
    Ok(String::from_utf8(w.finish()?).expect("utf8"))
}

#[test]
fn update_omits_an_empty_set_or_an_empty_unset() {
    let only_set = update(vec![(vec![field("a")], Bson::Int32(1))], vec![]).expect("renders");
    assert!(only_set.contains("update: {\"$set\": {\"a\": NumberInt(1)}} }"));
    assert!(!only_set.contains("$unset"));
    let only_unset = update(vec![], vec![vec![field("a")]]).expect("renders");
    assert!(only_unset.contains("update: {\"$unset\": {\"a\": \"\"}} }"));
    assert!(!only_unset.contains("$set"));
}

#[test]
fn update_with_nothing_to_do_or_unaddressable_paths_is_refused() {
    assert_eq!(update(vec![], vec![]), Err(RenderError::Unrenderable));
    assert_eq!(
        update(vec![(vec![field("_id")], Bson::Int32(1))], vec![]),
        Err(RenderError::BadName)
    );
    assert_eq!(
        update(vec![(vec![field("$a")], Bson::Int32(1))], vec![]),
        Err(RenderError::BadName)
    );
    assert_eq!(
        update(vec![], vec![vec![field("a"), field("b.c")]]),
        Err(RenderError::BadName)
    );
}

#[test]
fn an_unset_through_an_array_is_refused_like_the_store_does() {
    assert_eq!(
        update(vec![], vec![vec![field("a"), Index(1)]]),
        Err(RenderError::Unrenderable)
    );
}

#[test]
fn update_paths_are_string_keys_even_when_hostile() {
    let script = update(
        vec![(vec![field("a\"}); dropIt(); ({\"")], Bson::Int32(1))],
        vec![],
    )
    .expect("renders");
    assert!(script.contains("dropIt"));
    assert!(!code_only(&script).contains("dropIt"));
}

#[test]
fn an_unrenderable_key_does_not_stop_the_operation() {
    let mut w = new_writer("l");
    w.collection("d", "c").expect("collection");
    w.op(
        &meta(Direction::SourceToTarget, Bson::Undefined, None, Some(2)),
        &ShellOp::Delete { id: Bson::Int32(1) },
    )
    .expect("op");
    w.end_collection().expect("end");
    let script = text(w);
    assert!(script.contains(&format!("target={FP2} key=<unrenderable>")));
    assert!(script.contains("deleteOne"));
}

#[test]
fn a_hostile_label_stays_in_one_comment_line() {
    let w = new_writer("a\r\nb\u{2028}c\u{0}d");
    let script = text(w);
    let label = script
        .lines()
        .find(|l| l.starts_with("// label:"))
        .expect("label line");
    assert_eq!(label, "// label: a b c d");
    assert!(script.lines().all(|l| l.is_empty() || l.starts_with("//")));
}

#[test]
fn a_very_long_label_is_capped() {
    let script = text(new_writer(&"x".repeat(100_000)));
    assert!(script.len() < 2_000);
}

#[test]
fn an_empty_script_with_no_collections_is_only_comments() {
    let script = text(new_writer(""));
    assert!(script.lines().all(|l| l.is_empty() || l.starts_with("//")));
    assert!(script.ends_with('\n'));
}

#[test]
fn id_policy_is_named_in_the_header() {
    let mut w = ScriptWriter::new(Vec::new());
    let mut h = header("l");
    h.id_policy = IdPolicy::Generate;
    w.begin(&h).expect("begin");
    assert!(text(w).contains("// id policy: generate"));
}
