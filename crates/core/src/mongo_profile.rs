//! The profiler through the server: `profile` commands and reads of `system.profile`, for
//! [`crate::mongo_store::MongoStore`]. Commands, filters and reply parsing are pure functions so
//! tests can check them; the runners are a few driver calls each.
//!
//! Reads follow the connection's read preference, like the explains and finds they describe. A
//! secondary's profiler records only the operations that node served itself, so reading from one
//! shows that node's traffic: expected, and why [`ProfileRead::host`] names the node. The level
//! change always goes to the primary.

use bson::{Bson, DateTime, Document, doc};
use futures::TryStreamExt;
use mongodb::Database;
use mongodb::action::Action;
use mongodb::options::{ReadPreference, SelectionCriteria};

use crate::driver_error::classify_driver_error;
use crate::error::{CoreError, Result};
use crate::profile::{ProfileEntry, parse_entry};
use crate::store::{ProfileRead, ProfilingState, Topology};

/// The most entries one read returns.
pub(crate) const PROFILE_READ_LIMIT: i64 = 1000;

const PROFILE_COLLECTION: &str = "system.profile";
const MILLIS_PER_SEC: i64 = 1000;

/// `{profile: -1}`: reports the setting without changing it.
pub(crate) fn profile_level_command() -> Document {
    doc! { "profile": -1 }
}

/// `{profile: level}` for a level of 0, 1 or 2; anything else is refused before it is sent.
pub(crate) fn set_level_command(level: i32) -> Result<Document> {
    if !(0..=2).contains(&level) {
        return Err(CoreError::Unsupported(format!(
            "profiling level {level} is not 0, 1 or 2"
        )));
    }
    Ok(doc! { "profile": level })
}

/// The filter, sort and cap of one read of `system.profile`.
#[derive(Debug, PartialEq)]
pub(crate) struct ProfileQuery {
    pub filter: Document,
    pub sort: Document,
    pub limit: i64,
}

/// The entries newer than `since_ts_secs`, oldest first.
pub(crate) fn since_query(since_ts_secs: i64) -> ProfileQuery {
    let since = DateTime::from_millis(since_ts_secs.saturating_mul(MILLIS_PER_SEC));
    ProfileQuery {
        filter: doc! { "ts": { "$gt": since } },
        sort: doc! { "ts": 1 },
        limit: PROFILE_READ_LIMIT,
    }
}

/// The single oldest entry, to tell whether the capped collection rolled entries off.
fn oldest_query() -> ProfileQuery {
    ProfileQuery {
        filter: Document::new(),
        sort: doc! { "ts": 1 },
        limit: 1,
    }
}

/// The profiler is not reachable through mongos: each shard has its own.
pub(crate) fn mongos_decision(topology: Option<Topology>) -> Result<()> {
    match topology {
        Some(Topology::Sharded) => Err(CoreError::Unsupported(
            "profiler not available through mongos".into(),
        )),
        _ => Ok(()),
    }
}

fn number(value: Option<&Bson>) -> Option<i64> {
    match value? {
        Bson::Int32(n) => Some(i64::from(*n)),
        Bson::Int64(n) => Some(*n),
        Bson::Double(n) if n.is_finite() => Some(*n as i64),
        _ => None,
    }
}

/// Reads the reply of `{profile: ...}`: `was` is the level before the command, `slowms` the
/// threshold.
pub(crate) fn parse_profiling_state(reply: &Document) -> Result<ProfilingState> {
    let level = number(reply.get("was"))
        .and_then(|n| i32::try_from(n).ok())
        .ok_or_else(|| CoreError::Database("profile reply has no level".into()))?;
    let slow_ms = number(reply.get("slowms")).unwrap_or_default();
    Ok(ProfilingState { level, slow_ms })
}

/// The node a `hello` reply came from; empty when it does not say.
pub(crate) fn hello_host(reply: &Document) -> String {
    reply.get_str("me").unwrap_or_default().to_owned()
}

/// Parses the documents; returns the entries and how many could not be parsed.
pub(crate) fn parse_entries(docs: &[Document]) -> (Vec<ProfileEntry>, usize) {
    let entries: Vec<ProfileEntry> = docs.iter().filter_map(|d| parse_entry(d).ok()).collect();
    let skipped = docs.len() - entries.len();
    (entries, skipped)
}

/// The `ts` (seconds) of the first document, which is the oldest when sorted by `ts`.
pub(crate) fn oldest_ts_secs(sorted_by_ts: &[Document]) -> Option<i64> {
    parse_entry(sorted_by_ts.first()?).ok().map(|e| e.ts_secs)
}

fn db_err(e: mongodb::error::Error) -> CoreError {
    classify_driver_error(&e)
}

async fn run(
    db: &Database,
    command: Document,
    criteria: Option<SelectionCriteria>,
) -> Result<Document> {
    db.run_command(command)
        .optional(criteria, |action, c| action.selection_criteria(c))
        .await
        .map_err(db_err)
}

/// Where reads run: the connection's read preference.
fn read_criteria(db: &Database) -> Option<SelectionCriteria> {
    db.selection_criteria().cloned()
}

fn primary() -> Option<SelectionCriteria> {
    Some(SelectionCriteria::ReadPreference(ReadPreference::Primary))
}

/// The setting as the node the connection reads from reports it.
pub(crate) async fn profiling_level(db: &Database) -> Result<ProfilingState> {
    let reply = run(db, profile_level_command(), read_criteria(db)).await?;
    parse_profiling_state(&reply)
}

/// Sets the level on the primary, then reads the setting back from the primary.
pub(crate) async fn set_profiling_level(db: &Database, level: i32) -> Result<ProfilingState> {
    run(db, set_level_command(level)?, primary()).await?;
    let reply = run(db, profile_level_command(), primary()).await?;
    parse_profiling_state(&reply)
}

async fn find_profile(db: &Database, query: ProfileQuery) -> Result<Vec<Document>> {
    let cursor = db
        .collection::<Document>(PROFILE_COLLECTION)
        .find(query.filter)
        .sort(query.sort)
        .limit(query.limit)
        .await
        .map_err(db_err)?;
    cursor.try_collect().await.map_err(db_err)
}

/// Reads the entries newer than `since_ts_secs`. A missing collection finds nothing. Only the
/// count of unparseable entries is logged: an entry holds the query's literal values.
pub(crate) async fn read_profile(db: &Database, since_ts_secs: i64) -> Result<ProfileRead> {
    let host = hello_host(&run(db, doc! { "hello": 1 }, read_criteria(db)).await?);
    let docs = find_profile(db, since_query(since_ts_secs)).await?;
    let oldest = find_profile(db, oldest_query()).await?;
    let (entries, skipped) = parse_entries(&docs);
    if skipped > 0 {
        log::warn!("skipped {skipped} unparseable profile entries");
    }
    Ok(ProfileRead {
        entries,
        oldest_ts_secs: oldest_ts_secs(&oldest),
        host,
    })
}

#[cfg(test)]
#[path = "mongo_profile_tests.rs"]
mod tests;
