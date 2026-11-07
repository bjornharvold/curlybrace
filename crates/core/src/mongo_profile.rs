//! The profiler through the server: `profile` commands and reads of `system.profile`, for
//! [`crate::mongo_store::MongoStore`]. Commands, filters and reply parsing are pure functions so
//! tests can check them; the runners are a few driver calls each.
//!
//! The profiler is read from the node where its level is set: the primary. The level is per node
//! and not replicated, so a read that followed a `secondaryPreferred` read preference would ask a
//! secondary whose profiler is off while the level just set on the primary shows 2, and the
//! feature would look dead. Every call here (the level, `hello`, both `system.profile` finds)
//! therefore goes to the primary whatever the read preference; on a direct connection the driver
//! selects the single server. Reading a secondary's profile is not supported. The explains of
//! release A keep following the read preference. [`ProfileRead::host`] names the node read.

use bson::{Bson, DateTime, Document, doc};
use futures::TryStreamExt;
use mongodb::Database;
use mongodb::action::Action;
use mongodb::options::{ReadPreference, SelectionCriteria};

use crate::driver_error::classify_driver_error;
use crate::error::{CoreError, Result};
use crate::profile::{ProfileEntry, parse_entry};
use crate::store::{
    PROFILE_READ_LIMIT as PAGE_LIMIT, PROFILER_MONGOS_MESSAGE, ProfileRead, ProfilingState,
    Topology,
};

/// The most entries one read returns.
pub(crate) const PROFILE_READ_LIMIT: i64 = PAGE_LIMIT as i64;

const PROFILE_COLLECTION: &str = "system.profile";

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

/// The entries strictly newer than `since_ts_millis`, oldest first.
pub(crate) fn since_query(since_ts_millis: i64) -> ProfileQuery {
    let since = DateTime::from_millis(since_ts_millis);
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
        Some(Topology::Sharded) => Err(CoreError::Unsupported(PROFILER_MONGOS_MESSAGE.into())),
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

/// The node a `hello` reply came from. Empty when `me` is missing (a standalone mongod, a
/// mongos): an unknown node, which the poller keys as host `""`.
pub(crate) fn hello_host(reply: &Document) -> String {
    reply.get_str("me").unwrap_or_default().to_owned()
}

/// The state after `{profile: level}`: the reply's `was` is the PREVIOUS level, so the new
/// level is the one asked for; the reply's `slowms` is the threshold.
pub(crate) fn parse_set_reply(reply: &Document, level: i32) -> ProfilingState {
    ProfilingState {
        level,
        slow_ms: number(reply.get("slowms")).unwrap_or_default(),
    }
}

/// One page of `system.profile` documents, parsed.
#[derive(Debug)]
pub(crate) struct PageSummary {
    pub entries: Vec<ProfileEntry>,
    pub skipped: u64,
    /// The newest raw `ts` (ms) of the page, whether or not the document could be parsed.
    pub newest_ts_millis: Option<i64>,
}

/// The raw `ts` of a document, when it is a DateTime.
fn raw_ts_millis(doc: &Document) -> Option<i64> {
    match doc.get("ts") {
        Some(Bson::DateTime(ts)) => Some(ts.timestamp_millis()),
        _ => None,
    }
}

/// Parses the page; unparseable documents are counted, never dropped from the newest `ts`.
pub(crate) fn summarize_page(docs: &[Document]) -> PageSummary {
    let entries: Vec<ProfileEntry> = docs.iter().filter_map(|d| parse_entry(d).ok()).collect();
    PageSummary {
        skipped: (docs.len() - entries.len()) as u64,
        newest_ts_millis: docs.iter().filter_map(raw_ts_millis).max(),
        entries,
    }
}

/// The raw `ts` (ms) of the first document, which is the oldest when sorted by `ts`; found even
/// when the document cannot be parsed.
pub(crate) fn oldest_ts_millis(sorted_by_ts: &[Document]) -> Option<i64> {
    raw_ts_millis(sorted_by_ts.first()?)
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

/// Where every profiler call goes: the primary (see the module doc).
pub(crate) fn profiler_criteria() -> SelectionCriteria {
    SelectionCriteria::ReadPreference(ReadPreference::Primary)
}

fn primary() -> Option<SelectionCriteria> {
    Some(profiler_criteria())
}

/// The setting as the primary, where it is set, reports it.
pub(crate) async fn profiling_level(db: &Database) -> Result<ProfilingState> {
    let reply = run(db, profile_level_command(), primary()).await?;
    parse_profiling_state(&reply)
}

/// Sets the level on the primary. The reply carries `slowms`; there is no read-back, so a
/// write that succeeded cannot be reported as an error.
pub(crate) async fn set_profiling_level(db: &Database, level: i32) -> Result<ProfilingState> {
    let reply = run(db, set_level_command(level)?, primary()).await?;
    Ok(parse_set_reply(&reply, level))
}

async fn find_profile(db: &Database, query: ProfileQuery) -> Result<Vec<Document>> {
    let cursor = db
        .collection::<Document>(PROFILE_COLLECTION)
        .find(query.filter)
        .sort(query.sort)
        .limit(query.limit)
        .selection_criteria(profiler_criteria())
        .await
        .map_err(db_err)?;
    cursor.try_collect().await.map_err(db_err)
}

/// Reads the entries strictly newer than `since_ts_millis`. A missing collection finds nothing.
/// Only the count of unparseable entries is logged: an entry holds the query's literal values.
pub(crate) async fn read_profile(db: &Database, since_ts_millis: i64) -> Result<ProfileRead> {
    let host = hello_host(&run(db, doc! { "hello": 1 }, primary()).await?);
    let docs = find_profile(db, since_query(since_ts_millis)).await?;
    let oldest = find_profile(db, oldest_query()).await?;
    let page = summarize_page(&docs);
    if page.skipped > 0 {
        log::warn!("skipped {} unparseable profile entries", page.skipped);
    }
    Ok(ProfileRead {
        entries: page.entries,
        oldest_ts_millis: oldest_ts_millis(&oldest),
        newest_ts_millis: page.newest_ts_millis,
        skipped: page.skipped,
        host,
    })
}

#[cfg(test)]
#[path = "mongo_profile_tests.rs"]
mod tests;
