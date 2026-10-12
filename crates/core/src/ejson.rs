//! Extended-JSON conversion for the document viewer and editor.
//!
//! The read-only view uses RELAXED extended JSON (readable numbers); the
//! editor uses CANONICAL extended JSON so every BSON type survives a
//! round-trip (`Int32` stays `Int32`, `Int64` stays `Int64`, NaN, dates, ...).

use bson::{Bson, Document};
use serde_json::Value;

use crate::error::{CoreError, Result};

fn pretty(value: &Value) -> String {
    // Infallible by construction: a `Value` has only string keys and finite
    // numbers, so serialization cannot fail. Never fall back to a placeholder
    // like "{}": an editor showing it could overwrite a real document.
    serde_json::to_string_pretty(value).expect("serde_json::Value always serializes")
}

/// Human-readable relaxed extended JSON (pretty) for the read-only view.
pub fn doc_to_view_json(doc: &Document) -> String {
    pretty(&Bson::Document(doc.clone()).into_relaxed_extjson())
}

/// Type-preserving canonical extended JSON (pretty) for the editor.
pub fn doc_to_editor_json(doc: &Document) -> String {
    pretty(&Bson::Document(doc.clone()).into_canonical_extjson())
}

/// Parses editor text back into a document.
///
/// Error choice: text that is not valid JSON (including empty input, trailing
/// garbage, or nesting beyond serde_json's recursion limit) is
/// `CoreError::InvalidJson`; valid JSON that is not a document (`[]`, scalars)
/// or holds malformed extended-JSON wrappers is `CoreError::InvalidDocument`.
pub fn editor_json_to_doc(text: &str) -> Result<Document> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| CoreError::InvalidJson(e.to_string()))?;
    if !value.is_object() {
        return Err(CoreError::InvalidDocument(
            "expected a JSON object at the top level".into(),
        ));
    }
    match Bson::try_from(value).map_err(|e| CoreError::InvalidDocument(e.to_string()))? {
        Bson::Document(doc) => Ok(doc),
        other => Err(CoreError::InvalidDocument(format!(
            "expected a document, found {}",
            bson_type_name(&other)
        ))),
    }
}

