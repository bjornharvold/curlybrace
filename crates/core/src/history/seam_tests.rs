use super::seam::*;
use super::series::{Sample, accepted_indices};
use crate::error::Result;

fn at(secs: i64) -> Sample {
    Sample {
        at_secs: secs,
        ops: secs,
        since_ms: 1,
    }
}

fn rows(times: &[i64]) -> Vec<Result<(i64, Sample)>> {
    times
        .iter()
        .enumerate()
        .map(|(i, t)| Ok((i as i64 + 1, at(*t))))
        .collect()
}

/// What rolling used to compute from the whole raw window.
fn reference(times: &[i64], cutoff: i64) -> Option<SeamWindow> {
    let plain: Vec<Sample> = times.iter().map(|t| at(*t)).collect();
    let accepted = accepted_indices(&plain);
    let pos = accepted.iter().rposition(|&i| plain[i].at_secs < cutoff)?;
    let next = accepted.get(pos + 1);
    let end = pos + usize::from(next.is_some());
    Some(SeamWindow {
        baseline_id: accepted[pos] as i64 + 1,
        baseline_at: plain[accepted[pos]].at_secs,
        window: accepted[..=end].iter().map(|&i| plain[i]).collect(),
        has_next: next.is_some(),
        oldest_at: plain[accepted[0]].at_secs,
    })
}

#[test]
fn the_streamed_window_equals_the_one_from_the_whole_raw_set() {
    let cases: &[&[i64]] = &[
        &[],
        &[10, 20, 30],
        &[100, 200, 300, 400],
        &[10, 20, 400, 500, 600],
        // Out of order and future-stamped rows: not accepted, in the chain or past the cutoff.
        &[10, 5, 20, 20, 15, 30, 9_000, 40, 400, 500],
        &[10, 9_000, 20, 400],
        &[400, 10, 20],
        &[50, 100, 100, 99, 101],
        &[10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110],
    ];
    for cutoff in [0, 10, 11, 50, 100, 101, 300, 1_000] {
        for case in cases {
            let streamed = seam_window(rows(case).into_iter(), cutoff).unwrap();
            assert_eq!(streamed, reference(case, cutoff), "{case:?} at {cutoff}");
        }
    }
}

#[test]
fn rows_after_the_sample_that_follows_the_baseline_are_never_read() {
    let times: Vec<i64> = (0..10_000).map(|i| i * 60).collect();
    let cutoff = 100 * 60;
    let mut read = 0;
    let counted = rows(&times).into_iter().inspect(|_| read += 1);
    let seam = seam_window(counted, cutoff).unwrap().unwrap();
    assert!(seam.has_next);
    assert_eq!(seam.baseline_at, 99 * 60);
    assert_eq!(read, 101, "the rows before the cutoff and the next one");
}

#[test]
fn a_failing_row_fails_the_window() {
    let broken = vec![
        Ok((1, at(10))),
        Err(crate::error::CoreError::Database("history: boom".into())),
    ];
    assert!(seam_window(broken.into_iter(), 100).is_err());
}
