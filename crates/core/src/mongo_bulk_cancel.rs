//! Best-effort cancel of a running bulk write: find the operation by the run id it was tagged
//! with (`comment`), then `killOp` it. The killed write fails with `Interrupted`, which
//! `mongo_bulk` reports as `BulkOutcome::Unknown { cancelled: true }`. Server text never reaches
//! an error: it can quote document values.

use bson::{Bson, Document, doc};
use futures::TryStreamExt;
use mongodb::Client;
use mongodb::options::{ReadPreference, SelectionCriteria};

use crate::bulk::{BulkRunId, CancelEffect};
use crate::driver_error::{classify_driver_error, server_error_code};
use crate::error::{CoreError, Result};

/// `Unauthorized`, `CommandNotFound`, `CommandNotSupported`, `UnrecognizedStage` (no
/// `$currentOp`) and Atlas' `AtlasError` (a shared tier refuses it): the user cannot inspect or
/// kill operations here.
const NOT_PERMITTED_CODES: [i32; 5] = [13, 59, 115, 40324, 8000];

/// Kills the operation tagged with `run`: `Stopped` when one was found, `NothingToStop` when no
/// operation carries that id (it finished, or the server has not registered it yet: the command
/// may still wait for a connection). Without the privilege or support to do it, `Unsupported`.
pub(crate) async fn cancel_bulk(client: &Client, run: BulkRunId) -> Result<CancelEffect> {
    let admin = client.database("admin");
    let ops = find_ops(&admin, run).await.map_err(fail)?;
    if ops.is_empty() {
        return Ok(CancelEffect::NothingToStop);
    }
    for opid in ops {
        // The op may finish between the lookup and the kill; the kill is then a no-op.
        admin
            .run_command(doc! { "killOp": 1, "op": opid })
            .await
            .map_err(fail)?;
    }
    Ok(CancelEffect::Stopped)
}

/// The `opid` of every operation of this connection's user whose `command.comment` is `run`.
async fn find_ops(admin: &mongodb::Database, run: BulkRunId) -> mongodb::error::Result<Vec<Bson>> {
    let pipeline = vec![
        doc! { "$currentOp": {} },
        doc! { "$match": { "command.comment": run.0.to_string() } },
    ];
    let found: Vec<Document> = admin
        .aggregate(pipeline)
        .selection_criteria(primary())
        .await?
        .try_collect()
        .await?;
    Ok(found
        .iter()
        .filter_map(|op| op.get("opid").cloned())
        .collect())
}

/// The write and `killOp` go to the primary, so the lookup must too: on a secondary (a
/// `secondaryPreferred` client) it would find nothing and the cancel would silently do nothing.
pub(crate) fn primary() -> SelectionCriteria {
    SelectionCriteria::ReadPreference(ReadPreference::Primary)
}

/// Server errors map by code alone (their text can quote documents). Other failures keep the
/// driver's connection classification (unreachable, interrupted); a generic one stays static.
fn fail(e: mongodb::error::Error) -> CoreError {
    let code = server_error_code(&e);
    if code.is_some() {
        return cancel_failure(code);
    }
    match classify_driver_error(&e) {
        CoreError::Database(_) => cancel_failure(None),
        other => other,
    }
}

/// The error for a failed lookup or kill, from the server error code alone.
pub(crate) fn cancel_failure(code: Option<i32>) -> CoreError {
    match code {
        Some(c) if NOT_PERMITTED_CODES.contains(&c) => CoreError::Unsupported("cancel".into()),
        _ => CoreError::Database("cancel_failed".into()),
    }
}
