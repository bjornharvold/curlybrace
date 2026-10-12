//! Parsing of query-bar text (extended JSON) into a typed [`Query`].

use bson::{Bson, Document};

use crate::error::{CoreError, Result};
use crate::store::{PAGE_SIZE, Query};

const MAX_LIMIT: u64 = 1000;

/// Raw text of the query bar fields, as typed by the user.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueryText {
    pub filter: String,
    pub sort: String,
    pub projection: String,
    pub skip: String,
    pub limit: String,
}

fn invalid(label: &str, detail: impl std::fmt::Display) -> CoreError {
    CoreError::InvalidJson(format!("{label}: {detail}"))
}

fn parse_doc(label: &str, text: &str) -> Result<Option<Document>> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    // serde_json enforces its own recursion limit (128) and returns an error.
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| invalid(label, e))?;
    match Bson::try_from(value).map_err(|e| invalid(label, e))? {
        Bson::Document(d) => Ok(Some(d)),
        _ => Err(invalid(label, "expected an object")),
    }
}

fn parse_u64(label: &str, text: &str) -> Result<Option<u64>> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    t.parse::<u64>().map(Some).map_err(|e| invalid(label, e))
}

/// Parses the query-bar text. Empty fields yield defaults; limit is clamped to 1..=1000.
pub fn parse_query(t: &QueryText) -> Result<Query> {
    Ok(Query {
        filter: parse_doc("filter", &t.filter)?.unwrap_or_default(),
        sort: parse_doc("sort", &t.sort)?,
        projection: parse_doc("projection", &t.projection)?,
        skip: parse_u64("skip", &t.skip)?.unwrap_or(0),
        limit: parse_u64("limit", &t.limit)?
            .unwrap_or(PAGE_SIZE)
            .clamp(1, MAX_LIMIT),
    })
}

/// The query for the following page (`skip += limit`, saturating).
pub fn next_page(q: &Query) -> Query {
    Query {
        skip: q.skip.saturating_add(q.limit),
        ..q.clone()
    }
}

