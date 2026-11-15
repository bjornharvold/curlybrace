//! The handler of `Command::AggregateExplain`: explain the pipeline on the store, answering
//! under the request's number.

use bson::Document;

use super::handlers::{COMMAND_TIMEOUT_MESSAGE, is_connection_error};
use super::{Command, Event, SessionId};
use crate::error::{CoreError, Result};
use crate::explain::{AGGREGATE_EXPLAIN_MAX_TIME_MS, ExplainVerbosity};
use crate::store::{DocumentStore, Namespace};

/// Explains `stages`. Every failure of the pipeline, the store's write-stage refusal included, is
/// the outcome: the explain section shows it inline, never as a banner. A lost connection is not
/// the pipeline's failure: it fails the command like any read's, so `run` settles an interruption
/// by a ping and the UI learns the server is gone.
pub(super) async fn run(
    session: SessionId,
    store: &dyn DocumentStore,
    request: u64,
    ns: Namespace,
    stages: Vec<Document>,
    verbosity: ExplainVerbosity,
) -> Result<Event> {
    let max_time = std::time::Duration::from_millis(AGGREGATE_EXPLAIN_MAX_TIME_MS);
    let outcome = match store
        .aggregate_explain(&ns, stages, verbosity, max_time)
        .await
    {
        Err(error) if is_connection_error(&error) => return Err(error),
        other => other,
    };
    Ok(Event::AggregateExplained {
        session,
        request,
        ns,
        verbosity,
        outcome,
    })
}

/// The answer to an explain that outran the client bound: a slow explain (`executionStats` runs
/// the pipeline) says nothing about the server, so it is an outcome, never a ping-settled
/// failure. `None` for any other command.
pub(super) fn timeout_answer(session: SessionId, command: &Command) -> Option<Event> {
    match command {
        Command::AggregateExplain {
            request,
            ns,
            verbosity,
            ..
        } => Some(Event::AggregateExplained {
            session,
            request: *request,
            ns: ns.clone(),
            verbosity: *verbosity,
            outcome: Err(CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into())),
        }),
        _ => None,
    }
}
