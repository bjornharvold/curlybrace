//! Apply types and pure write planning for data compare. No I/O: the runtime re-reads the
//! documents, checks them against the recorded diff entry, and asks this module what to write.

use bson::{Bson, Document};

use crate::compare::key::KeyTuple;
use crate::compare::protocol::PairId;
use crate::error::{CoreError, Result};
use crate::store::{WriteFailure, WriteOp};

/// Operations per `bulk_write` call.
pub const APPLY_BATCH: usize = 500;
/// Failures kept in an `ApplyReport`; the rest are only counted.
pub const MAX_REPORTED_FAILURES: usize = 100;

/// Origin -> destination of the copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    SourceToTarget,
    TargetToSource,
}

/// What happens to the DESTINATION document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ApplyKind {
    Insert,
    Replace,
    Delete,
}

/// Insert only: keep the origin `_id` or let the server generate one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum IdPolicy {
    #[default]
    Keep,
    Generate,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ApplyOp {
    pub pair: PairId,
    pub key: KeyTuple,
    pub kind: ApplyKind,
    pub direction: Direction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    Stale,
    NotPlanned,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OpOutcome {
    Applied,
    Skipped(SkipReason),
    Failed(CoreError),
}

/// `key` is for the UI only and must never be logged.
#[derive(Clone, Debug, PartialEq)]
pub struct OpFailure {
    pub key: KeyTuple,
    pub error: CoreError,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ApplyReport {
    pub applied: u64,
    pub stale: u64,
    pub not_planned: u64,
    pub failed: u64,
    pub not_run: u64,
    /// The first `MAX_REPORTED_FAILURES` failures.
    pub failures: Vec<OpFailure>,
    pub more_failures: bool,
}

impl ApplyReport {
    /// Counts one outcome; failures beyond the cap are only counted.
    pub fn record(&mut self, key: &KeyTuple, outcome: &OpOutcome) {
        match outcome {
            OpOutcome::Applied => self.applied += 1,
            OpOutcome::Skipped(SkipReason::Stale) => self.stale += 1,
            OpOutcome::Skipped(SkipReason::NotPlanned) => self.not_planned += 1,
            OpOutcome::Failed(error) => {
                self.failed += 1;
                if self.failures.len() < MAX_REPORTED_FAILURES {
                    self.failures.push(OpFailure {
                        key: key.clone(),
                        error: error.clone(),
                    });
                } else {
                    self.more_failures = true;
                }
            }
        }
    }
}

/// Does `kind` fit a row that exists on the sides `has_source`/`has_target` (from the recorded
/// `DiffEntry`)? Insert: origin yes, destination no. Replace: both. Delete: destination yes,
/// origin no.
pub fn kind_fits(
    kind: ApplyKind,
    direction: Direction,
    has_source: bool,
    has_target: bool,
) -> bool {
    let (has_origin, has_dest) = match direction {
        Direction::SourceToTarget => (has_source, has_target),
        Direction::TargetToSource => (has_target, has_source),
    };
    match kind {
        ApplyKind::Insert => has_origin && !has_dest,
        ApplyKind::Replace => has_origin && has_dest,
        ApplyKind::Delete => !has_origin && has_dest,
    }
}

fn invalid() -> CoreError {
    CoreError::InvalidDocument(String::new())
}

fn require(doc: Option<&Document>) -> Result<&Document> {
    doc.ok_or_else(invalid)
}

fn dest_id(dest: Option<&Document>) -> Result<Bson> {
    require(dest)?.get("_id").cloned().ok_or_else(invalid)
}

/// The write for one verified op. `origin`/`dest` are the freshly re-read documents.
pub fn plan_write(
    kind: ApplyKind,
    origin: Option<&Document>,
    dest: Option<&Document>,
    policy: IdPolicy,
) -> Result<WriteOp> {
    match kind {
        ApplyKind::Insert => {
            let mut doc = require(origin)?.clone();
            if policy == IdPolicy::Generate {
                doc.remove("_id");
            }
            Ok(WriteOp::Insert { doc })
        }
        ApplyKind::Replace => {
            let origin = require(origin)?;
            let id = dest_id(dest)?;
            let mut doc = Document::new();
            doc.insert("_id", id.clone());
            for (k, v) in origin.iter().filter(|(k, _)| k.as_str() != "_id") {
                doc.insert(k.clone(), v.clone());
            }
            Ok(WriteOp::Replace { id, doc })
        }
        ApplyKind::Delete => Ok(WriteOp::Delete { id: dest_id(dest)? }),
    }
}

/// Static texts only: never a value or server message.
pub fn map_write_failure(f: WriteFailure) -> CoreError {
    match f {
        WriteFailure::Missing => CoreError::NotFound,
        WriteFailure::DuplicateKey => CoreError::Database("duplicate key".into()),
        WriteFailure::Rejected(_) => CoreError::ApplyWriteRejected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Direction::{SourceToTarget as ST, TargetToSource as TS};
    use bson::{doc, oid::ObjectId};

    #[test]
    fn kind_fits_full_table() {
        use ApplyKind::*;
        let cases = [
            // (has_source, has_target, kind, direction, fits)
            (true, false, Insert, ST, true),
            (true, false, Delete, TS, true),
            (true, false, Delete, ST, false),
            (true, false, Insert, TS, false),
            (true, false, Replace, ST, false),
            (true, false, Replace, TS, false),
            (false, true, Delete, ST, true),
            (false, true, Insert, TS, true),
            (false, true, Insert, ST, false),
            (false, true, Delete, TS, false),
            (false, true, Replace, ST, false),
            (false, true, Replace, TS, false),
            (true, true, Replace, ST, true),
            (true, true, Replace, TS, true),
            (true, true, Insert, ST, false),
            (true, true, Insert, TS, false),
            (true, true, Delete, ST, false),
            (true, true, Delete, TS, false),
        ];
        for (s, t, k, d, want) in cases {
            assert_eq!(kind_fits(k, d, s, t), want, "{s} {t} {k:?} {d:?}");
        }
    }

    #[test]
    fn plan_write_insert_keeps_the_origin_id_by_default() {
        let o = doc! {"_id": 1, "a": 2};
        let op = plan_write(ApplyKind::Insert, Some(&o), None, IdPolicy::default()).unwrap();
        assert_eq!(op, WriteOp::Insert { doc: o });
    }

    #[test]
    fn plan_write_insert_drops_it_with_generate() {
        let o = doc! {"_id": 1, "a": 2};
        let op = plan_write(ApplyKind::Insert, Some(&o), None, IdPolicy::Generate).unwrap();
        assert_eq!(op, WriteOp::Insert { doc: doc! {"a": 2} });
    }

    #[test]
    fn plan_write_replace_carries_the_destination_id_and_origin_fields() {
        let o = doc! {"n": 2, "_id": 1, "email": "a"};
        let d = doc! {"_id": 9, "email": "a", "n": 1};
        let op = plan_write(ApplyKind::Replace, Some(&o), Some(&d), IdPolicy::Keep).unwrap();
        let WriteOp::Replace { id, doc } = op else {
            panic!("expected Replace")
        };
        assert_eq!(id, Bson::Int32(9));
        assert_eq!(doc, doc! {"_id": 9, "n": 2, "email": "a"});
        let keys: Vec<_> = doc.keys().collect();
        assert_eq!(keys, ["_id", "n", "email"]);
    }

    #[test]
    fn plan_write_replace_works_for_every_id_type() {
        let ids = [
            Bson::ObjectId(ObjectId::new()),
            Bson::String("k".into()),
            Bson::Int64(7),
            Bson::Document(doc! {"a": 1}),
        ];
        for id in ids {
            let o = doc! {"_id": 1, "x": 1};
            let d = doc! {"_id": id.clone(), "x": 0};
            let op = plan_write(ApplyKind::Replace, Some(&o), Some(&d), IdPolicy::Keep).unwrap();
            let expected = doc! {"_id": id.clone(), "x": 1};
            assert_eq!(op, WriteOp::Replace { id, doc: expected });
        }
    }

    #[test]
    fn plan_write_delete_uses_the_destination_id() {
        let d = doc! {"_id": "z", "x": 0};
        let o = doc! {"_id": "other", "x": 1};
        let op = plan_write(ApplyKind::Delete, Some(&o), Some(&d), IdPolicy::Keep).unwrap();
        assert_eq!(
            op,
            WriteOp::Delete {
                id: Bson::String("z".into())
            }
        );
    }

    #[test]
    fn plan_write_refuses_a_missing_destination_for_replace_and_delete() {
        let o = doc! {"_id": 1};
        let no_id = doc! {"x": 1};
        for kind in [ApplyKind::Replace, ApplyKind::Delete] {
            for dest in [None, Some(&no_id)] {
                let r = plan_write(kind, Some(&o), dest, IdPolicy::Keep);
                assert_eq!(r, Err(invalid()), "{kind:?}");
            }
        }
        let r = plan_write(ApplyKind::Insert, None, None, IdPolicy::Keep);
        assert!(matches!(r, Err(CoreError::InvalidDocument(_))));
        let r = plan_write(ApplyKind::Replace, None, Some(&o), IdPolicy::Keep);
        assert!(matches!(r, Err(CoreError::InvalidDocument(_))));
    }

    #[test]
    fn report_counts_each_outcome_once_and_caps_failures() {
        let mut r = ApplyReport::default();
        let key = KeyTuple(vec![Bson::Int32(1)]);
        r.record(&key, &OpOutcome::Applied);
        r.record(&key, &OpOutcome::Skipped(SkipReason::Stale));
        r.record(&key, &OpOutcome::Skipped(SkipReason::NotPlanned));
        for _ in 0..MAX_REPORTED_FAILURES {
            r.record(&key, &OpOutcome::Failed(CoreError::NotFound));
        }
        assert_eq!(r.failures.len(), MAX_REPORTED_FAILURES);
        assert!(!r.more_failures);
        for _ in 0..50 {
            r.record(&key, &OpOutcome::Failed(CoreError::NotFound));
        }
        assert_eq!((r.applied, r.stale, r.not_planned, r.not_run), (1, 1, 1, 0));
        assert_eq!(r.failed, 150);
        assert_eq!(r.failures.len(), MAX_REPORTED_FAILURES);
        assert!(
            r.failures
                .iter()
                .all(|f| f.error == CoreError::NotFound && f.key == key)
        );
        assert!(r.more_failures);
    }

    #[test]
    fn map_write_failure_text_carries_no_values() {
        assert_eq!(
            map_write_failure(WriteFailure::Missing),
            CoreError::NotFound
        );
        assert_eq!(
            map_write_failure(WriteFailure::DuplicateKey),
            CoreError::Database("duplicate key".into())
        );
        let e = map_write_failure(WriteFailure::Rejected(121));
        assert_eq!(e, CoreError::ApplyWriteRejected);
        assert!(!e.to_string().contains("121"));
    }
}
