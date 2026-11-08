//! Scripted profiler of the in-memory store: per database, a level, the `system.profile`
//! entries and an optional error. The `DocumentStore` methods in `memory_store` delegate here.

use std::collections::BTreeMap;

use super::memory_store::InMemoryStore;
use crate::error::{CoreError, Result};
use crate::profile::ProfileEntry;
use crate::store::{PROFILE_READ_LIMIT, ProfileRead, ProfilingState};

/// The node every read reports as its source.
const FAKE_HOST: &str = "localhost:27017";
/// The server's default `slowms`.
const DEFAULT_SLOW_MS: i64 = 100;

/// What is scripted for one database.
#[derive(Clone, Default)]
pub(super) struct ScriptedProfile {
    level: i32,
    entries: Vec<ProfileEntry>,
    /// Timestamps (ms) of documents the real store would fail to parse.
    unparseable: Vec<i64>,
    error: Option<CoreError>,
}

pub(super) type ProfileMap = BTreeMap<String, ScriptedProfile>;

impl InMemoryStore {
    /// Scripts the `system.profile` entries of `db` (added to any scripted before).
    pub fn with_profile(self, db: &str, entries: Vec<ProfileEntry>) -> Self {
        self.script(db, |p| ScriptedProfile {
            entries: p.entries.into_iter().chain(entries).collect(),
            ..p
        })
    }

    /// Scripts documents with these `ts` (ms) that cannot be parsed: they take part in paging,
    /// `newest`/`oldest` and `skipped`, but yield no entry.
    pub fn with_profile_unparseable(self, db: &str, ts_millis: Vec<i64>) -> Self {
        self.script(db, |p| ScriptedProfile {
            unparseable: p.unparseable.into_iter().chain(ts_millis).collect(),
            ..p
        })
    }

    /// Scripts the profiler level `db` starts at.
    pub fn with_profiling_level(self, db: &str, level: i32) -> Self {
        self.script(db, |p| ScriptedProfile { level, ..p })
    }

    /// Makes every profiler call for `db` fail with `error`.
    pub fn with_profile_error(self, db: &str, error: CoreError) -> Self {
        self.script(db, |p| ScriptedProfile {
            error: Some(error),
            ..p
        })
    }

    fn script(self, db: &str, change: impl FnOnce(ScriptedProfile) -> ScriptedProfile) -> Self {
        {
            let mut map = self
                .profiles
                .lock()
                .expect("fresh mutex cannot be poisoned");
            let current = map.get(db).cloned().unwrap_or_default();
            map.insert(db.to_owned(), change(current));
        }
        self
    }
}

fn state(profile: &ScriptedProfile) -> ProfilingState {
    ProfilingState {
        level: profile.level,
        slow_ms: DEFAULT_SLOW_MS,
    }
}

/// The script of `db`, or its scripted error.
fn scripted<'a>(map: &'a ProfileMap, db: &str) -> Result<Option<&'a ScriptedProfile>> {
    match map.get(db) {
        Some(ScriptedProfile { error: Some(e), .. }) => Err(e.clone()),
        other => Ok(other),
    }
}

pub(super) fn level(map: &ProfileMap, db: &str) -> Result<ProfilingState> {
    Ok(scripted(map, db)?.map(state).unwrap_or(ProfilingState {
        level: 0,
        slow_ms: DEFAULT_SLOW_MS,
    }))
}

/// Mirrors the real read: `ts > since` (strict), ascending, capped; `skipped`, `newest` and
/// `oldest` computed from raw timestamps so unparseable documents count.
pub(super) fn read(map: &ProfileMap, db: &str, since_ts_millis: i64) -> Result<ProfileRead> {
    let Some(p) = scripted(map, db)? else {
        return Ok(empty_read());
    };
    let mut rows: Vec<(i64, Option<&ProfileEntry>)> = p
        .entries
        .iter()
        .map(|e| (e.ts_millis, Some(e)))
        .chain(p.unparseable.iter().map(|ts| (*ts, None)))
        .collect();
    let oldest = rows.iter().map(|(ts, _)| *ts).min();
    rows.retain(|(ts, _)| *ts > since_ts_millis);
    rows.sort_by_key(|(ts, _)| *ts);
    rows.truncate(PROFILE_READ_LIMIT);
    Ok(ProfileRead {
        newest_ts_millis: rows.last().map(|(ts, _)| *ts),
        skipped: rows.iter().filter(|(_, e)| e.is_none()).count() as u64,
        entries: rows.iter().filter_map(|(_, e)| e.cloned()).collect(),
        oldest_ts_millis: oldest,
        host: FAKE_HOST.into(),
    })
}

fn empty_read() -> ProfileRead {
    ProfileRead {
        entries: Vec::new(),
        oldest_ts_millis: None,
        newest_ts_millis: None,
        skipped: 0,
        host: FAKE_HOST.into(),
    }
}

