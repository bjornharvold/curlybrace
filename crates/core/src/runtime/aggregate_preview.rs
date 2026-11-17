//! The handler of `Command::AggregatePreview`: run the pipeline as a bounded read with the options
//! the preview sends (see `PipelineOptions::applied`) and answer the documents under the
//! request's number.

use bson::Document;

use super::handlers::{COMMAND_TIMEOUT_MESSAGE, is_connection_error};
use super::{Command, Event, SessionId};
use crate::error::{CoreError, Result};
use crate::pipeline::{Operation, PipelineOptions};
use crate::store::{DocumentStore, Namespace};

/// Previews `pipeline`. Every failure of the pipeline, the store's write-stage refusal, a hint
/// that names no index and a read preference nobody satisfies included, is the outcome: the
/// preview pane shows it inline, never as a banner. A lost connection is not the pipeline's
/// failure: it fails the command like any read's, so `run` settles an interruption by a ping and
/// the UI learns the server is gone.
pub(super) async fn run(
    session: SessionId,
    store: &dyn DocumentStore,
    request: u64,
    ns: Namespace,
    pipeline: Vec<Document>,
    options: &PipelineOptions,
) -> Result<Event> {
    let applied = options.applied(Operation::Preview);
    let outcome = match store.aggregate_preview(&ns, pipeline, &applied).await {
        Err(error) if is_connection_error(&error) => return Err(error),
        other => other,
    };
    Ok(Event::AggregatePreviewed {
        session,
        request,
        outcome,
    })
}

/// The answer to a preview that outran the client bound: a slow pipeline says nothing about the
/// server, so it is an outcome, never a ping-settled failure. `None` for any other command.
pub(super) fn timeout_answer(session: SessionId, command: &Command) -> Option<Event> {
    match command {
        Command::AggregatePreview { request, .. } => Some(Event::AggregatePreviewed {
            session,
            request: *request,
            outcome: Err(CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into())),
        }),
        _ => None,
    }
}
