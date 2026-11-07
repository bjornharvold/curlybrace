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
    let q = since_query(1_700_000_000);
    let expected = DateTime::from_millis(1_700_000_000_000);
    assert_eq!(q.filter, doc! { "ts": { "$gt": expected } });
    assert_eq!(q.sort, doc! { "ts": 1 });
    assert_eq!(q.limit, PROFILE_READ_LIMIT);
    assert_eq!(PROFILE_READ_LIMIT, 1000);
}

#[test]
fn mongos_decision_is_unsupported() {
    let refused = mongos_decision(Some(Topology::Sharded));
    assert!(matches!(refused, Err(CoreError::Unsupported(m)) if m.contains("mongos")));
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

fn raw(ts_millis: i64) -> bson::Document {
    doc! { "ts": DateTime::from_millis(ts_millis), "ns": "db.c", "op": "query" }
}

#[test]
fn entries_skip_unparseable_and_count_them() {
    let docs = vec![raw(1000), doc! { "ns": "db.c" }, raw(2000), doc! {}];
    let (entries, skipped) = parse_entries(&docs);
    assert_eq!(skipped, 2);
    let secs: Vec<i64> = entries.iter().map(|e| e.ts_secs).collect();
    assert_eq!(secs, vec![1, 2]);
}

#[test]
fn oldest_ts_is_read_from_the_first_document() {
    assert_eq!(oldest_ts_secs(&[raw(5000)]), Some(5));
    assert_eq!(oldest_ts_secs(&[]), None);
    assert_eq!(oldest_ts_secs(&[doc! { "x": 1 }]), None);
}