pub(super) fn set_level(map: &mut ProfileMap, db: &str, level: i32) -> Result<ProfilingState> {
    if !(0..=2).contains(&level) {
        return Err(CoreError::Unsupported(format!(
            "profiling level {level} is not 0, 1 or 2"
        )));
    }
    scripted(map, db)?;
    let next = ScriptedProfile {
        level,
        ..map.get(db).cloned().unwrap_or_default()
    };
    let out = state(&next);
    map.insert(db.to_owned(), next);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use crate::error::CoreError;
    use crate::profile::ProfileEntry;
    use crate::store::{DocumentStore, PROFILE_READ_LIMIT, ProfileRead};
    use crate::test_support::InMemoryStore;

    fn entry(ts_millis: i64) -> ProfileEntry {
        ProfileEntry {
            ts_millis,
            ts_secs: ts_millis.div_euclid(1000),
            ns: "db.c".into(),
            op: "query".into(),
            millis: 1,
            plan_summary: "COLLSCAN".into(),
            keys_examined: 0,
            docs_examined: 10,
            returned: 1,
            has_sort_stage: false,
            app_name: None,
            command: bson::Document::new(),
        }
    }

    fn ts(read: &ProfileRead) -> Vec<i64> {
        read.entries.iter().map(|e| e.ts_millis).collect()
    }

    #[tokio::test]
    async fn scripted_entries_are_filtered_by_since_and_sorted() {
        let store = InMemoryStore::new().with_profile("db", vec![entry(30), entry(10), entry(20)]);
        let read = store.read_profile("db", 10).await.unwrap();
        assert_eq!(ts(&read), vec![20, 30], "strictly greater than since");
        assert_eq!(read.oldest_ts_millis, Some(10), "oldest over all entries");
        assert_eq!(read.newest_ts_millis, Some(30));
        assert_eq!(read.skipped, 0);
        assert_eq!(read.host, "localhost:27017");
    }

    #[tokio::test]
    async fn fake_and_real_filter_agree_on_same_second_entries() {
        let (first, second) = (7_100, 7_900);
        let store = InMemoryStore::new().with_profile("db", vec![entry(first), entry(second)]);
        let read = store.read_profile("db", first).await.unwrap();
        assert_eq!(ts(&read), vec![second]);
        let bound = bson::DateTime::from_millis(first);
        let real = crate::mongo_profile::since_query(first);
        assert_eq!(real.filter, bson::doc! { "ts": { "$gt": bound } });
    }

    #[tokio::test]
    async fn unscripted_db_reads_level_zero_and_no_entries() {
        let store = InMemoryStore::new();
        let state = store.profiling_level("db").await.unwrap();
        assert_eq!((state.level, state.slow_ms), (0, 100));
        let read = store.read_profile("db", 0).await.unwrap();
        assert!(read.entries.is_empty());
        assert_eq!(read.oldest_ts_millis, None);
        assert_eq!(read.newest_ts_millis, None);
    }

    #[tokio::test]
    async fn level_changes_after_set_profiling_level() {
        let store = InMemoryStore::new().with_profiling_level("db", 1);
        assert_eq!(store.profiling_level("db").await.unwrap().level, 1);
        let new = store.set_profiling_level("db", 2).await.unwrap();
        assert_eq!(new.level, 2);
        assert_eq!(store.profiling_level("db").await.unwrap().level, 2);
        assert_eq!(store.profiling_level("other").await.unwrap().level, 0);
    }

    #[tokio::test]
    async fn out_of_range_level_is_rejected_and_changes_nothing() {
        let store = InMemoryStore::new().with_profiling_level("db", 1);
        for level in [-1, 3] {
            let out = store.set_profiling_level("db", level).await;
            assert!(matches!(out, Err(CoreError::Unsupported(_))), "{level}");
        }
        assert_eq!(store.profiling_level("db").await.unwrap().level, 1);
    }

    #[tokio::test]
    async fn scripted_error_fails_every_profile_call_for_that_db() {
        let store = InMemoryStore::new().with_profile_error("db", CoreError::Unauthorized);
        assert_eq!(
            store.profiling_level("db").await,
            Err(CoreError::Unauthorized)
        );
        assert_eq!(
            store.read_profile("db", 0).await,
            Err(CoreError::Unauthorized)
        );
        assert_eq!(
            store.set_profiling_level("db", 1).await,
            Err(CoreError::Unauthorized)
        );
        assert!(store.profiling_level("other").await.is_ok());
    }

    #[tokio::test]
    async fn read_is_capped_but_oldest_stays_the_global_minimum() {
        let all: Vec<ProfileEntry> = (1..=1500).rev().map(entry).collect();
        let store = InMemoryStore::new().with_profile("db", all);
        let read = store.read_profile("db", 500).await.unwrap();
        assert_eq!(read.entries.len(), 1000);
        assert_eq!(read.entries[0].ts_millis, 501);
        assert_eq!(read.entries[999].ts_millis, 1500);
        assert_eq!(read.newest_ts_millis, Some(1500));
        assert_eq!(read.oldest_ts_millis, Some(1));
        let capped = store.read_profile("db", 0).await.unwrap();
        assert_eq!(capped.entries.len(), PROFILE_READ_LIMIT);
        assert_eq!(capped.entries[999].ts_millis, 1000, "oldest 1000 first");
        assert_eq!(capped.newest_ts_millis, Some(1000));
    }

    #[tokio::test]
    async fn unparseable_entries_count_and_move_newest_and_oldest() {
        let store = InMemoryStore::new()
            .with_profile("db", vec![entry(20), entry(30)])
            .with_profile_unparseable("db", vec![5, 40]);
        let read = store.read_profile("db", 10).await.unwrap();
        assert_eq!(ts(&read), vec![20, 30]);
        assert_eq!(read.skipped, 1, "only the one inside the page");
        assert_eq!(read.newest_ts_millis, Some(40));
        assert_eq!(
            read.oldest_ts_millis,
            Some(5),
            "an unparseable oldest counts"
        );
    }

    #[tokio::test]
    async fn full_page_is_detectable() {
        let parseable: Vec<ProfileEntry> = (1..=600).map(entry).collect();
        let broken: Vec<i64> = (601..=1200).collect();
        let store = InMemoryStore::new()
            .with_profile("db", parseable)
            .with_profile_unparseable("db", broken);
        let read = store.read_profile("db", 0).await.unwrap();
        assert_eq!(read.entries.len() as u64 + read.skipped, 1000);
        assert_eq!(read.newest_ts_millis, Some(1000));
    }
}
