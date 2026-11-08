//! What the miss trend of the Queries view plots: misses per bucket summed over all shapes.
//! Pure and egui-free; x runs in seconds relative to the chart's end (0 is the end, the past is
//! negative), exactly like the usage chart of an index.
use curlybrace_core::history::{ChartRange, ShapeBucket, ShapeSummary};
use std::collections::BTreeMap;

/// The marks of the miss chart.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MissChartModel {
    /// `[x, misses]` of each bucket drawn, x at the middle of the bucket.
    pub bars: Vec<[f64; 2]>,
    /// `(from, to)` x of each stretch in which nothing was observed; touching stretches merge.
    pub gaps: Vec<(f64, f64)>,
    /// `(from, to)` x of each bucket that is only partly unobserved; its bar is drawn.
    pub partial_gaps: Vec<(f64, f64)>,
    /// x of each bar whose bucket holds unobserved time, so its count may be too low.
    pub lower_bounds: Vec<f64>,
    /// Misses over the whole window.
    pub total_misses: u64,
}

/// What one range bucket holds so far.
#[derive(Clone, Copy, Debug, Default)]
struct Cell {
    misses: u64,
    observed: bool,
    gap_secs: i64,
}

/// Assumption (checked against `HistoryDb::shapes_for`, `history/shapes_read.rs`): every bucket
/// `shapes_for` returns for a range is exactly the range's bucket width wide and starts on a
/// multiple of it, because the read folds every stored tier (10 minute, hourly, daily rows) into
/// the bucket its start falls in (`bucket_start(row.start_secs, width)`). So a bucket coarser than
/// the range's never reaches this model from the store, and flooring a start to the range width
/// is exact. Buckets of other widths are still summed by their start (the unit tests feed them) but
/// a coarser one would then land in one bar rather than be spread.
///
/// Plots the misses of `summaries` over the last `range` before `now_secs` (the chart's end,
/// anchored to a bucket boundary by the caller). Stored buckets of any width (10 minutes, hours,
/// days: older data is rolled up) are summed into the range's own bucket by the bucket start.
/// A bucket's unobserved time is the longest any shape saw in it (the shapes share one poller, so
/// summing would count the same outage per shape); a bucket with unobserved time and no misses is
/// a gap, never a zero.
pub fn miss_chart_model(
    summaries: &[ShapeSummary],
    range: ChartRange,
    now_secs: i64,
) -> MissChartModel {
    let width = range.bucket_width_secs();
    let from = now_secs - range.secs();
    let mut cells: BTreeMap<i64, Cell> = BTreeMap::new();
    for summary in summaries {
        for (key, cell) in shape_cells(summary, width, from, now_secs) {
            let total = cells.entry(key).or_default();
            total.misses += cell.misses;
            total.observed |= cell.observed;
            total.gap_secs = total.gap_secs.max(cell.gap_secs.min(width));
        }
    }
    marks(&cells, width, now_secs)
}

/// One shape's buckets summed into range buckets (gaps of its narrower buckets add up).
fn shape_cells(
    summary: &ShapeSummary,
    width: i64,
    from: i64,
    now_secs: i64,
) -> BTreeMap<i64, Cell> {
    let mut cells: BTreeMap<i64, Cell> = BTreeMap::new();
    for bucket in &summary.buckets {
        if !overlaps(bucket, from, now_secs) {
            continue;
        }
        let key = bucket.start_secs.div_euclid(width) * width;
        let cell = cells.entry(key.max(from)).or_default();
        cell.misses = cell.misses.saturating_add(bucket.misses);
        cell.observed |= bucket.executions > 0 || bucket.misses > 0;
        cell.gap_secs += i64::try_from(bucket.gap_secs).unwrap_or(i64::MAX);
    }
    cells
}

fn overlaps(bucket: &ShapeBucket, from: i64, now_secs: i64) -> bool {
    bucket.start_secs + bucket.width_secs > from && bucket.start_secs < now_secs
}

fn marks(cells: &BTreeMap<i64, Cell>, width: i64, now_secs: i64) -> MissChartModel {
    let x = |secs: i64| (secs - now_secs) as f64;
    let mut model = MissChartModel::default();
    for (start, cell) in cells {
        let span = (x(*start), x(start + width));
        let middle = (span.0 + span.1) / 2.0;
        if cell.gap_secs >= width {
            push_gap(&mut model.gaps, span);
        } else if cell.gap_secs > 0 {
            model.partial_gaps.push(span);
        }
        let drawn = cell.misses > 0 || (cell.observed && cell.gap_secs == 0);
        if drawn {
            model.bars.push([middle, cell.misses as f64]);
        }
        if cell.gap_secs > 0 && cell.misses > 0 {
            model.lower_bounds.push(middle);
        }
        model.total_misses = model.total_misses.saturating_add(cell.misses);
    }
    model
}

/// Adds `gap`, joining it to the previous one when they touch.
fn push_gap(gaps: &mut Vec<(f64, f64)>, gap: (f64, f64)) {
    match gaps.last_mut() {
        Some(last) if last.1 >= gap.0 => last.1 = last.1.max(gap.1),
        _ => gaps.push(gap),
    }
}

#[cfg(test)]
#[path = "queries_chart_tests.rs"]
mod tests;
