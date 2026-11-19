//! The handler of `Command::AggregateExplain`: explain the pipeline on the store, answering
//! under the request's number.

use std::sync::Arc;

use bson::Document;

use super::aggregate_record::{record_pipeline, shape_to_record};
use super::handlers::{COMMAND_TIMEOUT_MESSAGE, is_connection_error};
use super::{Command, Event, SessionId};
use crate::error::{CoreError, Result};
use crate::explain::ExplainVerbosity;
use crate::history::HistoryDb;
use crate::pipeline::{Operation, PipelineOptions};
use crate::store::{DocumentStore, Namespace};

/// Explains `stages`. Every failure of the pipeline, the store's write-stage refusal included, is
/// the outcome: the explain section shows it inline, never as a banner. A lost connection is not
/// the pipeline's failure: it fails the command like any read's, so `run` settles an interruption
/// by a ping and the UI learns the server is gone. A successful explain also records the
/// pipeline's shape in `history` (see [`record_pipeline`]), beside the answer, never inside it.
#[allow(clippy::too_many_arguments)] // the fields of the command, plus where it runs and records
pub(super) async fn run(
    session: SessionId,
    store: &dyn DocumentStore,
    history: Option<&Arc<HistoryDb>>,
    request: u64,
    ns: Namespace,
    stages: Vec<Document>,
    verbosity: ExplainVerbosity,
    options: &PipelineOptions,
) -> Result<Event> {
    let applied = options.applied(Operation::Explain);
    // Built before the stages move to the store, and only when there is a history to record into.
    let shape = history.and_then(|_| shape_to_record(&ns, &stages, options));
    let outcome = match store
        .aggregate_explain(&ns, stages, verbosity, &applied)
        .await
    {
        Err(error) if is_connection_error(&error) => return Err(error),
        other => other,
    };
    // Only a successful explain says anything about the pipeline; recording is detached.
    if let (Ok(explain), Some(history), Some(shape)) = (&outcome, history, &shape) {
        let now = crate::clock::now_secs;
        record_pipeline(history, session, &ns, shape, &explain.document, &now);
    }
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
