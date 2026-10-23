//! What the usage chart of an index plots. Pure and egui-free.
//!
//! Time runs along x in seconds relative to `now` (0 is now, the past is negative), so the
//! model does not depend on the wall clock and a chart reads the same whenever it is drawn.
use curlybrace_core::history::{Bucket, ChartRange};
use curlybrace_core::runtime::IndexSeries;

/// The marks of one chart.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChartModel {
    /// `[x, uses]` of each bucket that was observed, x at the middle of the bucket.
    pub points: Vec<[f64; 2]>,
    /// x of the start of each bucket the counter was reset in.
    pub restarts: Vec<f64>,
    /// `(from, to)` x of each stretch without data; neighbouring stretches are merged.
    pub gaps: Vec<(f64, f64)>,
    /// `(from, to)` x of each bucket that is only partly unobserved; its point is drawn.
    pub partial_gaps: Vec<(f64, f64)>,
    /// x of each point whose bucket holds unobserved time, so its value may be a lower bound.
    pub lower_bounds: Vec<f64>,
}

/// The numbers of the text line below the chart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChartSummary {
    pub uses: i64,
    pub restarts: i64,
    /// Whole hours without data (rounded down).
    pub gap_hours: i64,
}

/// Plots `series` over the last `range` before `now_secs`. Real uses never vanish: a bucket with
/// uses is always a point, and when it also holds unobserved time the point is a lower bound
/// (`lower_bounds`). A bucket with gap time and no uses has no point, since a gap is never
/// drawn as zero. The gap time itself is shaded: the whole bucket in `gaps` when all of it is
/// unobserved, in `partial_gaps` when only part of it is.
pub fn chart_model(series: &IndexSeries, range: ChartRange, now_secs: i64) -> ChartModel {
    let x = |secs: i64| (secs - now_secs) as f64;
    let mut model = ChartModel::default();
    for bucket in windowed(series, range, now_secs) {
        let span = (
            x(bucket.start_secs),
            x(bucket.start_secs + bucket.width_secs),
        );
        let middle = (span.0 + span.1) / 2.0;
        if bucket.restarts > 0 {
            model.restarts.push(span.0);
        }
        if bucket.gap_secs >= bucket.width_secs {
            push_gap(&mut model.gaps, span);
        } else if bucket.gap_secs > 0 {
            model.partial_gaps.push(span);
        }
        if bucket.gap_secs == 0 || bucket.uses > 0 {
            model.points.push([middle, bucket.uses as f64]);
        }
        if bucket.gap_secs > 0 && bucket.uses > 0 {
            model.lower_bounds.push(middle);
        }
    }
    model
}

/// Uses in the window, or `None` when no bucket of it was observed at all (a zero would then
/// be a guess, and an index must never look unused because nobody was watching).
pub fn observed_uses(series: &IndexSeries, range: ChartRange, now_secs: i64) -> Option<i64> {
    let buckets = windowed(series, range, now_secs);
    let observed = buckets.iter().any(|b| b.gap_secs < b.width_secs);
    observed.then(|| buckets.iter().map(|b| b.uses).sum())
}

/// Totals over the same window as [`chart_model`].
pub fn chart_summary(series: &IndexSeries, range: ChartRange, now_secs: i64) -> ChartSummary {
    let buckets = windowed(series, range, now_secs);
    let gap_secs: i64 = buckets.iter().map(|b| b.gap_secs).sum();
    ChartSummary {
        uses: buckets.iter().map(|b| b.uses).sum(),
        restarts: buckets.iter().map(|b| i64::from(b.restarts)).sum(),
        gap_hours: gap_secs / 3_600,
    }
}

/// The buckets of `series` that overlap the window, oldest first.
fn windowed(series: &IndexSeries, range: ChartRange, now_secs: i64) -> Vec<&Bucket> {
    let from = now_secs - range.secs();
    let mut buckets: Vec<&Bucket> = series
        .buckets
        .iter()
        .filter(|b| b.start_secs + b.width_secs > from && b.start_secs < now_secs)
        .collect();
    buckets.sort_by_key(|b| b.start_secs);
    buckets
}

