//! `WriteOp::UpdateFields` for [`crate::mongo_store::MongoStore`]: one `update` command per
//! chunk, one statement per document
//! `{ q: { _id: { $eq: id }, <guards> }, u: { $set: {..}, $unset: {..} }, multi: false }`.
//!
//! Paths are rendered as dotted strings; whatever cannot be rendered faithfully is refused
//! per op BEFORE any command is sent, so the other ops of the call still run:
//! * `Rejected(66)`: the path starts with `_id` (immutable; same code as the in-memory store),
//! * `Rejected(2)`: empty path, a first segment that is not a field, an `AnyElement`, a field
//!   name that is empty, contains `.` or starts with `$`, or an op with neither set nor unset,
//! * `Rejected(40)`: paths that conflict (equal or one is a prefix of another, checked on the
//!   segments and again on the rendered strings, where `Field("1")` and `Index(1)` coincide).
//!
//! Guards: a dotted path cannot tell an array index from a field called `"0"`, so on a document
//! where an array was expected the server would create a field named `"0"`. Every path with an
//! `Index` adds `"<prefix up to the array>": { $type: "array" }` to `q` (one clause per distinct
//! prefix), and a numeric `Field` name adds `{ $type: "object" }` for its parent. If a guard
//! makes the update match nothing, the op is reported `Missing`: that conflates "no such
//! document" with "the array vanished or changed shape"; both mean the document is no longer as
//! the plan expected. Not guarded: an index beyond the end of an array (the server would pad
//! with nulls); the planner only plans indexes that exist on both sides.
//!
//! Missing detection is the same as for replaces (reply `n` short, then a Primary-read probe),
//! except that the probe uses each op's full `q`, guards included. No server text is copied.

use std::collections::BTreeMap;

use bson::{Bson, Document, doc};

use crate::compare::apply_fields::paths_conflict;
use crate::compare::diff::PathSegment;
use crate::error::Result;
use crate::mongo_write::{
    Failures, MAX_BYTES_PER_COMMAND, MAX_OPS_PER_COMMAND, Writer, element_size, id_query,
    map_write_errors, matched_count, missing_among, split_chunks,
};
use crate::store::WriteFailure;

const IMMUTABLE_FIELD: i32 = 66;
const BAD_VALUE: i32 = 2;
const CONFLICTING_UPDATES: i32 = 40;

fn rejected(code: i32) -> WriteFailure {
    WriteFailure::Rejected(code)
}

/// One `UpdateFields` op with its position in the caller's list.
pub(crate) struct FieldUpdate {
    pub(crate) pos: usize,
    pub(crate) id: Bson,
    pub(crate) set: Vec<(Vec<PathSegment>, Bson)>,
    pub(crate) unset: Vec<Vec<PathSegment>>,
}

/// An update statement and the query it uses (the probe re-runs the query).
#[derive(Debug, PartialEq)]
pub(crate) struct FieldStatement {
    pub(crate) statement: Document,
    pub(crate) query: Document,
}

/// The dotted form of `path`; `Index` segments are decimal numbers.
pub(crate) fn render_path(path: &[PathSegment]) -> std::result::Result<String, WriteFailure> {
    match path.first() {
        None => return Err(rejected(BAD_VALUE)),
        Some(PathSegment::Field(name)) if name == "_id" => return Err(rejected(IMMUTABLE_FIELD)),
        Some(PathSegment::Field(_)) => {}
        Some(_) => return Err(rejected(BAD_VALUE)),
    }
    let parts: std::result::Result<Vec<String>, WriteFailure> = path
        .iter()
        .map(|segment| match segment {
            PathSegment::Field(n) if n.is_empty() || n.contains('.') || n.starts_with('$') => {
                Err(rejected(BAD_VALUE))
            }
            PathSegment::Field(n) => Ok(n.clone()),
            PathSegment::Index(i) => Ok(i.to_string()),
            PathSegment::AnyElement => Err(rejected(BAD_VALUE)),
        })
        .collect();
    Ok(parts?.join("."))
}

/// `(prefix, required type)` for every place where the dotted form is ambiguous.
fn guards_for(path: &[PathSegment]) -> Vec<(String, &'static str)> {
    (1..path.len())
        .filter_map(|i| {
            let wanted = match &path[i] {
                PathSegment::Index(_) => "array",
                PathSegment::Field(n) if n.bytes().all(|b| b.is_ascii_digit()) => "object",
                _ => return None,
            };
            render_path(&path[..i]).ok().map(|prefix| (prefix, wanted))
        })
        .collect()
}

