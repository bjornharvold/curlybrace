//! `WriteOp::UpdateFields` for the in-memory store: the segments are applied directly to the
//! stored document, and an op either applies completely or not at all.
//!
//! Failure codes mirror the server where there is a close match: 66 (`ImmutableField`) for
//! `_id`, 28 (`PathNotViable`) for a missing parent or an index beyond the end of an array
//! (the server would pad with nulls or fail; the planner never asks for it), 2 (`BadValue`)
//! for a path the planner would never produce (empty, `AnyElement`, a name that is empty, has
//! a `.` or starts with `$`, an `$unset` through an array).

use bson::{Bson, Document};

use crate::compare::apply_fields::paths_conflict;
use crate::compare::diff::PathSegment;
use crate::store::WriteFailure;

const IMMUTABLE_FIELD: i32 = 66;
const PATH_NOT_VIABLE: i32 = 28;
const BAD_VALUE: i32 = 2;
const CONFLICTING_UPDATES: i32 = 40;

fn rejected(code: i32) -> WriteFailure {
    WriteFailure::Rejected(code)
}

/// Refuses what no vetted plan contains, so a bad path is a typed failure and never a write.
fn check_path(path: &[PathSegment]) -> Result<(), WriteFailure> {
    match path.first() {
        None => return Err(rejected(BAD_VALUE)),
        Some(PathSegment::Field(name)) if name == "_id" => return Err(rejected(IMMUTABLE_FIELD)),
        Some(_) => {}
    }
    let unrenderable = path.iter().any(|s| match s {
        PathSegment::AnyElement => true,
        PathSegment::Field(name) => name.is_empty() || name.contains('.') || name.starts_with('$'),
        PathSegment::Index(_) => false,
    });
    if unrenderable || !matches!(path[0], PathSegment::Field(_)) {
        return Err(rejected(BAD_VALUE));
    }
    Ok(())
}

/// Sets `value` at `path` below `node`. The final field may be created; every parent and every
/// array index must already exist.
fn set_in(node: &mut Bson, path: &[PathSegment], value: Bson) -> Result<(), WriteFailure> {
    match (node, path) {
        (Bson::Document(d), [PathSegment::Field(name)]) => {
            d.insert(name.clone(), value);
            Ok(())
        }
        (Bson::Document(d), [PathSegment::Field(name), rest @ ..]) => {
            let child = d.get_mut(name).ok_or(rejected(PATH_NOT_VIABLE))?;
            set_in(child, rest, value)
        }
        (Bson::Array(a), [PathSegment::Index(i), rest @ ..]) => {
            let slot = a.get_mut(*i).ok_or(rejected(PATH_NOT_VIABLE))?;
            if rest.is_empty() {
                *slot = value;
                Ok(())
            } else {
                set_in(slot, rest, value)
            }
        }
        _ => Err(rejected(PATH_NOT_VIABLE)),
    }
}

/// Removes the field at `path` below `node`. Absent parents or fields are a no-op (as on the
/// server); passing through an array is refused, because the server would null the element.
fn unset_in(node: &mut Bson, path: &[PathSegment]) -> Result<(), WriteFailure> {
    match (node, path) {
        (Bson::Document(d), [PathSegment::Field(name)]) => {
            d.remove(name);
            Ok(())
        }
        (Bson::Document(d), [PathSegment::Field(name), rest @ ..]) => d
            .get_mut(name)
            .map_or(Ok(()), |child| unset_in(child, rest)),
        (Bson::Array(_), _) | (_, [PathSegment::Index(_), ..]) => Err(rejected(BAD_VALUE)),
        _ => Ok(()),
    }
}

/// Applies the update to a copy of `doc`; the caller replaces the stored document only on `Ok`,
/// so an op either applies completely or not at all.
pub(super) fn update_fields(
    doc: &Document,
    set: &[(Vec<PathSegment>, Bson)],
    unset: &[Vec<PathSegment>],
) -> Result<Document, WriteFailure> {
    let all: Vec<Vec<PathSegment>> = set
        .iter()
        .map(|(p, _)| p.clone())
        .chain(unset.iter().cloned())
        .collect();
    all.iter().try_for_each(|p| check_path(p))?;
    if paths_conflict(&all) {
        return Err(rejected(CONFLICTING_UPDATES));
    }
    let mut root = Bson::Document(doc.clone());
    for (path, value) in set {
        set_in(&mut root, path, value.clone())?;
    }
    for path in unset {
        unset_in(&mut root, path)?;
    }
    match root {
        Bson::Document(updated) => Ok(updated),
        _ => Err(rejected(BAD_VALUE)),
    }
}

#[cfg(test)]
mod tests {
    use bson::doc;

    use super::super::memory_store::InMemoryStore;
    use super::super::mock_objects::*;
    use crate::compare::diff::PathSegment::{self, Index as I};
    use crate::store::{DocumentStore, Namespace, Query, WriteFailure, WriteOp};
    use bson::{Bson, Document};

