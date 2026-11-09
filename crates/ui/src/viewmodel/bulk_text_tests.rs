use super::*;
use bson::{Bson, doc};
use curlybrace_core::error::CoreError;

/// What the preview showed before the clipping: the whole document serialized, then cut.
fn unbounded(doc: &Document) -> String {
    let text = Bson::Document(doc.clone())
        .into_relaxed_extjson()
        .to_string();
    if text.chars().count() <= SHOWN_CHARS {
        return text;
    }
    text.chars()
        .take(SHOWN_CHARS)
        .chain(std::iter::once('\u{2026}'))
        .collect()
}

#[test]
fn compact_matches_the_unclipped_rendering_for_every_shape() {
    let big_array: Vec<i32> = (0..5_000).collect();
    let nested = doc! { "a": { "b": { "c": [1, 2, { "d": "x" }] } }, "s": "y".repeat(900) };
    let wide: Document = (0..2_000)
        .map(|n| (format!("k{n}"), Bson::Int32(n)))
        .collect();
    let binary = Bson::Binary(bson::Binary {
        subtype: bson::spec::BinarySubtype::Generic,
        bytes: vec![7; 5_000],
    });
    let docs = [
        doc! {},
        doc! { "_id": 1, "name": "short" },
        doc! { "s": "é".repeat(2_000) },
        doc! { "quote": "\"\\\n".repeat(500) },
        doc! { "arr": big_array },
        nested,
        wide,
        doc! { "bin": binary },
        doc! { "id": bson::oid::ObjectId::new(), "when": bson::DateTime::now(), "n": 1.5 },
    ];
    for doc in docs {
        assert_eq!(compact(&doc), unbounded(&doc), "{:?}", doc.keys().next());
    }
}

#[test]
fn a_five_megabyte_value_is_clipped_before_it_is_serialized() {
    let big_string = "x".repeat(5 * 1024 * 1024);
    let big_array: Vec<Bson> = (0..200_000).map(Bson::Int32).collect();
    let big_binary = vec![1_u8; 5 * 1024 * 1024];
    let doc = doc! {
        "s": big_string,
        "a": big_array,
        "b": Bson::Binary(bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: big_binary,
        }),
    };
    let mut budget = NODE_BUDGET;
    let clipped = clip_document(&doc, &mut budget);
    assert_eq!(clipped.get_str("s").unwrap().chars().count(), SHOWN_CHARS);
    assert!(clipped.get_array("a").unwrap().len() <= SHOWN_CHARS);
    let Some(Bson::Binary(b)) = clipped.get("b") else {
        panic!("binary kept")
    };
    assert!(b.bytes.len() <= SHOWN_CHARS);
    let text = compact(&doc);
    assert_eq!(text.chars().count(), SHOWN_CHARS + 1);
    assert!(text.ends_with('\u{2026}'));
}

#[test]
fn the_node_budget_bounds_a_document_of_many_small_values() {
    let deep: Document = (0..10_000)
        .map(|n| (format!("k{n}"), Bson::Array(vec![Bson::Int32(1); 10_000])))
        .collect();
    let mut budget = NODE_BUDGET;
    let clipped = clip_document(&deep, &mut budget);
    let nodes: usize = clipped
        .values()
        .map(|v| 1 + v.as_array().map_or(0, Vec::len))
        .sum();
    assert!(nodes <= NODE_BUDGET + SHOWN_CHARS, "{nodes}");
}

#[test]
fn update_check_gives_the_verdict_once_for_its_text() {
    let good = UpdateCheck::of(r#"{"$set": {"a": 1}}"#);
    assert!(good.is_valid() && good.problem().is_none());
    assert!(good.is_for(r#"{"$set": {"a": 1}}"#) && !good.is_for("{}"));
    let blank = UpdateCheck::of("   ");
    assert!(!blank.is_valid() && blank.problem().is_none());
    let bad = UpdateCheck::of(r#"{"a": 1}"#);
    assert!(!bad.is_valid());
    assert_eq!(bad.problem(), Some(CoreError::BulkInvalidUpdate.i18n_key()));
    let junk = UpdateCheck::of("nope");
    assert!(!junk.is_valid() && junk.problem().is_some());
}

#[test]
fn doc_text_keeps_before_and_after_and_a_sample_has_no_after() {
    let preview = PreviewDoc {
        before: doc! { "a": 1 },
        after: Some(doc! { "a": 2 }),
    };
    assert_eq!(
        DocText::of_preview(&preview),
        DocText {
            before: r#"{"a":1}"#.into(),
            after: Some(r#"{"a":2}"#.into())
        }
    );
    let none = PreviewDoc {
        before: doc! {},
        after: None,
    };
    assert_eq!(DocText::of_preview(&none).after, None);
    assert_eq!(DocText::of_sample(&doc! { "a": 1 }).after, None);
}
