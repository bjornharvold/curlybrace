//! Recording the shape of a typed query, with its verdict, when an explain of it succeeded.
//!
//! It runs beside the answer, never inside it: the recording is a detached task on the blocking
//! pool, so a slow, busy or broken history cannot delay or change the verdict event. A failure is
//! logged by its variant name only (the shape is literal-free, but nothing about the query is
//! ever logged).

use std::sync::Arc;

use super::SessionId;
use super::profiler::variant_of;
use crate::clock;
use crate::explain::VerdictKind;
use crate::history::{HistoryDb, ShapeKey, ShapeObservation, ShapeSource, TYPED_HOST, is_miss};
use crate::shape::from_query;
use crate::store::{Namespace, Query};

/// Builds the one observation a typed explain stands for: a single execution with its verdict.
/// Typed rows are not node-specific, so their host is [`TYPED_HOST`].
pub(super) fn typed_observation(
    connection: SessionId,
    ns: &Namespace,
    query: &Query,
    kind: VerdictKind,
    at_secs: i64,
) -> ShapeObservation {
    let shape = from_query(query);
    ShapeObservation {
        key: ShapeKey {
            connection,
            host: TYPED_HOST.into(),
            db: ns.db.clone(),
            collection: ns.collection.clone(),
            hash: shape.hash(),
            source: ShapeSource::Typed,
        },
        shape_json: shape.json(),
        at_secs,
        executions: 1,
        misses: u64::from(is_miss(kind)),
        total_millis: 0,
        max_millis: 0,
        worst: kind,
        gap_secs: 0,
    }
}

/// Records the typed shape in the background. A purged connection is skipped by the history
/// itself; any other failure is logged and otherwise ignored.
pub(super) fn record_typed(
    history: &Arc<HistoryDb>,
    connection: SessionId,
    ns: &Namespace,
    query: &Query,
    kind: VerdictKind,
) {
    let row = typed_observation(connection, ns, query, kind, clock::now_secs());
    let history = Arc::clone(history);
    // Detached on purpose: nothing waits for the recording.
    drop(tokio::task::spawn_blocking(move || {
        if let Err(error) = history.record_shapes(&[row]) {
            log::warn!("typed query shape not recorded: {}", variant_of(&error));
        }
    }));
}