/// Short type label shown next to a value in the tree view.
pub fn bson_type_name(b: &Bson) -> &'static str {
    match b {
        Bson::String(_) => "string",
        Bson::Int32(_) => "int32",
        Bson::Int64(_) => "int64",
        Bson::Double(_) => "double",
        Bson::Decimal128(_) => "decimal128",
        Bson::ObjectId(_) => "objectId",
        Bson::DateTime(_) => "date",
        Bson::Binary(_) => "binary",
        Bson::Boolean(_) => "bool",
        Bson::Null => "null",
        Bson::Document(_) => "object",
        Bson::Array(_) => "array",
        Bson::RegularExpression(_) => "regex",
        Bson::Timestamp(_) => "timestamp",
        Bson::MinKey => "minKey",
        Bson::MaxKey => "maxKey",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mock_objects::exotic_doc;
    use bson::spec::BinarySubtype;
    use bson::{Binary, DateTime, doc};

    fn roundtrip(d: &Document) -> Document {
        editor_json_to_doc(&doc_to_editor_json(d)).expect("round-trips")
    }

    #[test]
    fn exotic_doc_round_trips_every_field() {
        let original = exotic_doc();
        let back = roundtrip(&original);
        assert_eq!(back.len(), original.len());
        for (key, want) in &original {
            let got = back.get(key).unwrap_or_else(|| panic!("missing {key}"));
            match want {
                Bson::Double(d) if d.is_nan() => {
                    assert!(matches!(got, Bson::Double(g) if g.is_nan()), "{key}")
                }
                _ => assert_eq!(got, want, "{key}"),
            }
        }
        assert_eq!(
            back.keys().collect::<Vec<_>>(),
            original.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn nested_key_order_survives_round_trip() {
        let d = doc! { "z": 1_i32, "a": doc! { "y": 1_i32, "b": 2_i32 } };
        let back = roundtrip(&d);
        assert_eq!(back.keys().collect::<Vec<_>>(), ["z", "a"]);
        let inner = back.get_document("a").unwrap();
        assert_eq!(inner.keys().collect::<Vec<_>>(), ["y", "b"]);
    }

    #[test]
    fn int_widths_are_preserved() {
        let back = roundtrip(&doc! { "a": 1_i32, "b": 1_i64 });
        assert_eq!(back.get("a"), Some(&Bson::Int32(1)));
        assert_eq!(back.get("b"), Some(&Bson::Int64(1)));
    }

    #[test]
    fn uuid_binary_subtype_round_trips() {
        let d = doc! { "u": Binary { subtype: BinarySubtype::Uuid, bytes: (0..16).collect() } };
        assert_eq!(roundtrip(&d), d);
    }

    #[test]
    fn extreme_dates_round_trip() {
        let d =
            doc! { "min": DateTime::MIN, "max": DateTime::MAX, "neg": DateTime::from_millis(-1) };
        assert_eq!(roundtrip(&d), d);
    }

    #[test]
    fn infinities_and_negative_zero_round_trip() {
        let back = roundtrip(&doc! {
            "inf": f64::INFINITY, "ninf": f64::NEG_INFINITY, "nz": -0.0_f64
        });
        assert_eq!(back.get_f64("inf"), Ok(f64::INFINITY));
        assert_eq!(back.get_f64("ninf"), Ok(f64::NEG_INFINITY));
        let nz = back.get_f64("nz").unwrap();
        assert_eq!(nz, 0.0);
        assert!(nz.is_sign_negative());
    }

    #[test]
    fn strings_with_unicode_escapes_and_newlines_round_trip() {
        let d = doc! {
            "s": "héllo \u{1F980} \"q\" \\ \n\t\r \u{0000} end",
            "k\"ey\n": "v",
        };
        assert_eq!(roundtrip(&d), d);
    }

    #[test]
    fn view_json_is_human_readable() {
        let json = doc_to_view_json(&doc! { "n": 5_i64, "d": 1.5_f64 });
        assert!(json.contains("\"n\": 5"), "{json}");
        assert!(!json.contains("$numberLong"), "{json}");
    }

    #[test]
    fn editor_json_is_canonical() {
        let json = doc_to_editor_json(&doc! { "n": 5_i64 });
        assert!(json.contains("$numberLong"), "{json}");
    }

    #[test]
    fn invalid_json_text_is_invalid_json() {
        for text in ["", "   ", "{\"a\":}", "{\"a\": 1} trailing", "{"] {
            assert!(
                matches!(editor_json_to_doc(text), Err(CoreError::InvalidJson(_))),
                "{text:?}"
            );
        }
    }

    #[test]
    fn valid_json_that_is_not_a_document_is_invalid_document() {
        for text in ["[]", "1", "\"s\"", "null", "true", "[{\"a\":1}]"] {
            assert!(
                matches!(editor_json_to_doc(text), Err(CoreError::InvalidDocument(_))),
                "{text:?}"
            );
        }
    }

    #[test]
    fn malformed_extjson_wrapper_is_invalid_document() {
        let r = editor_json_to_doc("{\"a\": {\"$numberInt\": \"nope\"}}");
        assert!(matches!(r, Err(CoreError::InvalidDocument(_))), "{r:?}");
    }

    #[test]
    fn deeply_nested_input_is_a_typed_error_not_a_panic() {
        let depth = 100_000;
        let objects = format!("{}{}", "{\"a\":".repeat(depth), "}".repeat(depth));
        assert!(matches!(
            editor_json_to_doc(&objects),
            Err(CoreError::InvalidJson(_))
        ));
        let arrays = format!("{{\"a\":{}{}}}", "[".repeat(depth), "]".repeat(depth));
        assert!(matches!(
            editor_json_to_doc(&arrays),
            Err(CoreError::InvalidJson(_))
        ));
    }

    #[test]
    fn type_names_cover_exotic_doc() {
        let d = exotic_doc();
        let names: Vec<_> = d
            .iter()
            .map(|(k, v)| (k.as_str(), bson_type_name(v)))
            .collect();
        let expect = [
            ("_id", "objectId"),
            ("int32", "int32"),
            ("int64", "int64"),
            ("double", "double"),
            ("nan", "double"),
            ("decimal", "decimal128"),
            ("binary", "binary"),
            ("date", "date"),
            ("minKey", "minKey"),
            ("maxKey", "maxKey"),
            ("nested", "object"),
            ("array", "array"),
            ("regex", "regex"),
            ("oid", "objectId"),
            ("timestamp", "timestamp"),
        ];
        assert_eq!(names, expect);
    }

    #[test]
    fn type_names_for_scalars_and_others() {
        assert_eq!(bson_type_name(&Bson::String("x".into())), "string");
        assert_eq!(bson_type_name(&Bson::Boolean(true)), "bool");
        assert_eq!(bson_type_name(&Bson::Null), "null");
        assert_eq!(bson_type_name(&Bson::Symbol("s".into())), "other");
        assert_eq!(bson_type_name(&Bson::Undefined), "other");
        assert_eq!(bson_type_name(&Bson::JavaScriptCode("1".into())), "other");
    }
}
