//! The handler of `Command::AggregateExplain`: explain the pipeline on the store, answering
//! under the request's number.

use std::sync::Arc;

use super::aggregate_record::record_pipeline;
use super::handlers::{COMMAND_TIMEOUT_MESSAGE, is_connection_error};
use super::{Command, Event, SessionId};
use crate::error::{CoreError, Result};
use crate::history::HistoryDb;
use crate::pipeline::Operation;
use crate::store::DocumentStore;

/// Explains `stages`. Every failure of the pipeline, the store's write-stage refusal included, is
/// the outcome: the explain section shows it inline, never as a banner. A lost connection is not
/// the pipeline's failure: it fails the command like any read's, so `run` settles an interruption
/// by a ping and the UI learns the server is gone. A successful explain also records the
/// pipeline's shape in `history` (see [`record_pipeline`]), beside the answer, never inside it.
pub(super) async fn run(
    session: SessionId,
    store: &dyn DocumentStore,
    history: Option<&Arc<HistoryDb>>,
    command: Command,
) -> Result<Event> {
    let Command::AggregateExplain {
        request,
        ns,
        stages,
        verbosity,
        options,
        ..
    } = command
    else {
        return Err(CoreError::Unsupported("not an aggregate explain".into()));
    };
    let options = &options;
    let applied = options.applied(Operation::Explain);
    // The stage list is copied only when there is a history to record into.
    let to_record = history.map(|history| (history, stages.clone()));
    let outcome = match store
        .aggregate_explain(&ns, stages, verbosity, &applied)
        .await
    {
        Err(error) if is_connection_error(&error) => return Err(error),
        other => other,
    };
    // Only a successful explain says anything about the pipeline; recording is detached.
    if let (Ok(explain), Some((history, stages))) = (&outcome, &to_record) {
        let now = crate::clock::now_secs;
        record_pipeline(
            history,
            session,
            &ns,
            stages,
            options,
            &explain.document,
            &now,
        );
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
