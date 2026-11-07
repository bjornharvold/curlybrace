use bson::{Bson, DateTime, doc};

use super::*;
use crate::error::CoreError;
use crate::store::Topology;

#[test]
fn profile_level_command_is_minus_one() {
    assert_eq!(profile_level_command(), doc! { "profile": -1 });
}

#[test]
fn set_level_command_carries_level() {
    for level in 0..=2 {
        assert_eq!(set_level_command(level), Ok(doc! { "profile": level }));
    }
}

#[test]
fn level_out_of_range_is_rejected() {
    for level in [-1, 3, i32::MIN, i32::MAX] {
        assert!(
            matches!(set_level_command(level), Err(CoreError::Unsupported(_))),
            "{level}"
        );
    }
}

#[test]
fn since_filter_uses_ts_gt_and_sorts_by_ts() {
    let q = since_query(1_700_000_000_000);
    let expected = DateTime::from_millis(1_700_000_000_000);
    assert_eq!(q.filter, doc! { "ts": { "$gt": expected } });
    assert_eq!(q.sort, doc! { "ts": 1 });
    assert_eq!(q.limit, PROFILE_READ_LIMIT);
    assert_eq!(PROFILE_READ_LIMIT, 1000);
}

#[test]
fn filter_uses_millisecond_precision() {
    let q = since_query(1_700_000_000_123);
    let expected = DateTime::from_millis(1_700_000_000_123);
    assert_eq!(q.filter, doc! { "ts": { "$gt": expected } });
}

#[test]
fn mongos_decision_is_unsupported() {
    let refused = mongos_decision(Some(Topology::Sharded));
    assert_eq!(
        refused,
        Err(CoreError::Unsupported(PROFILER_MONGOS_MESSAGE.into()))
    );
    assert_eq!(
        PROFILER_MONGOS_MESSAGE,
        "profiler not available through mongos"
    );
    assert_eq!(mongos_decision(Some(Topology::ReplicaSet)), Ok(()));
    assert_eq!(mongos_decision(Some(Topology::Direct)), Ok(()));
    assert_eq!(mongos_decision(None), Ok(()));
}

#[test]
fn profiling_state_accepts_any_numeric_width() {
    let want = ProfilingState {
        level: 1,
        slow_ms: 100,
    };
    let int32 = doc! { "was": 1_i32, "slowms": 100_i32 };
    let int64 = doc! { "was": 1_i64, "slowms": 100_i64 };
    let double = doc! { "was": 1.0_f64, "slowms": 100.0_f64 };
    for reply in [int32, int64, double] {
        assert_eq!(parse_profiling_state(&reply), Ok(want));
    }
}

#[test]
fn profiling_state_without_a_numeric_was_is_an_error() {
    for reply in [doc! { "slowms": 100 }, doc! { "was": "x", "slowms": 100 }] {
        assert!(matches!(
            parse_profiling_state(&reply),
            Err(CoreError::Database(_))
        ));
    }
}

#[test]
fn hello_host_comes_from_me() {
    assert_eq!(hello_host(&doc! { "me": "node1:27017" }), "node1:27017");
    assert_eq!(hello_host(&doc! {}), "");
    assert_eq!(hello_host(&doc! { "me": Bson::Int32(1) }), "");
}

#[test]
fn set_reply_gives_new_level_and_slowms() {
    // `was` is the PREVIOUS level; the new one is what was asked for.
    let reply = doc! { "was": 0, "slowms": 250_i64, "ratelimit": 1, "ok": 1.0 };
    assert_eq!(
        parse_set_reply(&reply, 2),
        ProfilingState {
            level: 2,
            slow_ms: 250
        }
    );
    assert_eq!(parse_set_reply(&doc! { "was": 1 }, 1).slow_ms, 0);
}

fn raw(ts_millis: i64) -> bson::Document {
    doc! { "ts": DateTime::from_millis(ts_millis), "ns": "db.c", "op": "query" }
}

#[test]
fn entries_skip_unparseable_and_count_them() {
    let docs = vec![raw(1000), doc! { "ns": "db.c" }, raw(2000), doc! {}];
    let page = summarize_page(&docs);
    assert_eq!(page.skipped, 2);
    let millis: Vec<i64> = page.entries.iter().map(|e| e.ts_millis).collect();
    assert_eq!(millis, vec![1000, 2000]);
}

#[test]
fn a_ts_that_is_not_a_datetime_is_skipped_and_counted() {
    let page = summarize_page(&[doc! { "ts": "x", "ns": "db.c" }]);
    assert!(page.entries.is_empty());
    assert_eq!(page.skipped, 1);
    assert_eq!(page.newest_ts_millis, None, "no usable ts to advance to");
}

#[test]
fn newest_ts_covers_unparseable_entries() {
    // The newest document has no `ns`, so it cannot be parsed, yet the watermark must pass it.
    let broken = doc! { "ts": DateTime::from_millis(9_500) };
    let page = summarize_page(&[raw(1_000), raw(2_000), broken]);
    assert_eq!(page.entries.len(), 2);
    assert_eq!(page.skipped, 1);
    assert_eq!(page.newest_ts_millis, Some(9_500));
    assert_eq!(summarize_page(&[]).newest_ts_millis, None);
}

#[test]
fn oldest_ts_survives_an_unparseable_oldest_document() {
    let broken = doc! { "ts": DateTime::from_millis(5_250), "ns": 1 };
    assert_eq!(oldest_ts_millis(&[broken]), Some(5_250));
    assert_eq!(oldest_ts_millis(&[raw(5_000)]), Some(5_000));
    assert_eq!(oldest_ts_millis(&[]), None);
    assert_eq!(oldest_ts_millis(&[doc! { "x": 1 }]), None);
}

#[test]
fn full_page_is_detectable() {
    let docs: Vec<_> = (0..PROFILE_READ_LIMIT)
        .map(|i| {
            if i % 2 == 0 {
                raw(i)
            } else {
                doc! { "ts": DateTime::from_millis(i) }
            }
        })
        .collect();
    let page = summarize_page(&docs);
    let total = page.entries.len() as u64 + page.skipped;
    assert_eq!(total, PROFILE_READ_LIMIT as u64);
}