    fn f(n: &str) -> PathSegment {
        PathSegment::Field(n.into())
    }

    fn store() -> (Namespace, InMemoryStore) {
        let ns = mock_namespace();
        let docs = vec![
            doc! { "_id": 1, "a": { "b": 1, "c": 2 }, "arr": [10, 20, { "k": 1 }], "keep": "k" },
            doc! { "_id": 2, "x": 1 },
        ];
        (ns.clone(), InMemoryStore::new().with_docs(&ns, docs))
    }

    async fn get(s: &InMemoryStore, ns: &Namespace, id: i32) -> Document {
        let q = Query {
            filter: doc! { "_id": id },
            ..Query::default()
        };
        s.find(ns, &q).await.unwrap().docs.remove(0)
    }

    fn update(
        id: i32,
        set: Vec<(Vec<PathSegment>, Bson)>,
        unset: Vec<Vec<PathSegment>>,
    ) -> WriteOp {
        WriteOp::UpdateFields {
            id: Bson::Int32(id),
            set,
            unset,
        }
    }

    async fn run(ops: Vec<WriteOp>) -> (Vec<Result<(), WriteFailure>>, InMemoryStore, Namespace) {
        let (ns, s) = store();
        let out = s.bulk_write(&ns, ops).await.unwrap();
        (out, s, ns)
    }

    #[tokio::test]
    async fn sets_a_nested_field_and_leaves_the_rest() {
        let (out, s, ns) = run(vec![update(
            1,
            vec![(vec![f("a"), f("b")], Bson::Int32(9))],
            vec![],
        )])
        .await;
        assert_eq!(out, vec![Ok(())]);
        assert_eq!(
            get(&s, &ns, 1).await,
            doc! { "_id": 1, "a": { "b": 9, "c": 2 }, "arr": [10, 20, { "k": 1 }], "keep": "k" }
        );
    }

    #[tokio::test]
    async fn creates_a_missing_final_field_in_an_existing_parent() {
        let set = vec![
            (vec![f("a"), f("new")], Bson::String("v".into())),
            (vec![f("top")], Bson::Int32(1)),
        ];
        let (out, s, ns) = run(vec![update(1, set, vec![])]).await;
        assert_eq!(out, vec![Ok(())]);
        let d = get(&s, &ns, 1).await;
        assert_eq!(
            d.get_document("a").unwrap(),
            &doc! { "b": 1, "c": 2, "new": "v" }
        );
        assert_eq!(d.get_i32("top").unwrap(), 1);
    }

    #[tokio::test]
    async fn a_missing_parent_is_rejected_and_nothing_is_written() {
        let set = vec![
            (vec![f("keep2")], Bson::Int32(1)),
            (vec![f("nope"), f("deep")], Bson::Int32(1)),
        ];
        let (out, s, ns) = run(vec![update(1, set, vec![])]).await;
        assert!(matches!(out[0], Err(WriteFailure::Rejected(_))));
        assert!(!get(&s, &ns, 1).await.contains_key("keep2"));
    }

    #[tokio::test]
    async fn sets_an_array_element_by_index_within_bounds() {
        let set = vec![
            (vec![f("arr"), I(0)], Bson::Int32(11)),
            (vec![f("arr"), I(2), f("k")], Bson::Int32(5)),
        ];
        let (out, s, ns) = run(vec![update(1, set, vec![])]).await;
        assert_eq!(out, vec![Ok(())]);
        assert_eq!(
            get(&s, &ns, 1).await.get_array("arr").unwrap(),
            &vec![
                Bson::Int32(11),
                Bson::Int32(20),
                Bson::Document(doc! { "k": 5 })
            ]
        );
    }

    #[tokio::test]
    async fn an_index_beyond_the_array_is_rejected_without_padding() {
        let set = vec![(vec![f("arr"), I(3)], Bson::Int32(1))];
        let (out, s, ns) = run(vec![update(1, set, vec![])]).await;
        assert!(matches!(out[0], Err(WriteFailure::Rejected(_))));
        assert_eq!(get(&s, &ns, 1).await.get_array("arr").unwrap().len(), 3);
    }

    #[tokio::test]
    async fn an_index_on_a_non_array_or_a_field_on_a_scalar_is_rejected() {
        for path in [
            vec![f("a"), I(0)],
            vec![f("keep"), f("x")],
            vec![f("arr"), f("x")],
        ] {
            let (out, _, _) = run(vec![update(1, vec![(path, Bson::Int32(1))], vec![])]).await;
            assert!(matches!(out[0], Err(WriteFailure::Rejected(_))));
        }
    }