/// The query: the `$eq` id plus one guard clause per distinct prefix. Two paths wanting
/// different types at one prefix cannot both hold: refused as a conflict.
fn build_query(id: &Bson, paths: &[&[PathSegment]]) -> std::result::Result<Document, WriteFailure> {
    let mut wanted: BTreeMap<String, &'static str> = BTreeMap::new();
    for (prefix, kind) in paths.iter().flat_map(|p| guards_for(p)) {
        if wanted.insert(prefix, kind).is_some_and(|old| old != kind) {
            return Err(rejected(CONFLICTING_UPDATES));
        }
    }
    let mut query = id_query(id);
    for (prefix, kind) in wanted {
        query.insert(prefix, doc! { "$type": kind });
    }
    Ok(query)
}

/// Equal, or one a dotted-prefix of the other (`a.1` and `a.1.b`).
fn dotted_conflict(keys: &[String]) -> bool {
    keys.iter().enumerate().any(|(n, a)| {
        keys[n + 1..]
            .iter()
            .any(|b| a == b || a.starts_with(&format!("{b}.")) || b.starts_with(&format!("{a}.")))
    })
}

/// Builds the statement for one op, or the typed reason it must not be sent.
pub(crate) fn build_statement(
    id: &Bson,
    set: &[(Vec<PathSegment>, Bson)],
    unset: &[Vec<PathSegment>],
) -> std::result::Result<FieldStatement, WriteFailure> {
    if set.is_empty() && unset.is_empty() {
        return Err(rejected(BAD_VALUE));
    }
    let set_keys = set.iter().map(|(p, _)| render_path(p));
    let set_keys: Vec<String> = set_keys.collect::<std::result::Result<_, _>>()?;
    let unset_keys: Vec<String> = unset
        .iter()
        .map(|p| render_path(p))
        .collect::<std::result::Result<_, _>>()?;
    let segments: Vec<&[PathSegment]> = set
        .iter()
        .map(|(p, _)| p.as_slice())
        .chain(unset.iter().map(Vec::as_slice))
        .collect();
    let owned: Vec<Vec<PathSegment>> = segments.iter().map(|p| p.to_vec()).collect();
    let all_keys: Vec<String> = set_keys.iter().chain(&unset_keys).cloned().collect();
    if paths_conflict(&owned) || dotted_conflict(&all_keys) {
        return Err(rejected(CONFLICTING_UPDATES));
    }
    let query = build_query(id, &segments)?;
    let mut update = Document::new();
    if !set.is_empty() {
        let fields: Document = set_keys
            .into_iter()
            .zip(set.iter().map(|(_, v)| v.clone()))
            .collect();
        update.insert("$set", fields);
    }
    if !unset.is_empty() {
        let fields: Document = unset_keys
            .into_iter()
            .map(|k| (k, Bson::from("")))
            .collect();
        update.insert("$unset", fields);
    }
    let statement = doc! { "q": query.clone(), "u": update, "multi": false };
    Ok(FieldStatement { statement, query })
}

/// Size of the statement for chunking: encoded `set` values, rendered paths, the query and the
/// same per-element overhead as every other kind. An unsendable op never travels.
pub(crate) fn field_element_size(statement: &FieldStatement) -> usize {
    element_size(&statement.statement)
}

