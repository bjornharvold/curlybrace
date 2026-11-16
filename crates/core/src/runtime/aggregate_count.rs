//! The handler of `Command::AggregateCount`: run the counting pipeline as a bounded read and
//! answer the number under the request's number. The pipeline arrives built (see
//! `pipeline::count_pipeline`, which carries the cap); the store refuses a write stage.

use bson::Document;

use super::handlers::{COMMAND_TIMEOUT_MESSAGE, is_connection_error};
use super::{Command, Event, SessionId};
use crate::error::{CoreError, Result};
use crate::pipeline::{Operation, PipelineOptions, count_of};
use crate::store::{DocumentStore, Namespace};

/// Counts with `pipeline`. Every failure of the pipeline (the store's write-stage refusal, a
/// server timeout, an unreadable answer) is the outcome: the stage card shows it, never a banner.
/// A lost connection is not the pipeline's failure: it fails the command like any read's, so
/// `run` settles an interruption by a ping and the UI learns the server is gone.
pub(super) async fn run(
    session: SessionId,
    store: &dyn DocumentStore,
    request: u64,
    ns: Namespace,
    stage_index: usize,
    pipeline: Vec<Document>,
    options: &PipelineOptions,
) -> Result<Event> {
    let applied = options.applied(Operation::Count);
    let outcome = match store.aggregate_preview(&ns, pipeline, &applied).await {
        Err(error) if is_connection_error(&error) => return Err(error),
        other => other.and_then(|docs| count_of(&docs)),
    };
    Ok(Event::AggregateCounted {
        session,
        request,
        ns,
        stage_index,
        outcome,
    })
}

/// The answer to a count that outran the client bound: a slow count says nothing about the
/// server, so it is an outcome, never a ping-settled failure. `None` for any other command.
pub(super) fn timeout_answer(session: SessionId, command: &Command) -> Option<Event> {
    match command {
        Command::AggregateCount {
            request,
            ns,
            stage_index,
            ..
        } => Some(Event::AggregateCounted {
            session,
            request: *request,
            ns: ns.clone(),
            stage_index: *stage_index,
            outcome: Err(CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into())),
        }),
        _ => None,
    }
}
