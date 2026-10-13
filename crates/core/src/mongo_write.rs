//! Batched writes for [`crate::mongo_store::MongoStore`], sent as raw write commands
//! (`insert` / `update` / `delete`, `ordered: false`).
//!
//! Server text (`errmsg`) is never copied into a result: duplicate-key messages quote the key
//! values.

use std::cmp::Ordering;

use bson::{Bson, Document, doc};
use futures::TryStreamExt;
use mongodb::options::{ReadPreference, SelectionCriteria};
use mongodb::{Collection, Database};

use crate::compare::key::compare_bson;
use crate::error::{CoreError, Result};
use crate::store::{WriteFailure, WriteOp, WriteOutcome};

/// Most operations sent in one write command.
pub(crate) const MAX_OPS_PER_COMMAND: usize = 500;

/// The server's duplicate-key error code.
const DUPLICATE_KEY_CODE: i32 = 11000;

/// Failures found by a group: `(position of the op in the caller's list, why)`.
type Failures = Vec<(usize, WriteFailure)>;

/// The caller's ops split by kind, each paired with its original position.
#[derive(Default)]
struct Groups {
    inserts: Vec<(usize, Document)>,
    replaces: Vec<(usize, Bson, Document)>,
    deletes: Vec<(usize, Bson)>,
}

fn group_ops(ops: Vec<WriteOp>) -> Groups {
    ops.into_iter()
        .enumerate()
        .fold(Groups::default(), |mut groups, (pos, op)| {
            match op {
                WriteOp::Insert { doc } => groups.inserts.push((pos, doc)),
                WriteOp::Replace { id, doc } => groups.replaces.push((pos, id, doc)),
                WriteOp::Delete { id } => groups.deletes.push((pos, id)),
            }
            groups
        })
}

/// Applies `ops` to `coll_name` in `db`: one outcome per op, index-aligned.
///
/// Inserts are sent first, then replaces, then deletes; there is no ordering within a call. An
/// `Err` may follow commands that already wrote, so after an `Err` nothing may be assumed about
/// what was written. Callers must not display the text of a `CoreError::Database` from here
/// (it can carry driver text). Limitation: the `Missing` probe compares ids under the simple
/// collation, so on a collection with a non-simple default collation an id that matches only
/// under that collation may be reported `Missing`.
pub(crate) async fn bulk_write(
    db: &Database,
    coll_name: &str,
    ops: Vec<WriteOp>,
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> Result<Vec<WriteOutcome>> {
    let total = ops.len();
    let groups = group_ops(ops);
    let writer = Writer {
        db,
        coll: db.collection::<Document>(coll_name),
        coll_name,
        db_err,
    };
    let mut failures: Failures = writer.run_inserts(&groups.inserts).await?;
    failures.extend(writer.run_replaces(&groups.replaces).await?);
    failures.extend(writer.run_deletes(&groups.deletes).await?);
    let mut outcomes: Vec<WriteOutcome> = vec![Ok(()); total];
    for (pos, failure) in failures {
        outcomes[pos] = Err(failure);
    }
    Ok(outcomes)
}

struct Writer<'a> {
    db: &'a Database,
    coll: Collection<Document>,
    coll_name: &'a str,
    db_err: fn(mongodb::error::Error) -> CoreError,
}