impl Writer<'_> {
    /// Sends the field updates; ops that cannot be rendered fail individually without a command.
    pub(crate) async fn run_field_updates(&self, items: &[FieldUpdate]) -> Result<Failures> {
        let mut failures: Failures = Vec::new();
        let mut ready: Vec<(&FieldUpdate, FieldStatement)> = Vec::new();
        for item in items {
            match build_statement(&item.id, &item.set, &item.unset) {
                Ok(statement) => ready.push((item, statement)),
                Err(failure) => failures.push((item.pos, failure)),
            }
        }
        let sizes: Vec<usize> = ready.iter().map(|(_, s)| field_element_size(s)).collect();
        for range in split_chunks(&sizes, MAX_OPS_PER_COMMAND, MAX_BYTES_PER_COMMAND) {
            failures.extend(self.send_field_chunk(&ready[range]).await?);
        }
        Ok(failures)
    }

    async fn send_field_chunk(&self, chunk: &[(&FieldUpdate, FieldStatement)]) -> Result<Failures> {
        let updates: Vec<Bson> = chunk
            .iter()
            .map(|(_, s)| Bson::Document(s.statement.clone()))
            .collect();
        let reply = self
            .send(doc! { "update": self.coll_name, "updates": updates, "ordered": false })
            .await?;
        let errors = map_write_errors(&reply, |i| chunk.get(i).map(|c| c.0.pos))?;
        let expected = chunk.len().saturating_sub(errors.len());
        let mut failures = errors.clone();
        if matched_count(&reply) < expected {
            let queries: Vec<Document> = chunk.iter().map(|(_, s)| s.query.clone()).collect();
            let present = self.existing_matching(queries).await?;
            failures.extend(missing_among(
                chunk.iter().map(|(u, _)| (u.pos, &u.id)),
                &present,
                &errors,
            ));
        }
        Ok(failures)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::diff::PathSegment::Index as I;

    fn f(n: &str) -> PathSegment {
        PathSegment::Field(n.into())
    }

    fn code(r: std::result::Result<impl std::fmt::Debug, WriteFailure>) -> WriteFailure {
        r.unwrap_err()
    }

    #[test]
    fn render_path_table() {
        let ok: Vec<(Vec<PathSegment>, &str)> = vec![
            (vec![f("a")], "a"),
            (vec![f("a"), f("b"), f("c")], "a.b.c"),
            (vec![f("arr"), I(2)], "arr.2"),
            (vec![f("a"), I(0), f("k"), I(12)], "a.0.k.12"),
            (vec![f("ünï"), f("sp ace")], "ünï.sp ace"),
            (vec![f("a$")], "a$"),
        ];
        for (path, want) in ok {
            assert_eq!(render_path(&path).as_deref(), Ok(want), "{path:?}");
        }
        let bad: Vec<(Vec<PathSegment>, i32)> = vec![
            (vec![], 2),
            (vec![f("")], 2),
            (vec![f("a.b")], 2),
            (vec![f("$set")], 2),
            (vec![f("a"), f("")], 2),
            (vec![f("a"), f("b.c")], 2),
            (vec![f("a"), f("$x")], 2),
            (vec![f("a"), PathSegment::AnyElement], 2),
            (vec![PathSegment::AnyElement], 2),
            (vec![I(0)], 2),
            (vec![f("_id")], 66),
            (vec![f("_id"), f("x")], 66),
        ];
        for (path, want) in bad {
            assert_eq!(code(render_path(&path)), rejected(want), "{path:?}");
        }
        assert_eq!(render_path(&[f("x_id")]).as_deref(), Ok("x_id"));
        assert_eq!(render_path(&[f("a"), f("_id")]).as_deref(), Ok("a._id"));
    }

    fn id() -> Bson {
        Bson::Int32(7)
    }

    #[test]
    fn a_set_only_statement_is_exact() {
        let set = vec![
            (vec![f("a"), f("b")], Bson::Int64(1)),
            (vec![f("c")], Bson::String("v".into())),
        ];
        let got = build_statement(&id(), &set, &[]).unwrap();
        let q = doc! { "_id": { "$eq": 7 } };
        let want = doc! {
            "q": q.clone(),
            "u": { "$set": { "a.b": 1_i64, "c": "v" } },
            "multi": false,
        };
        assert_eq!(got.statement, want);
        assert_eq!(got.query, q);
        assert!(
            !got.statement
                .get_document("u")
                .unwrap()
                .contains_key("$unset")
        );
    }

    #[test]
    fn an_unset_only_and_a_both_statement_are_exact() {
        let unset = vec![vec![f("a"), f("b")], vec![f("x")]];
        let got = build_statement(&id(), &[], &unset).unwrap();
        assert_eq!(
            got.statement.get_document("u").unwrap(),
            &doc! { "$unset": { "a.b": "", "x": "" } }
        );
        let set = vec![(vec![f("s")], Bson::Null)];
        let got = build_statement(&id(), &set, &[vec![f("u")]]).unwrap();
        assert_eq!(
            got.statement.get_document("u").unwrap(),
            &doc! { "$set": { "s": Bson::Null }, "$unset": { "u": "" } }
        );
        assert_eq!(got.statement.get_bool("multi"), Ok(false));
    }

    #[test]
    fn a_sub_document_id_stays_a_literal_in_the_query() {
        let id = Bson::Document(doc! { "$gt": "" });
        let got = build_statement(&id, &[(vec![f("a")], Bson::Null)], &[]).unwrap();
        assert_eq!(got.query, doc! { "_id": { "$eq": { "$gt": "" } } });
    }

    #[test]
    fn an_index_path_adds_one_array_guard_per_distinct_prefix() {
        let set = vec![
            (vec![f("arr"), I(0)], Bson::Int32(1)),
            (vec![f("arr"), I(2), f("k")], Bson::Int32(2)),
            (vec![f("m"), I(1), f("n"), I(3)], Bson::Int32(3)),
        ];
        let got = build_statement(&id(), &set, &[]).unwrap();
        let want = doc! {
            "_id": { "$eq": 7 },
            "arr": { "$type": "array" },
            "m": { "$type": "array" },
            "m.1.n": { "$type": "array" },
        };
        assert_eq!(got.query, want);
        assert_eq!(got.statement.get_document("q").unwrap(), &want);
        let u = got
            .statement
            .get_document("u")
            .unwrap()
            .get_document("$set")
            .unwrap();
        assert_eq!(
            u.keys().collect::<Vec<_>>(),
            ["arr.0", "arr.2.k", "m.1.n.3"]
        );
    }

    #[test]
    fn a_numeric_field_name_guards_its_parent_as_an_object() {
        let got = build_statement(&id(), &[(vec![f("a"), f("0")], Bson::Null)], &[]).unwrap();
        assert_eq!(
            got.query,
            doc! { "_id": { "$eq": 7 }, "a": { "$type": "object" } }
        );
        let top = build_statement(&id(), &[(vec![f("0")], Bson::Null)], &[]).unwrap();
        assert_eq!(top.query, doc! { "_id": { "$eq": 7 } });
    }

    #[test]
    fn contradicting_guards_are_a_conflict() {
        let set = vec![
            (vec![f("a"), I(0), f("x")], Bson::Null),
            (vec![f("a"), f("0"), f("y")], Bson::Null),
        ];
        assert_eq!(code(build_statement(&id(), &set, &[])), rejected(40));
    }

    #[test]
    fn conflicting_paths_are_refused_before_sending() {
        let a = |p: &[PathSegment]| (p.to_vec(), Bson::Null);
        type Case = (Vec<(Vec<PathSegment>, Bson)>, Vec<Vec<PathSegment>>);
        let cases: Vec<Case> = vec![
            (vec![a(&[f("a")]), a(&[f("a"), f("b")])], vec![]),
            (vec![a(&[f("a")]), a(&[f("a")])], vec![]),
            (vec![a(&[f("a"), f("b")])], vec![vec![f("a")]]),
            (vec![a(&[f("x")])], vec![vec![f("x")]]),
            (vec![a(&[f("a"), f("1")]), a(&[f("a"), I(1)])], vec![]),
            (vec![a(&[f("a"), f("1")])], vec![vec![f("a"), I(1), f("z")]]),
        ];
        for (set, unset) in cases {
            assert_eq!(
                code(build_statement(&id(), &set, &unset)),
                rejected(40),
                "{set:?} {unset:?}"
            );
        }
    }

    #[test]
    fn bad_names_empty_ops_and_id_are_refused() {
        for bad in [vec![f("a.b")], vec![f("$x")], vec![f("")], vec![f("_id")]] {
            let set = vec![(vec![f("ok")], Bson::Null), (bad.clone(), Bson::Null)];
            assert!(build_statement(&id(), &set, &[]).is_err(), "{bad:?}");
            assert!(
                build_statement(&id(), &[], &[bad.clone()]).is_err(),
                "{bad:?}"
            );
        }
        assert_eq!(code(build_statement(&id(), &[], &[])), rejected(2));
    }

    #[test]
    fn element_size_counts_values_paths_and_overhead() {
        let small = build_statement(&id(), &[(vec![f("a")], Bson::Int32(1))], &[]).unwrap();
        let big_value = build_statement(
            &id(),
            &[(vec![f("a")], Bson::String("x".repeat(1 << 20)))],
            &[],
        )
        .unwrap();
        let long_path =
            build_statement(&id(), &[(vec![f(&"p".repeat(5_000))], Bson::Int32(1))], &[]).unwrap();
        let (s, v, p) = (
            field_element_size(&small),
            field_element_size(&big_value),
            field_element_size(&long_path),
        );
        assert!(s >= 128, "overhead counted: {s}");
        assert!(v > (1 << 20) && v < s + (1 << 20) + 64, "{v}");
        assert!(p > s + 4_900 && p < s + 5_200, "{p}");
    }

    #[test]
    fn large_field_updates_split_by_bytes_and_keep_order() {
        let statement = |n: usize| {
            let set = vec![(vec![f("v")], Bson::String("x".repeat(n)))];
            build_statement(&id(), &set, &[]).unwrap()
        };
        let sizes: Vec<usize> = (0..20)
            .map(|_| field_element_size(&statement(1_000_000)))
            .collect();
        let got = split_chunks(&sizes, MAX_OPS_PER_COMMAND, MAX_BYTES_PER_COMMAND);
        assert!(got.len() >= 2);
        assert!(got.iter().all(|r| r.len() <= 15));
        let flat: Vec<usize> = got.iter().flat_map(|r| r.clone()).collect();
        assert_eq!(flat, (0..20).collect::<Vec<_>>());
    }
}