/// Adds `gap`, joining it to the previous one when they touch.
fn push_gap(gaps: &mut Vec<(f64, f64)>, gap: (f64, f64)) {
    match gaps.last_mut() {
        Some(last) if last.1 >= gap.0 => last.1 = last.1.max(gap.1),
        _ => gaps.push(gap),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000_000;
    const HOUR: i64 = 3_600;

    fn bucket(start: i64, uses: i64, restarts: u32, gap_secs: i64) -> Bucket {
        Bucket {
            start_secs: start,
            width_secs: HOUR,
            uses,
            restarts,
            gap_secs,
        }
    }

    fn series(buckets: Vec<Bucket>) -> IndexSeries {
        IndexSeries {
            index_name: "a_1".into(),
            definition_hash: "h".into(),
            host: "n1".into(),
            buckets,
            latest: None,
        }
    }

    #[test]
    fn chart_model_marks_restarts_and_leaves_gaps_unplotted() {
        let s = series(vec![
            bucket(NOW - 5 * HOUR, 4, 0, 0),
            bucket(NOW - 4 * HOUR, 0, 1, 0),
            bucket(NOW - 3 * HOUR, 0, 0, HOUR),
            bucket(NOW - 2 * HOUR, 0, 0, HOUR),
            bucket(NOW - HOUR, 7, 0, 0),
        ]);
        let model = chart_model(&s, ChartRange::Week, NOW);
        let h = HOUR as f64;
        assert_eq!(
            model.points,
            vec![[-4.5 * h, 4.0], [-3.5 * h, 0.0], [-0.5 * h, 7.0]]
        );
        assert_eq!(model.restarts, vec![-4.0 * h]);
        assert_eq!(model.gaps, vec![(-3.0 * h, -1.0 * h)]);
    }

    #[test]
    fn chart_model_never_plots_a_gap_as_zero() {
        let s = series(vec![
            bucket(NOW - 3 * HOUR, 0, 0, HOUR),
            bucket(NOW - 2 * HOUR, 0, 0, 1_800),
            bucket(NOW - HOUR, 2, 0, 0),
        ]);
        let model = chart_model(&s, ChartRange::Day, NOW);
        assert_eq!(model.points, vec![[-(HOUR as f64) / 2.0, 2.0]]);
    }

    #[test]
    fn a_bucket_with_gap_time_and_real_uses_is_still_plotted() {
        let s = series(vec![bucket(NOW - 2 * HOUR, 40, 0, 300)]);
        let model = chart_model(&s, ChartRange::Week, NOW);
        let h = HOUR as f64;
        assert_eq!(model.points, vec![[-1.5 * h, 40.0]]);
        assert_eq!(model.partial_gaps, vec![(-2.0 * h, -1.0 * h)]);
        assert_eq!(model.lower_bounds, vec![-1.5 * h]);
        assert!(
            model.gaps.is_empty(),
            "only part of the bucket is unobserved"
        );
    }

    #[test]
    fn a_bucket_with_gap_time_and_no_uses_has_no_point() {
        let s = series(vec![
            bucket(NOW - 2 * HOUR, 0, 0, 300),
            bucket(NOW - HOUR, 0, 0, 0),
        ]);
        let model = chart_model(&s, ChartRange::Week, NOW);
        // The observed empty bucket is a real zero; the one with gap time is not plotted.
        assert_eq!(model.points, vec![[-(HOUR as f64) / 2.0, 0.0]]);
        assert!(model.lower_bounds.is_empty());
        // The unobserved part is still shown, lightly.
        let h = HOUR as f64;
        assert_eq!(model.partial_gaps, vec![(-2.0 * h, -1.0 * h)]);
    }

    #[test]
    fn a_fully_unobserved_bucket_is_shaded_whole() {
        let s = series(vec![
            bucket(NOW - 3 * HOUR, 0, 0, HOUR),
            bucket(NOW - 2 * HOUR, 6, 0, HOUR),
        ]);
        let model = chart_model(&s, ChartRange::Day, NOW);
        let h = HOUR as f64;
        assert_eq!(model.gaps, vec![(-3.0 * h, -1.0 * h)]);
        assert!(model.partial_gaps.is_empty());
        // Uses counted over the silence are still shown, as a lower bound.
        assert_eq!(model.points, vec![[-1.5 * h, 6.0]]);
    }

    #[test]
    fn observed_uses_is_none_unless_a_bucket_of_the_window_was_observed() {
        let all_gap = series(vec![bucket(NOW - HOUR, 0, 0, HOUR)]);
        assert_eq!(observed_uses(&all_gap, ChartRange::Day, NOW), None);
        assert_eq!(
            observed_uses(&series(Vec::new()), ChartRange::Day, NOW),
            None
        );
        let outside = series(vec![bucket(NOW - 30 * HOUR, 9, 0, 0)]);
        assert_eq!(observed_uses(&outside, ChartRange::Day, NOW), None);
        let partly = series(vec![bucket(NOW - HOUR, 0, 0, 300)]);
        assert_eq!(observed_uses(&partly, ChartRange::Day, NOW), Some(0));
        let seen = series(vec![
            bucket(NOW - 2 * HOUR, 0, 0, 0),
            bucket(NOW - HOUR, 3, 0, HOUR),
        ]);
        assert_eq!(observed_uses(&seen, ChartRange::Day, NOW), Some(3));
    }

    #[test]
    fn buckets_outside_the_range_are_left_out() {
        let s = series(vec![
            bucket(NOW - 30 * HOUR, 9, 1, 0),
            bucket(NOW - 2 * HOUR, 1, 0, 0),
        ]);
        let model = chart_model(&s, ChartRange::Day, NOW);
        assert_eq!(model.points.len(), 1);
        assert!(model.restarts.is_empty());
    }

    #[test]
    fn the_summary_counts_the_window_only() {
        let s = series(vec![
            bucket(NOW - 30 * HOUR, 9, 1, 0),
            bucket(NOW - 6 * HOUR, 5, 1, 0),
            bucket(NOW - 5 * HOUR, 0, 0, HOUR),
            bucket(NOW - 4 * HOUR, 0, 0, 5_400),
            bucket(NOW - HOUR, 3, 0, 0),
        ]);
        let summary = chart_summary(&s, ChartRange::Day, NOW);
        assert_eq!(
            summary,
            ChartSummary {
                uses: 8,
                restarts: 1,
                gap_hours: 2
            }
        );
    }

    #[test]
    fn an_empty_series_plots_nothing() {
        let model = chart_model(&series(Vec::new()), ChartRange::Day, NOW);
        assert_eq!(model, ChartModel::default());
    }
}