impl Writer<'_> {
    /// Sends one command; a command-level failure (`ok: 0`, network) is an `Err`.
    async fn send(&self, command: Document) -> Result<Document> {
        let reply = self.db.run_command(command).await.map_err(self.db_err)?;
        check_write_concern(&reply)?;
        Ok(reply)
    }

    async fn run_inserts(&self, items: &[(usize, Document)]) -> Result<Failures> {
        let mut failures = Vec::new();
        for chunk in items.chunks(MAX_OPS_PER_COMMAND) {
            let documents: Vec<Bson> = chunk
                .iter()
                .map(|(_, d)| Bson::Document(d.clone()))
                .collect();
            let reply = self
                .send(doc! { "insert": self.coll_name, "documents": documents, "ordered": false })
                .await?;
            failures.extend(map_write_errors(&reply, |i| chunk.get(i).map(|c| c.0))?);
        }
        Ok(failures)
    }

    async fn run_replaces(&self, items: &[(usize, Bson, Document)]) -> Result<Failures> {
        let mut failures = Vec::new();
        for chunk in items.chunks(MAX_OPS_PER_COMMAND) {
            let updates: Vec<Bson> = chunk
                .iter()
                .map(|(_, id, d)| {
                    Bson::Document(
                        doc! { "q": id_query(id), "u": replacement(id, d), "multi": false },
                    )
                })
                .collect();
            let reply = self
                .send(doc! { "update": self.coll_name, "updates": updates, "ordered": false })
                .await?;
            let errors = map_write_errors(&reply, |i| chunk.get(i).map(|c| c.0))?;
            let expected = chunk.len().saturating_sub(errors.len());
            let matched = matched_count(&reply);
            failures.extend(errors.clone());
            if matched < expected {
                // Replaced documents still exist: whichever id is now absent matched nothing.
                let ids: Vec<&Bson> = chunk.iter().map(|c| &c.1).collect();
                let present = self.existing_ids(&ids).await?;
                failures.extend(missing_among(
                    chunk.iter().map(|c| (c.0, &c.1)),
                    &present,
                    &errors,
                ));
            }
        }
        Ok(failures)
    }

    async fn run_deletes(&self, items: &[(usize, Bson)]) -> Result<Failures> {
        let mut failures = Vec::new();
        for chunk in items.chunks(MAX_OPS_PER_COMMAND) {
            // Deleted documents are gone afterwards, so what existed is read first.
            let ids: Vec<&Bson> = chunk.iter().map(|c| &c.1).collect();
            let present = self.existing_ids(&ids).await?;
            let deletes: Vec<Bson> = chunk
                .iter()
                .map(|(_, id)| Bson::Document(doc! { "q": id_query(id), "limit": 1 }))
                .collect();
            let reply = self
                .send(doc! { "delete": self.coll_name, "deletes": deletes, "ordered": false })
                .await?;
            let errors = map_write_errors(&reply, |i| chunk.get(i).map(|c| c.0))?;
            let expected = chunk.len().saturating_sub(errors.len());
            failures.extend(errors.clone());
            if matched_count(&reply) < expected {
                failures.extend(missing_among(
                    chunk.iter().map(|c| (c.0, &c.1)),
                    &present,
                    &errors,
                ));
            }
        }
        Ok(failures)
    }

    /// The `_id`s among `ids` that exist in the collection right now.
    async fn existing_ids(&self, ids: &[&Bson]) -> Result<Vec<Bson>> {
        let any_of: Vec<Bson> = ids.iter().map(|id| Bson::Document(id_query(id))).collect();
        let cursor = self
            .coll
            .find(doc! { "$or": any_of })
            .projection(doc! { "_id": 1 })
            .selection_criteria(SelectionCriteria::ReadPreference(ReadPreference::Primary))
            .await
            .map_err(self.db_err)?;
        let found: Vec<Document> = cursor.try_collect().await.map_err(self.db_err)?;
        Ok(found
            .into_iter()
            .filter_map(|d| d.get("_id").cloned())
            .collect())
    }
}

/// `{ _id: { $eq: id } }`: an id that is a sub-document with `$` keys stays a literal value.
fn id_query(id: &Bson) -> Document {
    doc! { "_id": { "$eq": id.clone() } }
}

/// The replacement document with `_id` first (from the op) and the rest of `doc` in order, so
/// the server can never read it as an operator update (`{ $set: .. }` is then a field named
/// `$set`, which it rejects).
fn replacement(id: &Bson, doc: &Document) -> Document {
    std::iter::once(("_id".to_string(), id.clone()))
        .chain(
            doc.iter()
                .filter(|(k, _)| *k != "_id")
                .map(|(k, v)| (k.clone(), v.clone())),
        )
        .collect()
}

/// A reply whose write concern was not satisfied is a failure of the whole command (what was
/// written is unknown). No server text is kept.
fn check_write_concern(reply: &Document) -> Result<()> {
    if reply.contains_key("writeConcernError") {
        return Err(CoreError::Database("write concern not satisfied".into()));
    }
    Ok(())
}

/// The reply's `n` (matched / deleted count).
fn matched_count(reply: &Document) -> usize {
    match reply.get("n") {
        Some(Bson::Int32(n)) => usize::try_from(*n).unwrap_or(0),
        Some(Bson::Int64(n)) => usize::try_from(*n).unwrap_or(0),
        _ => 0,
    }
}

/// `writeErrors` of a reply as failures: `position_of` maps the command-local index back to the
/// op's position. Only the numeric code is kept.
fn map_write_errors(
    reply: &Document,
    position_of: impl Fn(usize) -> Option<usize>,
) -> Result<Failures> {
    let unmappable = || CoreError::Database("unexpected write error reply".into());
    let Ok(errors) = reply.get_array("writeErrors") else {
        return Ok(Vec::new());
    };
    errors
        .iter()
        .map(|entry| {
            let e = entry.as_document().ok_or_else(unmappable)?;
            let index = usize::try_from(error_number(e.get("index").ok_or_else(unmappable)?))
                .map_err(|_| unmappable())?;
            let code = i32::try_from(e.get("code").map_or(0, error_number)).unwrap_or(0);
            Ok((
                position_of(index).ok_or_else(unmappable)?,
                failure_for_code(code),
            ))
        })
        .collect()
}