    #[tokio::test]
    async fn unsets_a_field_nested_or_top_level_and_an_absent_one_is_a_no_op() {
        let unset = vec![
            vec![f("a"), f("b")],
            vec![f("keep")],
            vec![f("absent")],
            vec![f("nope"), f("deep")],
        ];
        let (out, s, ns) = run(vec![update(1, vec![], unset)]).await;
        assert_eq!(out, vec![Ok(())]);
        assert_eq!(
            get(&s, &ns, 1).await,
            doc! { "_id": 1, "a": { "c": 2 }, "arr": [10, 20, { "k": 1 }] }
        );
    }

    #[tokio::test]
    async fn an_unset_through_an_array_is_rejected() {
        for path in [vec![f("arr"), I(0)], vec![f("arr"), I(2), f("k")]] {
            let (out, s, ns) = run(vec![update(1, vec![], vec![path])]).await;
            assert!(matches!(out[0], Err(WriteFailure::Rejected(_))));
            assert_eq!(get(&s, &ns, 1).await.get_array("arr").unwrap().len(), 3);
        }
    }

    #[tokio::test]
    async fn id_empty_and_unrenderable_paths_are_rejected() {
        let bad: Vec<Vec<PathSegment>> = vec![
            vec![f("_id")],
            vec![f("_id"), f("x")],
            vec![],
            vec![f("")],
            vec![f("a.b")],
            vec![f("$x")],
            vec![f("a"), PathSegment::AnyElement],
        ];
        for path in bad {
            let (o1, s, ns) = run(vec![update(
                1,
                vec![(path.clone(), Bson::Int32(1))],
                vec![],
            )])
            .await;
            let (o2, _, _) = run(vec![update(1, vec![], vec![path.clone()])]).await;
            assert!(matches!(o1[0], Err(WriteFailure::Rejected(_))), "{path:?}");
            assert!(matches!(o2[0], Err(WriteFailure::Rejected(_))), "{path:?}");
            assert_eq!(get(&s, &ns, 1).await.get_i32("_id").unwrap(), 1);
        }
    }

    #[tokio::test]
    async fn conflicting_paths_across_set_and_unset_are_rejected_and_nothing_is_written() {
        let cases = [
            (
                vec![(vec![f("a"), f("b")], Bson::Int32(0))],
                vec![vec![f("a")]],
            ),
            (
                vec![
                    (vec![f("a")], Bson::Int32(0)),
                    (vec![f("a")], Bson::Int32(1)),
                ],
                vec![],
            ),
            (vec![(vec![f("x")], Bson::Int32(0))], vec![vec![f("x")]]),
        ];
        for (set, unset) in cases {
            let (out, s, ns) = run(vec![update(1, set, unset)]).await;
            assert_eq!(out, vec![Err(WriteFailure::Rejected(40))]);
            assert_eq!(
                get(&s, &ns, 1).await.get_document("a").unwrap(),
                &doc! { "b": 1, "c": 2 }
            );
        }
    }

    #[tokio::test]
    async fn a_missing_id_is_missing() {
        let (out, _, _) = run(vec![update(
            77,
            vec![(vec![f("x")], Bson::Int32(1))],
            vec![],
        )])
        .await;
        assert_eq!(out, vec![Err(WriteFailure::Missing)]);
    }

    #[tokio::test]
    async fn outcomes_are_index_aligned_with_other_kinds_in_one_call() {
        let ops = vec![
            WriteOp::Delete { id: Bson::Int32(2) },
            update(1, vec![(vec![f("a"), f("b")], Bson::Int32(0))], vec![]),
            update(2, vec![(vec![f("x")], Bson::Int32(0))], vec![]),
            WriteOp::Insert {
                doc: doc! { "_id": 5 },
            },
            update(1, vec![(vec![f("_id")], Bson::Int32(0))], vec![]),
            update(1, vec![], vec![vec![f("keep")]]),
        ];
        let (out, s, ns) = run(ops).await;
        assert_eq!(out.len(), 6);
        assert_eq!(out[0], Ok(()));
        assert_eq!(out[1], Ok(()));
        assert_eq!(out[2], Err(WriteFailure::Missing));
        assert_eq!(out[3], Ok(()));
        assert!(matches!(out[4], Err(WriteFailure::Rejected(_))));
        assert_eq!(out[5], Ok(()));
        let d = get(&s, &ns, 1).await;
        assert_eq!(d.get_document("a").unwrap().get_i32("b").unwrap(), 0);
        assert!(!d.contains_key("keep"));
    }

    #[tokio::test]
    async fn a_set_keeps_the_stored_value_type_given() {
        let (_, s, ns) = run(vec![update(
            1,
            vec![(vec![f("a"), f("b")], Bson::Int64(4))],
            vec![],
        )])
        .await;
        assert_eq!(
            get(&s, &ns, 1).await.get_document("a").unwrap().get("b"),
            Some(&Bson::Int64(4))
        );
    }
}
