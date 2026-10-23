//! Turns cumulative `$indexStats` counters into plottable points.
//!
//! `accesses.ops` is a per-node counter that only grows until the server
//! restarts, and `since` is the moment it was last reset. Sampling it gives
//! a sequence of [`Sample`]s; [`derive`] turns consecutive pairs into
//! [`Point`]s and [`bucketize`] folds those into fixed-width [`Bucket`]s.

/// Seconds between two scheduled samples.
pub const SAMPLE_INTERVAL_SECS: i64 = 60;

/// Missed sample intervals tolerated before the silence is a gap.
pub const GAP_INTERVALS: i64 = 3;

/// Longest span between two samples that still counts as continuous.
const MAX_CONTINUOUS_SECS: i64 = GAP_INTERVALS * SAMPLE_INTERVAL_SECS;

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
    /// Delta uses plus the uses of any gap that ends in this window.
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
/// time between them exceeds [`GAP_INTERVALS`] sample intervals, and both
/// (a `Gap` then a `Restart`) when it is a restart across a gap.
pub fn derive(samples: &[Sample]) -> Vec<Point> {
    let mut points = Vec::new();
    let mut previous: Option<Sample> = None;
    for &cur in samples {
        if let Some(prev) = previous {
            if cur.at_secs <= prev.at_secs {
                continue;
            }
            points.extend(classify(prev, cur));
        }
        previous = Some(cur);
    }
    points
}

fn classify(prev: Sample, cur: Sample) -> Vec<Point> {
    let span_secs = cur.at_secs - prev.at_secs;
    let restarted = cur.since_ms != prev.since_ms || cur.ops < prev.ops;
    let gap = span_secs > MAX_CONTINUOUS_SECS;
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

/// Folds points into dense, fixed-width buckets from the first to the last
/// bucket touched (empty ones included). A delta or restart lands in the
/// bucket holding its `at_secs`. A gap adds, to every bucket it overlaps, the
/// seconds inside that bucket; its `uses` land in the bucket holding
/// `to_secs`, so the total of `uses` is preserved. A non-positive width or no
/// points gives no buckets.
pub fn bucketize(points: &[Point], width_secs: i64) -> Vec<Bucket> {
    let Some((lo, hi)) = time_range(points) else {
        return Vec::new();
    };
    if width_secs <= 0 {
        return Vec::new();
    }
    let first = lo.div_euclid(width_secs);
    let last = hi.div_euclid(width_secs);
    let mut buckets: Vec<Bucket> = (first..=last)
        .map(|i| Bucket {
            start_secs: i * width_secs,
            width_secs,
            uses: 0,
            restarts: 0,
            gap_secs: 0,
        })
        .collect();
    let index = |secs: i64| usize::try_from(secs.div_euclid(width_secs) - first).unwrap_or(0);
    for point in points {
        match *point {
            Point::Delta { at_secs, uses, .. } => {
                let b = &mut buckets[index(at_secs)];
                b.uses = b.uses.saturating_add(uses);
            }
            Point::Restart { at_secs, .. } => buckets[index(at_secs)].restarts += 1,
            Point::Gap {
                from_secs,
                to_secs,
                uses,
            } => {
                let end = &mut buckets[index(to_secs)];
                end.uses = end.uses.saturating_add(uses);
                for b in &mut buckets[index(from_secs)..=index(to_secs)] {
                    let overlap =
                        (b.start_secs + width_secs).min(to_secs) - b.start_secs.max(from_secs);
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