fn error_number(value: &Bson) -> i64 {
    match value {
        Bson::Int32(n) => i64::from(*n),
        Bson::Int64(n) => *n,
        Bson::Double(n) => *n as i64,
        _ => -1,
    }
}

fn failure_for_code(code: i32) -> WriteFailure {
    if code == DUPLICATE_KEY_CODE {
        WriteFailure::DuplicateKey
    } else {
        WriteFailure::Rejected(code)
    }
}

/// `Missing` for every `(position, id)` whose id is not in `present` (compared as the server
/// does, so `1` and `1.0` are the same id) and that did not already fail with a write error.
fn missing_among<'a>(
    items: impl Iterator<Item = (usize, &'a Bson)>,
    present: &[Bson],
    already_failed: &Failures,
) -> Failures {
    items
        .filter(|(pos, _)| !already_failed.iter().any(|(p, _)| p == pos))
        .filter(|(_, id)| {
            !present
                .iter()
                .any(|p| compare_bson(p, id) == Ordering::Equal)
        })
        .map(|(pos, _)| (pos, WriteFailure::Missing))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_errors_map_codes_and_positions_without_server_text() {
        let reply = doc! { "ok": 1, "n": 1, "writeErrors": [
            { "index": 0, "code": 11000, "errmsg": "dup key: { _id: \"SECRET\" }" },
            { "index": 2, "code": 121, "errmsg": "validation" },
        ]};
        let got = map_write_errors(&reply, |i| Some(i + 10)).unwrap();
        assert_eq!(
            got,
            vec![
                (10, WriteFailure::DuplicateKey),
                (12, WriteFailure::Rejected(121))
            ]
        );
        assert!(!format!("{got:?}").contains("SECRET"));
    }

    #[test]
    fn an_unmappable_write_error_fails_the_whole_call() {
        let reply = doc! { "writeErrors": [{ "index": 7, "code": 1 }] };
        assert!(map_write_errors(&reply, |i| (i < 3).then_some(i)).is_err());
        let reply = doc! { "writeErrors": ["junk"] };
        assert!(map_write_errors(&reply, Some).is_err());
    }

    #[test]
    fn a_write_concern_error_fails_the_command_without_server_text() {
        let reply =
            doc! { "ok": 1, "n": 1, "writeConcernError": { "code": 64, "errmsg": "SECRET" } };
        let err = check_write_concern(&reply).unwrap_err();
        assert!(!format!("{err:?}").contains("SECRET"));
        assert!(check_write_concern(&doc! { "ok": 1 }).is_ok());
    }

    #[test]
    fn a_replacement_starts_with_the_ops_id_and_drops_any_other_id() {
        let got = replacement(
            &Bson::Int32(1),
            &doc! { "$set": { "x": 1 }, "_id": 9, "a": 2 },
        );
        assert_eq!(got.keys().collect::<Vec<_>>(), ["_id", "$set", "a"]);
        assert_eq!(got.get("_id"), Some(&Bson::Int32(1)));
    }

    #[test]
    fn ops_are_grouped_by_kind_keeping_their_positions() {
        let groups = group_ops(vec![
            WriteOp::Delete { id: Bson::Int32(1) },
            WriteOp::Insert {
                doc: doc! { "a": 1 },
            },
            WriteOp::Replace {
                id: Bson::Int32(2),
                doc: doc! { "_id": 2 },
            },
        ]);
        assert_eq!(groups.deletes[0].0, 0);
        assert_eq!(groups.inserts[0].0, 1);
        assert_eq!(groups.replaces[0].0, 2);
    }

    #[test]
    fn missing_ignores_ids_that_failed_and_matches_numbers_across_types() {
        let (a, b, c) = (Bson::Int32(1), Bson::Int32(2), Bson::Int32(3));
        let items = vec![(0, &a), (1, &b), (2, &c)];
        let present = vec![Bson::Double(1.0)];
        let failed = vec![(1, WriteFailure::Rejected(5))];
        let got = missing_among(items.into_iter(), &present, &failed);
        assert_eq!(got, vec![(2, WriteFailure::Missing)]);
    }
}
