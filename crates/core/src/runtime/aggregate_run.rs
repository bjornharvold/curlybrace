//! The handler of `Command::AggregateRun`: run the write pipeline, then count its target.

use bson::Document;

use super::{Event, SessionId};
use crate::error::Result;
use crate::pipeline::{AGGREGATE_RUN_MAX_TIME_MS, run_target};
use crate::store::{DocumentStore, Namespace};

/// The collection the last stage writes to; the source database when the stage names none.
/// A pipeline whose target cannot be resolved answers the source itself: the store refuses such
/// a pipeline before anything runs, so the name only labels that refusal.
pub(super) fn target_namespace(source: &Namespace, pipeline: &[Document]) -> Namespace {
    match run_target(pipeline) {
        Some((_, target)) => Namespace::new(
            target.db.unwrap_or_else(|| source.db.clone()),
            target.collection,
        ),
        None => source.clone(),
    }
}

/// Runs the pipeline and counts the target. A lost connection fails the command, so the runtime
/// settles it by a ping (an interrupted write is then an unknown outcome). Every other failure
/// of the pipeline, the store's shape refusal included, is the outcome, shown inline.
pub(super) async fn run(
    session: SessionId,
    store: &dyn DocumentStore,
    ns: Namespace,
    pipeline: Vec<Document>,
) -> Result<Event> {
    let target = target_namespace(&ns, &pipeline);
    let max_time = std::time::Duration::from_millis(AGGREGATE_RUN_MAX_TIME_MS);
    let outcome = match store.aggregate_write(&ns, pipeline, max_time).await {
        Ok(()) => store.estimated_count(&target).await,
        Err(error) => Err(error),
    };
    match outcome {
        Err(error) if super::handlers::is_connection_error(&error) => Err(error),
        outcome => Ok(Event::AggregateRan {
            session,
            ns,
            target,
            outcome,
        }),
    }
}
