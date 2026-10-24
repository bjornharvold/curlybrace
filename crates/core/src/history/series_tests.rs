use super::series::*;
use proptest::prelude::*;
use std::time::Duration;

const SINCE: i64 = 1_000;

fn s(at_secs: i64, ops: i64) -> Sample {
    Sample {
        at_secs,
        ops,
        since_ms: SINCE,
    }
}

fn total_uses(points: &[Point]) -> i64 {
    points
        .iter()
        .map(|p| match p {
            Point::Delta { uses, .. } | Point::Gap { uses, .. } => *uses,
            Point::Restart {
                uses_since_restart, ..
            } => *uses_since_restart,
        })
        .sum()
}

#[test]
fn the_first_sample_has_no_point() {
    assert!(derive(&[s(100, 5)], DEFAULT_MAX_CONTINUOUS_SECS).is_empty());
    assert!(derive(&[], DEFAULT_MAX_CONTINUOUS_SECS).is_empty());
}

#[test]
fn consecutive_samples_give_deltas() {
    let points = derive(
        &[s(100, 5), s(160, 9), s(220, 9)],
        DEFAULT_MAX_CONTINUOUS_SECS,
    );
    assert_eq!(
        points,
        vec![
            Point::Delta {
                at_secs: 160,
                uses: 4,
                span_secs: 60
            },
            Point::Delta {
                at_secs: 220,
                uses: 0,
                span_secs: 60
            },
        ]
    );
}

#[test]
fn a_lower_counter_is_a_restart_with_no_delta() {
    let points = derive(&[s(100, 50), s(160, 3)], DEFAULT_MAX_CONTINUOUS_SECS);
    assert_eq!(
        points,
        vec![Point::Restart {
            at_secs: 160,
            uses_since_restart: 3
        }]
    );
}

#[test]
fn a_changed_since_is_a_restart() {
    let restarted = Sample {
        at_secs: 160,
        ops: 80,
        since_ms: SINCE + 5,
    };
    let points = derive(&[s(100, 50), restarted], DEFAULT_MAX_CONTINUOUS_SECS);
    assert_eq!(
        points,
        vec![Point::Restart {
            at_secs: 160,
            uses_since_restart: 80
        }]
    );
}

#[test]
fn a_long_silence_is_a_gap_carrying_the_uses_in_it() {
    let points = derive(&[s(100, 5), s(1_000, 45)], DEFAULT_MAX_CONTINUOUS_SECS);
    assert_eq!(
        points,
        vec![Point::Gap {
            from_secs: 100,
            to_secs: 1_000,
            uses: 40
        }]
    );
}

#[test]
fn a_gap_is_declared_only_beyond_three_intervals() {
    let edge = GAP_INTERVALS * SAMPLE_INTERVAL_SECS;
    let at_edge = derive(&[s(0, 0), s(edge, 1)], DEFAULT_MAX_CONTINUOUS_SECS);
    assert!(matches!(at_edge[0], Point::Delta { .. }));
    let past_edge = derive(&[s(0, 0), s(edge + 1, 1)], DEFAULT_MAX_CONTINUOUS_SECS);
    assert!(matches!(past_edge[0], Point::Gap { .. }));
}

#[test]
fn a_gap_never_becomes_a_zero_delta() {
    let points = derive(&[s(0, 7), s(10_000, 7)], DEFAULT_MAX_CONTINUOUS_SECS);
    assert_eq!(
        points,
        vec![Point::Gap {
            from_secs: 0,
            to_secs: 10_000,
            uses: 0
        }]
    );
    assert!(points.iter().all(|p| !matches!(p, Point::Delta { .. })));
}

#[test]
fn a_restart_inside_a_gap_is_both() {
    let restarted = Sample {
        at_secs: 1_000,
        ops: 12,
        since_ms: SINCE + 1,
    };
    let points = derive(&[s(100, 50), restarted], DEFAULT_MAX_CONTINUOUS_SECS);
    assert_eq!(
        points,
        vec![
            Point::Gap {
                from_secs: 100,
                to_secs: 1_000,
                uses: 0
            },
            Point::Restart {
                at_secs: 1_000,
                uses_since_restart: 12
            },
        ]
    );
}

#[test]
fn a_sample_not_after_the_previous_one_is_ignored() {
    // The clock moved back (130) and a duplicate (160) are both dropped.
    let points = derive(
        &[s(100, 1), s(160, 2), s(130, 99), s(160, 50), s(220, 5)],
        DEFAULT_MAX_CONTINUOUS_SECS,
    );
    assert_eq!(
        points,
        vec![
            Point::Delta {
                at_secs: 160,
                uses: 1,
                span_secs: 60
            },
            Point::Delta {
                at_secs: 220,
                uses: 3,
                span_secs: 60
            },
        ]
    );
}

#[test]
fn bucketize_sums_deltas_and_counts_restarts_and_gap_seconds() {
    let points = vec![
        Point::Delta {
            at_secs: 30,
            uses: 2,
            span_secs: 60,
        },
        Point::Delta {
            at_secs: 90,
            uses: 3,
            span_secs: 60,
        },
        Point::Restart {
            at_secs: 150,
            uses_since_restart: 4,
        },
        Point::Gap {
            from_secs: 150,
            to_secs: 330,
            uses: 10,
        },
    ];
    let buckets = bucketize(&points, 100);
    let b = |start_secs, uses, restarts, gap_secs| Bucket {
        start_secs,
        width_secs: 100,
        uses,
        restarts,
        gap_secs,
    };
    assert_eq!(
        buckets,
        vec![
            b(0, 5, 0, 0),
            // The restart adds the uses counted since the reset.
            b(100, 4, 1, 50),
            b(200, 0, 0, 100),
            // The gap's uses land in the bucket holding its end.
            b(300, 10, 0, 30),
        ]
    );
}

