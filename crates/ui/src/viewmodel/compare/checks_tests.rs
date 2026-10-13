use super::*;
use curlybrace_core::store::IndexStatus;

fn pre(count: u64, indexed: bool, too_large: bool) -> Preflight {
    let index = if indexed {
        IndexStatus::Indexed
    } else {
        IndexStatus::NotIndexed
    };
    Preflight {
        source_count: count,
        target_count: count,
        source_index: index,
        target_index: index,
        unindexed_too_large: too_large,
    }
}

#[test]
fn queue_sends_one_at_a_time_in_order() {
    let queue = CheckQueue::queued(&[3, 1, 2]);
    assert_eq!(queue.next_to_send(), Some(1));
    let queue = queue.sending(1, 10);
    assert_eq!(queue.in_flight(), Some((1, 10)));
    let (queue, fresh) = queue.answered(10, pre(5, true, false));
    assert!(fresh);
    assert_eq!(queue.next_to_send(), Some(2));
    let queue = queue.sending(2, 11);
    let (queue, _) = queue.answered(11, pre(5, true, false));
    assert_eq!(queue.next_to_send(), Some(3));
}

#[test]
fn next_to_send_is_none_while_one_is_checking() {
    let queue = CheckQueue::queued(&[1, 2]).sending(1, 7);
    assert_eq!(queue.next_to_send(), None);
}

#[test]
fn answer_for_an_unknown_probe_is_stale() {
    let queue = CheckQueue::queued(&[1]).sending(1, 7);
    let (after, fresh) = queue.clone().answered(8, pre(5, true, false));
    assert!(!fresh);
    assert_eq!(after, queue);
    let (idle, fresh) = CheckQueue::default().answered(7, pre(5, true, false));
    assert!(!fresh);
    assert_eq!(idle, CheckQueue::default());
}

#[test]
fn answered_stores_the_preflight_verdict() {
    let answer = pre(5, true, false);
    let (queue, _) = CheckQueue::queued(&[1])
        .sending(1, 7)
        .answered(7, answer.clone());
    assert_eq!(queue.verdict(1), Some(&PreflightView::Ok));
    assert_eq!(queue.preflight(1), Some(&answer));
    assert_eq!(queue.in_flight(), None);
    assert_eq!(queue.verdict(2), None);
}

#[test]
fn failed_marks_only_the_in_flight_entry() {
    let queue = CheckQueue::queued(&[1, 2]).sending(1, 7).failed(7);
    assert_eq!(queue.entries[&1], EntryCheck::Failed);
    assert_eq!(queue.entries[&2], EntryCheck::Queued);
    let untouched = queue.clone().failed(99);
    assert_eq!(untouched, queue);
}

#[test]
fn invalidated_resets_everything_including_in_flight() {
    let (queue, _) = CheckQueue::queued(&[1, 2, 3])
        .sending(1, 7)
        .answered(7, pre(5, true, false));
    let queue = queue.sending(2, 8).invalidated();
    assert!(queue.entries.values().all(|c| *c == EntryCheck::Unchecked));
    assert_eq!(queue.in_flight(), None);
    let (late, fresh) = queue.answered(8, pre(5, true, false));
    assert!(!fresh, "the answer of an invalidated probe is stale");
    assert_eq!(late.verdict(2), None);
}

#[test]
fn all_settled_needs_every_id() {
    let (queue, _) = CheckQueue::queued(&[1, 2])
        .sending(1, 7)
        .answered(7, pre(5, true, false));
    assert!(queue.all_settled(&[1]));
    assert!(!queue.all_settled(&[1, 2]));
    let queue = queue.sending(2, 8).failed(8);
    assert!(queue.all_settled(&[1, 2]));
    assert!(
        !queue.all_settled(&[1, 2, 3]),
        "an unknown id is not settled"
    );
    assert!(queue.all_settled(&[]));
}

#[test]
fn refused_verdict_does_not_allow_start() {
    let (queue, _) = CheckQueue::queued(&[1])
        .sending(1, 7)
        .answered(7, pre(9, false, true));
    assert!(!queue.allows_start(1));
    assert!(!queue.allows_start(2), "unchecked");
}

#[test]
fn empty_both_allows_start() {
    let (queue, _) = CheckQueue::queued(&[1])
        .sending(1, 7)
        .answered(7, pre(0, true, false));
    assert_eq!(queue.verdict(1), Some(&PreflightView::EmptyBoth));
    assert!(queue.allows_start(1));
}

#[test]
fn pending_covers_queued_and_checking_only() {
    assert!(!CheckQueue::default().has_pending());
    let queue = CheckQueue::queued(&[1, 2]);
    assert!(queue.has_pending());
    let (queue, _) = queue.sending(1, 7).answered(7, pre(1, true, false));
    assert!(queue.has_pending());
    let queue = queue.sending(2, 8).failed(8);
    assert!(!queue.has_pending());
}

#[test]
fn a_skipped_entry_ends_as_failed_and_the_queue_moves_on() {
    let queue = CheckQueue::queued(&[1, 2]).skipped(1);
    assert_eq!(queue.entries.get(&1), Some(&EntryCheck::Failed));
    assert_eq!(queue.next_to_send(), Some(2));
    // Only a queued entry can be skipped.
    let checking = CheckQueue::queued(&[1]).sending(1, 5).skipped(1);
    assert_eq!(checking.in_flight(), Some((1, 5)));
}
