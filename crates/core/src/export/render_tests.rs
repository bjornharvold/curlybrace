use bson::{Bson, Document, doc};
use serde_json::Value;

use super::protocol::{DocFormat, JsonMode};
use super::render::{DocWriter, doc_line};
use crate::ejson::editor_json_to_doc;
use crate::test_support::mock_objects::{exotic_doc, mock_numbered_docs};

fn written(format: DocFormat, docs: &[Document]) -> String {
    let mut writer = DocWriter::new(Vec::new(), format, JsonMode::Canonical);
    writer.begin().expect("begin");
    for d in docs {
        writer.doc(d).expect("doc");
    }
    String::from_utf8(writer.finish().expect("finish")).expect("utf8")
}

#[test]
fn canonical_round_trip_preserves_every_bson_type() {
    let original = exotic_doc();
    let line = doc_line(&original, JsonMode::Canonical);
    let back = editor_json_to_doc(&line).expect("parses");
    // NaN never equals itself, so it is checked apart from the rest.
    assert!(matches!(back.get("nan"), Some(Bson::Double(n)) if n.is_nan()));
    let without_nan = |d: &Document| {
        let mut copy = d.clone();
        copy.remove("nan");
        copy
    };
    assert_eq!(without_nan(&back), without_nan(&original));
}

#[test]
fn relaxed_is_not_type_exact_but_is_valid_json() {
    let long = doc! { "n": Bson::Int64(5) };
    let short = doc! { "n": Bson::Int32(5) };
    let relaxed_long = doc_line(&long, JsonMode::Relaxed);
    assert_eq!(relaxed_long, doc_line(&short, JsonMode::Relaxed));
    assert!(serde_json::from_str::<Value>(&relaxed_long).is_ok());
    assert_ne!(
        doc_line(&long, JsonMode::Canonical),
        doc_line(&short, JsonMode::Canonical)
    );
}

#[test]
fn a_string_with_line_breaks_stays_on_one_line() {
    let original = doc! { "s": "a\nb\r\u{2028}c\"d" };
    let line = doc_line(&original, JsonMode::Canonical);
    assert!(!line.contains('\n') && !line.contains('\r'));
    assert_eq!(editor_json_to_doc(&line).expect("parses"), original);
}

#[test]
fn ndjson_writes_one_line_per_document_and_nothing_for_none() {
    assert_eq!(written(DocFormat::Ndjson, &[]), "");
    let docs = mock_numbered_docs(3);
    let out = written(DocFormat::Ndjson, &docs);
    assert!(out.ends_with('\n'));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3);
    for (line, d) in lines.iter().zip(&docs) {
        assert_eq!(&editor_json_to_doc(line).expect("parses"), d);
    }
}

#[test]
fn json_array_is_valid_json_after_zero_one_and_many_documents() {
    assert_eq!(written(DocFormat::JsonArray, &[]), "[]\n");
    for count in [1, 3] {
        let docs = mock_numbered_docs(count);
        let out = written(DocFormat::JsonArray, &docs);
        let parsed: Vec<Value> = serde_json::from_slice(out.as_bytes()).expect("valid json");
        assert_eq!(parsed.len(), count as usize);
        for (value, d) in parsed.iter().zip(&docs) {
            assert_eq!(&editor_json_to_doc(&value.to_string()).expect("parses"), d);
        }
    }
}

#[test]
fn default_mode_is_canonical() {
    assert_eq!(JsonMode::default(), JsonMode::Canonical);
    assert_eq!(DocFormat::default(), DocFormat::Ndjson);
}
