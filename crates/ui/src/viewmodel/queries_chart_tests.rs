//! The miss trend: misses per bucket over all shapes, gaps as gaps.
use super::*;
use curlybrace_core::explain::VerdictKind;
use curlybrace_core::history::{ShapeBucket, ShapeSource, ShapeSummary};

const NOW: i64 = 1_000_800;
const TEN: i64 = 600;
const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;

fn bucket(start: i64, width: i64, misses: u64, gap_secs: u64) -> ShapeBucket {
    ShapeBucket {
        start_secs: start,
        width_secs: width,
        executions: misses + 1,
        misses,
        gap_secs,
    }
}

fn shape(buckets: Vec<ShapeBucket>) -> ShapeSummary {
    ShapeSummary {
        hash: "h".into(),
        shape_json: "{}".into(),
        source: ShapeSource::App,
        executions: 0,
        misses: 0,
        total_millis: 0,
        max_millis: 0,
        worst: VerdictKind::CollectionScan,
        latest: VerdictKind::CollectionScan,
        buckets,
    }
}

#[test]
fn miss_chart_counts_misses_per_bucket_and_marks_gaps() {
    let a = shape(vec![
        bucket(NOW - 3 * TEN, TEN, 4, 0),
        bucket(NOW - 2 * TEN, TEN, 0, TEN as u64),
        bucket(NOW - TEN, TEN, 1, 0),
    ]);
    let b = shape(vec![bucket(NOW - 3 * TEN, TEN, 2, 0)]);
    let model = miss_chart_model(&[a, b], ChartRange::Day, NOW);
    let x = |start: i64| (start + TEN / 2 - NOW) as f64;
    // Summed over both shapes; the wholly unobserved bucket is a gap, not a zero bar.
    assert_eq!(
        model.bars,
        vec![[x(NOW - 3 * TEN), 6.0], [x(NOW - TEN), 1.0]]
    );
    assert_eq!(
        model.gaps,
        vec![((NOW - 2 * TEN - NOW) as f64, (NOW - TEN - NOW) as f64)]
    );
    assert!(model.partial_gaps.is_empty());
    assert_eq!(model.total_misses, 7);
}

#[test]
fn a_partly_observed_bucket_keeps_its_bar_as_a_lower_bound() {
    let a = shape(vec![bucket(NOW - TEN, TEN, 3, 100)]);
    let model = miss_chart_model(&[a], ChartRange::Day, NOW);
    let middle = (-TEN / 2) as f64;
    assert_eq!(model.bars, vec![[middle, 3.0]]);
    assert_eq!(model.partial_gaps, vec![(-TEN as f64, 0.0)]);
    assert_eq!(model.lower_bounds, vec![middle]);
}

#[test]
fn buckets_of_mixed_widths_are_summed_into_the_range_buckets() {
    let hour = NOW - NOW.rem_euclid(HOUR) - HOUR;
    let a = shape(vec![
        bucket(hour, TEN, 1, 0),
        bucket(hour + TEN, TEN, 2, 0),
        bucket(hour + 2 * TEN, TEN, 3, 0),
    ]);
    let b = shape(vec![bucket(hour, HOUR, 10, 0)]);
    let week = miss_chart_model(&[a.clone(), b], ChartRange::Week, NOW + HOUR);
    assert_eq!(week.bars.len(), 1);
    assert_eq!(week.bars[0][1], 16.0);

    let day = NOW - NOW.rem_euclid(DAY) - DAY;
    let hourly = shape(vec![
        bucket(day, HOUR, 2, 0),
        bucket(day + HOUR, HOUR, 5, 0),
    ]);
    let month = miss_chart_model(&[hourly], ChartRange::Month, NOW + DAY);
    assert_eq!(month.bars.len(), 1);
    assert_eq!(month.bars[0][1], 7.0);
}

#[test]
fn gaps_of_narrow_buckets_add_up_inside_a_range_bucket() {
    let hour = NOW - NOW.rem_euclid(HOUR) - HOUR;
    let narrow: Vec<ShapeBucket> = (0..6)
        .map(|i| bucket(hour + i * TEN, TEN, 0, TEN as u64))
        .collect();
    let model = miss_chart_model(&[shape(narrow)], ChartRange::Week, NOW + HOUR);
    assert_eq!(model.gaps.len(), 1);
    assert!(model.bars.is_empty());
}

#[test]
fn no_data_is_empty_not_zero() {
    assert_eq!(
        miss_chart_model(&[], ChartRange::Day, NOW),
        MissChartModel::default()
    );
    let empty = shape(vec![]);
    assert_eq!(
        miss_chart_model(&[empty], ChartRange::Week, NOW),
        MissChartModel::default()
    );
}

#[test]
fn buckets_outside_the_range_are_dropped() {
    let old = shape(vec![bucket(NOW - DAY - 2 * TEN, TEN, 9, 0)]);
    let future = shape(vec![bucket(NOW, TEN, 9, 0)]);
    let inside = shape(vec![bucket(NOW - TEN, TEN, 1, 0)]);
    let model = miss_chart_model(&[old, future, inside], ChartRange::Day, NOW);
    assert_eq!(model.bars.len(), 1);
    assert_eq!(model.total_misses, 1);
}
