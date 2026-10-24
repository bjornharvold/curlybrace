//! Turns cumulative `$indexStats` counters into plottable points.
//!
//! `accesses.ops` is a per-node counter that only grows until the server
//! restarts, and `since` is the moment it was last reset. Sampling it gives
//! a sequence of [`Sample`]s; [`derive`] turns consecutive pairs into
//! [`Point`]s and [`bucketize`] folds those into fixed-width [`Bucket`]s.

use std::time::Duration;

/// Seconds between two scheduled samples.
pub const SAMPLE_INTERVAL_SECS: i64 = 60;

/// Missed sample intervals tolerated before the silence is a gap.
pub const GAP_INTERVALS: i64 = 3;

/// Longest span between two samples that still counts as continuous at the default interval.
pub const DEFAULT_MAX_CONTINUOUS_SECS: i64 = GAP_INTERVALS * SAMPLE_INTERVAL_SECS;

/// Longest span between two samples that still counts as continuous when samples are taken every
/// `interval`: [`GAP_INTERVALS`] intervals, but never less than
/// [`DEFAULT_MAX_CONTINUOUS_SECS`]. The interval is a runtime tunable, so the threshold follows
/// it; the floor keeps a very short interval from turning every reading into a gap, since a pass
/// over many collections takes longer than three such intervals.
pub fn max_continuous_secs(interval: Duration) -> i64 {
    let scaled = i64::try_from(interval.as_secs())
        .unwrap_or(i64::MAX)
        .saturating_mul(GAP_INTERVALS);
    scaled.max(DEFAULT_MAX_CONTINUOUS_SECS)
}

/// How far ahead of the wall clock a stored sample may be stamped and still be believed. A
/// sample further ahead came from a clock that was wrong at the time (set far forward, or
/// stepped back since): reads ignore it and thinning deletes it.
pub const MAX_FUTURE_SKEW_SECS: i64 = 300;

/// One reading of an index's cumulative access counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    /// When the reading was taken, in Unix seconds.
    pub at_secs: i64,
    /// `accesses.ops`: uses since the counter was last reset.
    pub ops: i64,
    /// `accesses.since` in Unix milliseconds; changes when the counter resets.
    pub since_ms: i64,
}

/// One plottable fact derived from a pair of consecutive samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Point {
    /// Uses between two samples that were close enough to be continuous.
    Delta {
        at_secs: i64,
        uses: i64,
        span_secs: i64,
    },
    /// The counter was reset. There is no delta, only the count reached since.
    Restart {
        at_secs: i64,
        uses_since_restart: i64,
    },
    /// Silence longer than [`GAP_INTERVALS`] intervals. `uses` is the counter
    /// difference over the whole gap, never spread over time. It is 0 when a
    /// restart happened inside the gap, because the difference is then unknown.
    Gap {
        from_secs: i64,
        to_secs: i64,
        uses: i64,
    },
}

/// Points folded into a fixed-width window `[start_secs, start_secs + width_secs)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bucket {
    pub start_secs: i64,
    pub width_secs: i64,
    /// Delta uses, the uses counted since a restart, and the uses of any gap that ends in this
    /// window.
    pub uses: i64,
    /// Restart points inside the window.
    pub restarts: u32,
    /// Seconds of the window covered by a gap.
    pub gap_secs: i64,
}

/// Derives points from samples taken in the order they were recorded.
///
/// A sample whose `at_secs` is not strictly after the previously accepted one
/// (the clock moved back, or a duplicate) is ignored; the input is therefore
/// not re-sorted. The first accepted sample yields no point. A pair is a
/// restart when `since_ms` changed or the counter dropped, a gap when the
/// time between them exceeds `max_continuous_secs` (see [`max_continuous_secs`]), and both
/// (a `Gap` then a `Restart`) when it is a restart across a gap.
pub fn derive(samples: &[Sample], max_continuous_secs: i64) -> Vec<Point> {
    let accepted: Vec<Sample> = accepted_indices(samples)
        .into_iter()
        .map(|i| samples[i])
        .collect();
    accepted
        .windows(2)
        .flat_map(|pair| classify(pair[0], pair[1], max_continuous_secs))
        .collect()
}

/// Positions of the samples [`derive`] accepts: each one strictly after the previously
/// accepted one. The single definition of that rule; the store uses it to pick what it may
/// roll up and delete, so a backwards-clock sample never decides that.
///
/// Clock steps: a sample far ahead of the wall clock never reaches this rule, because the store
/// leaves it out when reading and deletes it when thinning (see [`MAX_FUTURE_SKEW_SECS`]). A
/// step back of less than that skew between two passes makes the next samples not strictly
/// after the last stored one; they are ignored until real time catches up, at most the skew. A
/// bigger step back makes the stored samples "future" ones, so the new samples take over at
/// once. Either way nothing grows: the buckets are bounded by [`MAX_BUCKETS`].
pub fn accepted_indices(samples: &[Sample]) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    for (i, cur) in samples.iter().enumerate() {
        if out.last().is_none_or(|&p| cur.at_secs > samples[p].at_secs) {
            out.push(i);
        }
    }
    out
}

