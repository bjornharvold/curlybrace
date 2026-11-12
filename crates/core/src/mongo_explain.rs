//! `explain` of the find a query sends, for [`crate::mongo_store::MongoStore::explain_find`].
//! The command is built by a pure function so tests can check where each part of the query
//! lands; running it is a single `run_command` plus one retry when the time limit expires.

use bson::{Document, doc};
use mongodb::Database;
use mongodb::action::Action;
use mongodb::error::ErrorKind;
use mongodb::options::SelectionCriteria;

use crate::error::Result;
use crate::mongo_store::{fetch_limit, skip_as_i64};
use crate::store::{Explain, Namespace, Query};

/// Server-side time limit of one explain, so a heavy scan cannot hold the connection.
pub(crate) const EXPLAIN_MAX_TIME_MS: i64 = 2000;

/// Server error code `MaxTimeMSExpired`.
const CODE_MAX_TIME_MS_EXPIRED: i32 = 50;

/// How much the server does for an explain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verbosity {
    /// Runs the query and reports the counters.
    ExecutionStats,
    /// Plans only; never runs the query.
    QueryPlanner,
}

impl Verbosity {
    fn as_str(self) -> &'static str {
        match self {
            Self::ExecutionStats => "executionStats",
            Self::QueryPlanner => "queryPlanner",
        }
    }
}

/// The `explain` command for `q` on `ns`: the same find the query bar sends (filter, sort,
/// projection, skip, and the `limit + 1` that `find` fetches), wrapped with the verbosity and
/// the time limit.
pub(crate) fn explain_command(ns: &Namespace, q: &Query, verbosity: Verbosity) -> Result<Document> {
    let skip = skip_as_i64(q)?;
    let mut find = doc! { "find": ns.collection.clone(), "filter": q.filter.clone() };
    if let Some(sort) = &q.sort {
        find.insert("sort", sort.clone());
    }
    if let Some(projection) = &q.projection {
        find.insert("projection", projection.clone());
    }
    if skip > 0 {
        find.insert("skip", i64::try_from(skip).unwrap_or(i64::MAX));
    }
    if let Some(limit) = fetch_limit(q) {
        find.insert("limit", limit);
    }
    Ok(doc! {
        "explain": find,
        "verbosity": verbosity.as_str(),
        "maxTimeMS": EXPLAIN_MAX_TIME_MS,
    })
}

/// Whether a server error code is the time limit expiring.
pub(crate) fn is_time_limit(code: Option<i32>) -> bool {
    code == Some(CODE_MAX_TIME_MS_EXPIRED)
}

fn timed_out(e: &mongodb::error::Error) -> bool {
    is_time_limit(match e.kind.as_ref() {
        ErrorKind::Command(c) => Some(c.code),
        _ => None,
    })
}

/// Where an explain runs: the connection's read preference, like the `find` it describes, so a
/// secondary-reading connection neither loads the primary nor judges another node's plan.
pub(crate) fn explain_criteria(db: &Database) -> Option<SelectionCriteria> {
    db.selection_criteria().cloned()
}

async fn run_explain(db: &Database, command: Document) -> mongodb::error::Result<Document> {
    db.run_command(command)
        .optional(explain_criteria(db), |action, c| {
            action.selection_criteria(c)
        })
        .await
}

/// Explains `q` with execution statistics; when the time limit expires, falls back to the plan
/// alone (`plan_only`). Any other error is classified like every store error.
pub(crate) async fn explain_find(db: &Database, ns: &Namespace, q: &Query) -> Result<Explain> {
    let stats = explain_command(ns, q, Verbosity::ExecutionStats)?;
    match run_explain(db, stats).await {
        Ok(document) => Ok(Explain {
            document,
            plan_only: false,
        }),
        Err(e) if timed_out(&e) => {
            let plan = explain_command(ns, q, Verbosity::QueryPlanner)?;
            let document = run_explain(db, plan)
                .await
                .map_err(|e| crate::driver_error::classify_driver_error(&e))?;
            Ok(Explain {
                document,
                plan_only: true,
            })
        }
        Err(e) => Err(crate::driver_error::classify_driver_error(&e)),
    }
}

/// Explains `q` with the plan alone: the server plans the find and never runs it.
pub(crate) async fn explain_plan(db: &Database, ns: &Namespace, q: &Query) -> Result<Explain> {
    let plan = explain_command(ns, q, Verbosity::QueryPlanner)?;
    let document = run_explain(db, plan)
        .await
        .map_err(|e| crate::driver_error::classify_driver_error(&e))?;
    Ok(Explain {
        document,
        plan_only: true,
    })
}

#[cfg(test)]
#[path = "mongo_explain_tests.rs"]
mod tests;