#[test]
fn bucketize_of_nothing_or_a_bad_width_is_empty() {
    assert!(bucketize(&[], 60).is_empty());
    let p = [Point::Delta {
        at_secs: 1,
        uses: 1,
        span_secs: 1,
    }];
    assert!(bucketize(&p, 0).is_empty());
    assert!(bucketize(&p, -5).is_empty());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn bucketing_preserves_total_uses_for_any_sample_sequence(
        steps in proptest::collection::vec((-100i64..2_000, 0i64..500, any::<bool>()), 0..40),
        // At most 80,000 s of samples, so no width here reaches the bucket cap.
        width in 10i64..5_000,
    ) {
        let mut at = 10_000;
        let mut since = SINCE;
        let samples: Vec<Sample> = steps
            .iter()
            .map(|(dt, ops, bump)| {
                at += dt;
                if *bump {
                    since += 1;
                }
                Sample { at_secs: at, ops: *ops, since_ms: since }
            })
            .collect();
        let points = derive(&samples, DEFAULT_MAX_CONTINUOUS_SECS);
        let buckets = bucketize(&points, width);
        prop_assert_eq!(buckets.iter().map(|b| b.uses).sum::<i64>(), total_uses(&points));
        let gap_total: i64 = points
            .iter()
            .map(|p| match p {
                Point::Gap { from_secs, to_secs, .. } => to_secs - from_secs,
                _ => 0,
            })
            .sum();
        prop_assert_eq!(buckets.iter().map(|b| b.gap_secs).sum::<i64>(), gap_total);
    }
}

#[test]
fn uses_counted_since_a_restart_land_in_the_bucket() {
    let restart = Sample {
        at_secs: 160,
        ops: 7,
        since_ms: SINCE + 1,
    };
    let buckets = bucketize(
        &derive(&[s(100, 50), restart], DEFAULT_MAX_CONTINUOUS_SECS),
        600,
    );
    assert_eq!(buckets.len(), 1);
    assert_eq!(buckets[0].restarts, 1);
    assert_eq!(buckets[0].uses, 7);
}

#[test]
fn the_gap_threshold_follows_the_sampling_interval() {
    let five_minutes = max_continuous_secs(Duration::from_secs(300));
    assert_eq!(five_minutes, GAP_INTERVALS * 300);
    // Consecutive samples five minutes apart are continuous at a five minute interval...
    let points = derive(&[s(0, 0), s(300, 4), s(600, 9)], five_minutes);
    assert!(
        points.iter().all(|p| matches!(p, Point::Delta { .. })),
        "{points:?}"
    );
    // ...and the same samples are gaps against the default one-minute threshold.
    let strict = derive(&[s(0, 0), s(300, 4)], DEFAULT_MAX_CONTINUOUS_SECS);
    assert!(matches!(strict[0], Point::Gap { .. }));
    // The default interval gives the default threshold.
    assert_eq!(
        max_continuous_secs(Duration::from_secs(SAMPLE_INTERVAL_SECS as u64)),
        DEFAULT_MAX_CONTINUOUS_SECS
    );
}

#[test]
fn bucketize_never_allocates_more_than_the_cap_however_wide_the_span() {
    // One sample taken with the clock far ahead makes a gap of ~1.6 years: about 833,000 buckets
    // of a minute without a cap.
    let far = 50_000_000;
    let points = [
        Point::Delta {
            at_secs: 100,
            uses: 1,
            span_secs: 60,
        },
        Point::Gap {
            from_secs: 100,
            to_secs: far,
            uses: 4,
        },
    ];
    let buckets = bucketize(&points, 60);
    assert!(buckets.len() <= MAX_BUCKETS, "{} buckets", buckets.len());
    let newest = buckets.last().expect("the newest buckets are kept");
    assert_eq!(newest.start_secs, far.div_euclid(60) * 60);
    assert_eq!(newest.uses, 4, "the gap's uses sit at its end");
}

#[test]
fn bucketize_within_keeps_only_the_buckets_of_the_window() {
    let points = [
        Point::Delta {
            at_secs: 30,
            uses: 2,
            span_secs: 60,
        },
        Point::Delta {
            at_secs: 330,
            uses: 3,
            span_secs: 60,
        },
        Point::Gap {
            from_secs: 0,
            to_secs: 10_000_000,
            uses: 7,
        },
    ];
    let buckets = bucketize_within(&points, 100, 300, 499);
    let starts: Vec<i64> = buckets.iter().map(|b| b.start_secs).collect();
    assert_eq!(starts, vec![300, 400]);
    // The delta in the window counts; the gap covers both buckets whole; its uses land at its
    // far end, outside the window, so they are not here.
    assert_eq!(
        buckets.iter().map(|b| b.uses).collect::<Vec<_>>(),
        vec![3, 0]
    );
    assert_eq!(
        buckets.iter().map(|b| b.gap_secs).collect::<Vec<_>>(),
        vec![100, 100]
    );
    // A window the points never touch gives nothing.
    assert!(bucketize_within(&points, 100, 20_000_000, 20_000_100).is_empty());
}