fn classify(prev: Sample, cur: Sample, max_continuous_secs: i64) -> Vec<Point> {
    let span_secs = cur.at_secs - prev.at_secs;
    let restarted = cur.since_ms != prev.since_ms || cur.ops < prev.ops;
    let gap = span_secs > max_continuous_secs;
    let uses = cur.ops.saturating_sub(prev.ops);
    let mut out = Vec::with_capacity(2);
    if gap {
        out.push(Point::Gap {
            from_secs: prev.at_secs,
            to_secs: cur.at_secs,
            uses: if restarted { 0 } else { uses },
        });
    }
    if restarted {
        out.push(Point::Restart {
            at_secs: cur.at_secs,
            uses_since_restart: cur.ops,
        });
    } else if !gap {
        out.push(Point::Delta {
            at_secs: cur.at_secs,
            uses,
            span_secs,
        });
    }
    out
}

/// Most buckets one fold may allocate. A chart never needs more (a month of daily buckets is
/// 31), so this only bites on a span a bad stored sample made absurd; the newest buckets win.
pub const MAX_BUCKETS: usize = 20_000;

/// [`bucketize_within`] over all time: bounded by [`MAX_BUCKETS`] alone.
pub fn bucketize(points: &[Point], width_secs: i64) -> Vec<Bucket> {
    bucketize_within(points, width_secs, i64::MIN, i64::MAX)
}

/// Folds points into dense, fixed-width buckets from the first to the last bucket touched
/// (empty ones included), keeping only the buckets that overlap `[from_secs, to_secs]` and at
/// most [`MAX_BUCKETS`] of them (the newest), so the allocation is bounded by the window asked
/// for, never by what is stored. A delta or restart lands in the bucket holding its `at_secs`; a
/// restart also adds the uses counted since the reset. A gap adds, to every bucket it overlaps,
/// the seconds inside that bucket; its `uses` land in the bucket holding `to_secs`, so the total
/// of `uses` is preserved when that bucket is kept. A non-positive width or no points gives no
/// buckets.
pub fn bucketize_within(
    points: &[Point],
    width_secs: i64,
    from_secs: i64,
    to_secs: i64,
) -> Vec<Bucket> {
    let Some((lo, hi)) = time_range(points) else {
        return Vec::new();
    };
    if width_secs <= 0 {
        return Vec::new();
    }
    let last = hi
        .div_euclid(width_secs)
        .min(to_secs.div_euclid(width_secs));
    let capped = last.saturating_sub(MAX_BUCKETS as i64 - 1);
    let first = lo
        .div_euclid(width_secs)
        .max(from_secs.div_euclid(width_secs))
        .max(capped);
    if last < first {
        return Vec::new();
    }
    let mut buckets: Vec<Bucket> = (first..=last)
        .map(|i| Bucket {
            start_secs: i * width_secs,
            width_secs,
            uses: 0,
            restarts: 0,
            gap_secs: 0,
        })
        .collect();
    let slot = |secs: i64| {
        let n = secs.div_euclid(width_secs);
        (first..=last)
            .contains(&n)
            .then(|| usize::try_from(n - first).unwrap_or(0))
    };
    for point in points {
        match *point {
            Point::Delta { at_secs, uses, .. } => {
                if let Some(i) = slot(at_secs) {
                    buckets[i].uses = buckets[i].uses.saturating_add(uses);
                }
            }
            Point::Restart {
                at_secs,
                uses_since_restart,
            } => {
                // Uses counted since the reset are real uses seen in this window.
                if let Some(i) = slot(at_secs) {
                    buckets[i].restarts += 1;
                    buckets[i].uses = buckets[i].uses.saturating_add(uses_since_restart);
                }
            }
            Point::Gap {
                from_secs: gap_from,
                to_secs: gap_to,
                uses,
            } => {
                if let Some(i) = slot(gap_to) {
                    buckets[i].uses = buckets[i].uses.saturating_add(uses);
                }
                let (a, z) = (
                    gap_from.div_euclid(width_secs).max(first),
                    gap_to.div_euclid(width_secs).min(last),
                );
                for n in a..=z {
                    let b = &mut buckets[usize::try_from(n - first).unwrap_or(0)];
                    let overlap = b.start_secs.saturating_add(width_secs).min(gap_to)
                        - b.start_secs.max(gap_from);
                    b.gap_secs += overlap.max(0);
                }
            }
        }
    }
    buckets
}

fn time_range(points: &[Point]) -> Option<(i64, i64)> {
    points
        .iter()
        .map(|p| match *p {
            Point::Delta { at_secs, .. } | Point::Restart { at_secs, .. } => (at_secs, at_secs),
            Point::Gap {
                from_secs, to_secs, ..
            } => (from_secs, to_secs),
        })
        .reduce(|(lo, hi), (a, b)| (lo.min(a), hi.max(b)))
}