/// The query for the preceding page; saturates at 0 and does not realign to page boundaries.
pub fn prev_page(q: &Query) -> Query {
    Query {
        skip: q.skip.saturating_sub(q.limit),
        ..q.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::{doc, oid::ObjectId};

    fn with_filter(f: &str) -> QueryText {
        QueryText {
            filter: f.into(),
            ..Default::default()
        }
    }

    fn assert_invalid(t: &QueryText) {
        assert!(
            matches!(parse_query(t), Err(CoreError::InvalidJson(_))),
            "{t:?}"
        );
    }

    #[test]
    fn empty_text_is_default_query() {
        assert_eq!(
            parse_query(&QueryText::default()).unwrap(),
            Query::default()
        );
    }

    #[test]
    fn parses_plain_filter() {
        let q = parse_query(&with_filter(r#"{"a": 1}"#)).unwrap();
        assert_eq!(q.filter, doc! {"a": 1});
    }

    #[test]
    fn parses_extended_json_oid_and_date() {
        let q = parse_query(&with_filter(
            r#"{"_id": {"$oid": "507f1f77bcf86cd799439011"}, "d": {"$date": "2020-01-01T00:00:00Z"}}"#,
        ))
        .unwrap();
        let oid = ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap();
        assert_eq!(q.filter.get("_id"), Some(&Bson::ObjectId(oid)));
        let expected = bson::DateTime::parse_rfc3339_str("2020-01-01T00:00:00Z").unwrap();
        assert_eq!(q.filter.get("d"), Some(&Bson::DateTime(expected)));
        assert_eq!(expected.timestamp_millis(), 1_577_836_800_000);
    }

    #[test]
    fn parses_sort_projection_skip_limit() {
        let q = parse_query(&QueryText {
            filter: String::new(),
            sort: r#"{"a": -1}"#.into(),
            projection: r#"{"a": 1}"#.into(),
            skip: " 7 ".into(),
            limit: "20".into(),
        })
        .unwrap();
        assert_eq!(q.sort, Some(doc! {"a": -1}));
        assert_eq!(q.projection, Some(doc! {"a": 1}));
        assert_eq!((q.skip, q.limit), (7, 20));
    }

    #[test]
    fn malformed_filters_are_typed_errors() {
        for bad in [r#"{"a":1,}"#, "[1,2]", r#""str""#, "{", r#"{"a": }"#] {
            assert_invalid(&with_filter(bad));
        }
    }

    #[test]
    fn error_names_the_field() {
        let e = parse_query(&QueryText {
            sort: "[1]".into(),
            ..Default::default()
        })
        .unwrap_err();
        assert!(e.to_string().contains("sort"), "{e}");
    }

    #[test]
    fn absurdly_deep_nesting_is_an_error_not_a_panic() {
        assert_invalid(&with_filter(&"[".repeat(10_000)));
    }

    /// Deepest accepted container nesting (serde_json allows 127 open containers).
    const MAX_DEPTH: usize = 127;

    fn nested_objects(depth: usize) -> String {
        format!("{}1{}", r#"{"a":"#.repeat(depth), "}".repeat(depth))
    }

    // Pins serde_json's recursion limit (the filter's own
    // outer object counts). Fails loudly if `unbounded_depth` is ever
    // enabled by a dependency. Runs on a normal test thread (no big stack).
    #[test]
    fn nesting_limit_is_pinned() {
        let q = parse_query(&with_filter(&nested_objects(MAX_DEPTH))).unwrap();
        assert!(q.filter.get_document("a").is_ok());
        assert_invalid(&with_filter(&nested_objects(MAX_DEPTH + 1)));
    }

    #[test]
    fn mixed_object_array_nesting_parses() {
        let text = format!("{}1{}", r#"{"a":[{"b":"#.repeat(30), "}]}".repeat(30));
        let q = parse_query(&with_filter(&text)).unwrap();
        assert!(q.filter.get_array("a").is_ok());
    }

    #[test]
    fn invalid_oid_is_an_error() {
        assert_invalid(&with_filter(r#"{"$oid":"bad"}"#));
        assert_invalid(&with_filter(r#"{"_id": {"$oid":"bad"}}"#));
    }

    #[test]
    fn whitespace_only_text_is_default() {
        let ws = || " \t\n".to_string();
        let q = parse_query(&QueryText {
            filter: ws(),
            sort: ws(),
            projection: ws(),
            skip: ws(),
            limit: ws(),
        })
        .unwrap();
        assert_eq!(q, Query::default());
    }

    #[test]
    fn non_object_sort_and_projection_are_errors() {
        assert_invalid(&QueryText {
            projection: "[1]".into(),
            ..Default::default()
        });
        assert_invalid(&QueryText {
            sort: "5".into(),
            ..Default::default()
        });
    }

    #[test]
    fn skip_edge_cases() {
        let skip = |s: &str| {
            parse_query(&QueryText {
                skip: s.into(),
                ..Default::default()
            })
        };
        // Current behaviour: Rust's u64 parser accepts a leading '+'.
        assert_eq!(skip("+5").unwrap().skip, 5);
        assert!(matches!(
            skip("18446744073709551616"),
            Err(CoreError::InvalidJson(_))
        ));
    }

    #[test]
    fn bad_skip_is_an_error() {
        for bad in ["-1", "abc", "1.5"] {
            assert_invalid(&QueryText {
                skip: bad.into(),
                ..Default::default()
            });
        }
    }

    #[test]
    fn bad_limit_is_an_error() {
        for bad in ["x", "-1"] {
            assert_invalid(&QueryText {
                limit: bad.into(),
                ..Default::default()
            });
        }
    }

    #[test]
    fn limit_is_clamped() {
        let lim = |s: &str| {
            parse_query(&QueryText {
                limit: s.into(),
                ..Default::default()
            })
            .unwrap()
            .limit
        };
        assert_eq!(lim("0"), 1);
        assert_eq!(lim("5000"), 1000);
        assert_eq!(lim("1000"), 1000);
    }

    #[test]
    fn paging_never_underflows_or_overflows() {
        let q = Query {
            skip: 10,
            limit: 50,
            ..Query::default()
        };
        assert_eq!(prev_page(&q).skip, 0);
        assert_eq!(next_page(&q).skip, 60);
        assert_eq!(prev_page(&prev_page(&q)).skip, 0);
        let big = Query {
            skip: u64::MAX,
            limit: 5,
            ..Query::default()
        };
        assert_eq!(next_page(&big).skip, u64::MAX);
    }
}
