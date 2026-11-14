//! The handler of `Command::AggregateRun`: run the write pipeline, then count its target.

use bson::Document;

use super::handlers::{COMMAND_TIMEOUT_MESSAGE, is_connection_error};
use super::{Event, SessionId};
use crate::error::{CoreError, Result};
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

/// A write that outran the server's own time limit was cut off part-way: a `$merge` is not atomic
/// and `$out` may already have swapped, so what it left is unknown, not a plain failure.
fn expiry_is_unknown(error: CoreError) -> CoreError {
    match error {
        CoreError::Database(text) if text == COMMAND_TIMEOUT_MESSAGE => {
            CoreError::WriteOutcomeUnknown
        }
        other => other,
    }
}

/// Runs the pipeline and counts the target. A lost connection during the write fails the command,
/// so the runtime settles it by a ping (an interrupted write is then an unknown outcome). Every
/// other failure of the pipeline, the store's shape refusal and a target in a server database
/// included, is the outcome, shown inline. A run that succeeded stays a success when only the
/// count of its target cannot be read (`Ok(None)`), a lost connection included: the next command
/// detects that.
pub(super) async fn run(
    session: SessionId,
    store: &dyn DocumentStore,
    ns: Namespace,
    pipeline: Vec<Document>,
    run: u64,
) -> Result<Event> {
    let target = target_namespace(&ns, &pipeline);
    let max_time = std::time::Duration::from_millis(AGGREGATE_RUN_MAX_TIME_MS);
    let outcome = match crate::admin::RESERVED_DATABASES.contains(&target.db.as_str()) {
        true => Err(CoreError::ReservedDatabase),
        false => match store.aggregate_write(&ns, pipeline, max_time).await {
            Ok(()) => Ok(store.estimated_count(&target).await.ok()),
            Err(error) if is_connection_error(&error) => return Err(error),
            Err(error) => Err(expiry_is_unknown(error)),
        },
    };
    Ok(Event::AggregateRan {
        session,
        run,
        ns,
        target,
        outcome,
    })
}
