use super::*;
use bson::{Document, doc};
use curlybrace_core::bulk::{
    BulkOutcome, BulkRunId, MatchScope, PreviewDoc, PreviewGap, UpdatePreview,
};
use curlybrace_core::error::CoreError;
use curlybrace_core::store::Namespace;

const GOOD_UPDATE: &str = r#"{"$set": {"a": 1}}"#;

fn ns() -> Namespace {
    Namespace::new("shop", "orders")
}

fn filtered() -> Document {
    doc! { "status": "old" }
}

fn update_flow() -> BulkFlow {
    BulkFlow::open(BulkKind::Update, ns(), filtered(), false)
        .with_update_text(GOOD_UPDATE.into())
        .with_count(CountState::Known(5))
        .with_preview(PreviewState::Update(UpdatePreview {
            docs: vec![PreviewDoc {
                before: doc! { "a": 0 },
                after: Some(doc! { "a": 1 }),
            }],
            gap: None,
        }))
}

fn delete_flow() -> BulkFlow {
    BulkFlow::open(BulkKind::Delete, ns(), filtered(), false)
        .with_count(CountState::Known(5))
        .with_preview(PreviewState::Sample(vec![doc! { "a": 1 }]))
}

fn confirm(flow: BulkFlow) -> BulkFlow {
    let f = flow.advanced().advanced();
    assert_eq!(f.step, BulkStep::Confirm);
    f
}

fn run_id() -> BulkRunId {
    BulkRunId(uuid::Uuid::nil())
}

#[test]
fn open_starts_at_define_with_pending_count() {
    let f = BulkFlow::open(BulkKind::Delete, ns(), filtered(), true);
    assert_eq!(f.step, BulkStep::Define);
    assert_eq!(f.count, CountState::Pending);
    assert_eq!(f.preview, PreviewState::Pending);
    assert!(f.production && f.typed.is_empty() && f.run.is_none());
    assert!(!f.upsert && !f.all_documents_ticked);
}

#[test]
fn threshold_boundary() {
    let at = |c| update_flow().with_count(c).needs_typed();
    assert!(!at(CountState::Known(99)));
    assert!(!at(CountState::Known(100)));
    assert!(at(CountState::Known(101)));
    assert!(at(CountState::Unavailable));
}

#[test]
fn unavailable_count_needs_typed_for_update() {
    assert!(
        update_flow()
            .with_count(CountState::Unavailable)
            .needs_typed()
    );
}

#[test]
fn delete_always_needs_typed() {
    assert!(delete_flow().needs_typed());
    assert!(delete_flow().with_count(CountState::Known(1)).needs_typed());
}

#[test]
fn update_needs_typed_on_production() {
    assert!(!update_flow().needs_typed());
    let prod = BulkFlow::open(BulkKind::Update, ns(), filtered(), true);
    assert!(prod.needs_typed());
}

#[test]
fn update_needs_typed_when_preview_has_no_after() {
    let p = UpdatePreview {
        docs: vec![PreviewDoc {
            before: doc! {},
            after: None,
        }],
        gap: None,
    };
    assert!(
        update_flow()
            .with_preview(PreviewState::Update(p))
            .needs_typed()
    );
}

#[test]
fn needs_typed_for_update_when_preview_failed() {
    assert!(
        update_flow()
            .with_preview(PreviewState::Failed(CoreError::BulkPreviewUnavailable))
            .needs_typed()
    );
}

#[test]
fn needs_typed_when_preview_gap_is_transactions_unavailable() {
    for gap in [PreviewGap::TransactionsUnavailable, PreviewGap::Conflict] {
        let p = UpdatePreview {
            docs: vec![],
            gap: Some(gap),
        };
        assert!(
            update_flow()
                .with_preview(PreviewState::Update(p))
                .needs_typed()
        );
    }
}

#[test]
fn no_match_gap_alone_does_not_need_typed() {
    let p = UpdatePreview {
        docs: vec![],
        gap: Some(PreviewGap::NoMatch),
    };
    assert!(
        !update_flow()
            .with_preview(PreviewState::Update(p))
            .needs_typed()
    );
}

#[test]
fn update_all_documents_needs_typed() {
    let f = update_flow().with_all_documents(true);
    let all = BulkFlow {
        filter: doc! {},
        ..f
    };
    assert!(all.needs_typed());
}

fn empty_delete() -> BulkFlow {
    BulkFlow {
        filter: doc! {},
        ..delete_flow()
    }
}

#[test]
fn empty_filter_requires_the_all_documents_tick_before_can_run() {
    let base = confirm(empty_delete()).with_typed("orders".into());
    assert!(base.needs_all_documents_step());
    assert!(!base.can_run());
    let ticked = base.with_all_documents(true).with_typed("orders".into());
    assert!(ticked.can_run());
    assert!(!delete_flow().needs_all_documents_step());
}

#[test]
fn delete_with_empty_filter_without_tick_cannot_run() {
    let f = confirm(empty_delete()).with_typed("orders".into());
    assert_eq!(f.scope(), MatchScope::Filtered);
    assert!(!f.can_run());
}

#[test]
fn delete_with_empty_filter_and_tick_still_requires_the_collection_name() {
    let f = confirm(empty_delete()).with_all_documents(true);
    assert_eq!(f.scope(), MatchScope::AllDocuments);
    assert!(!f.can_run());
    assert!(f.with_typed("orders".into()).can_run());
}

#[test]
fn scope_is_all_documents_only_when_filter_empty_and_ticked() {
    let empty = empty_delete();
    assert_eq!(empty.scope(), MatchScope::Filtered);
    assert_eq!(
        empty.with_all_documents(true).scope(),
        MatchScope::AllDocuments
    );
}

#[test]
fn tick_on_a_non_empty_filter_keeps_scope_filtered_and_can_run_unchanged() {
    let f = confirm(delete_flow()).with_typed("orders".into());
    let ticked = f
        .clone()
        .with_all_documents(true)
        .with_typed("orders".into());
    assert_eq!(ticked.scope(), MatchScope::Filtered);
    assert_eq!(ticked.can_run(), f.can_run());
}

#[test]
fn typed_must_match_collection_name_exactly_trimmed() {
    let f = confirm(delete_flow());
    assert!(!f.can_run());
    assert!(!f.clone().with_typed("Orders".into()).can_run());
    assert!(!f.clone().with_typed("order".into()).can_run());
    assert!(f.clone().with_typed("  orders ".into()).can_run());
}

#[test]
fn blank_collection_name_never_confirms() {
    let f = BulkFlow::open(
        BulkKind::Delete,
        Namespace::new("shop", ""),
        filtered(),
        false,
    )
    .with_count(CountState::Known(1))
    .with_preview(PreviewState::Sample(vec![]));
    let f = confirm(f).with_typed("".into());
    assert!(!f.can_run());
    assert!(!f.with_typed("  ".into()).can_run());
}

#[test]
fn pending_count_blocks_run() {
    let f = confirm(delete_flow()).with_typed("orders".into());
    assert!(f.can_run());
    let pending = BulkFlow {
        count: CountState::Pending,
        ..f
    };
    assert!(!pending.can_run());
}

#[test]
fn update_can_run_when_valid_and_no_typed_needed() {
    assert!(confirm(update_flow()).can_run());
    let bad = |text: &str| BulkFlow {
        update_text: text.into(),
        ..confirm(update_flow())
    };
    assert!(!bad("nope").can_run());
    assert!(!bad(r#"{"a": 1}"#).can_run());
}

#[test]
fn advance_is_blocked_while_update_text_is_invalid() {
    let bad = update_flow().with_update_text("{".into());
    assert_eq!(bad.advanced().step, BulkStep::Define);
    assert_eq!(update_flow().advanced().step, BulkStep::Preview);
}

#[test]
fn advance_is_blocked_while_count_or_preview_pending() {
    let at_preview = update_flow().advanced();
    assert_eq!(at_preview.step, BulkStep::Preview);
    let no_count = at_preview.clone().with_count(CountState::Pending);
    assert_eq!(no_count.advanced().step, BulkStep::Preview);
    let no_preview = at_preview.clone().with_preview(PreviewState::Pending);
    assert_eq!(no_preview.advanced().step, BulkStep::Preview);
    assert_eq!(at_preview.advanced().step, BulkStep::Confirm);
}

#[test]
fn delete_advances_from_define_without_update_text() {
    assert_eq!(delete_flow().advanced().step, BulkStep::Preview);
}

#[test]
fn advanced_is_a_noop_from_confirm_running_and_report() {
    let c = confirm(delete_flow()).with_typed("orders".into());
    assert_eq!(c.clone().advanced(), c);
    let r = c.running(run_id());
    assert_eq!(r.clone().advanced(), r);
    let done = r.reported(Ok(BulkOutcome::Unknown { cancelled: false }));
    assert_eq!(done.clone().advanced(), done);
}

fn running_flow() -> BulkFlow {
    confirm(delete_flow())
        .with_typed("orders".into())
        .running(run_id())
}

#[test]
fn running_and_reported_move_the_step() {
    let running = running_flow();
    assert_eq!(
        (running.step, running.run),
        (BulkStep::Running, Some(run_id()))
    );
    let done = running.reported(Ok(BulkOutcome::Unknown { cancelled: false }));
    assert_eq!(done.step, BulkStep::Report);
}

#[test]
fn reported_records_unknown_outcome() {
    let done = running_flow().reported(Ok(BulkOutcome::Unknown { cancelled: true }));
    assert_eq!(
        done.report,
        Some(Ok(BulkOutcome::Unknown { cancelled: true }))
    );
    let failed = running_flow().reported(Err(CoreError::BulkInvalidUpdate));
    assert_eq!(failed.report, Some(Err(CoreError::BulkInvalidUpdate)));
}

#[test]
fn reported_requires_running() {
    let c = confirm(delete_flow());
    assert_eq!(c.clone().reported(Err(CoreError::BulkInvalidUpdate)), c);
}

#[test]
fn running_requires_can_run() {
    let untyped = confirm(delete_flow());
    assert_eq!(untyped.clone().running(run_id()), untyped);
}

#[test]
fn running_from_define_is_a_noop() {
    let f = update_flow();
    assert_eq!(f.clone().running(run_id()), f);
}

#[test]
fn second_run_after_report_is_impossible() {
    let done = running_flow().reported(Ok(BulkOutcome::Unknown { cancelled: false }));
    assert!(!done.can_run());
    assert_eq!(done.clone().running(run_id()), done);
}

#[test]
fn editing_update_text_after_define_is_ignored() {
    let p = update_flow().advanced();
    assert_eq!(p.clone().with_update_text("{}".into()), p);
    let c = confirm(update_flow());
    assert_eq!(c.clone().with_update_text("{}".into()), c);
}

#[test]
fn upsert_change_after_define_is_ignored() {
    let p = update_flow().advanced();
    assert_eq!(p.clone().with_upsert(true), p);
    assert!(update_flow().with_upsert(true).upsert);
}

#[test]
fn back_from_preview_resets_preview_and_typed() {
    let p = update_flow().advanced().back();
    assert_eq!(p.step, BulkStep::Define);
    assert_eq!(p.preview, PreviewState::Pending);
    assert!(p.typed.is_empty());
}

#[test]
fn back_from_confirm_keeps_preview_clears_typed() {
    let c = confirm(delete_flow()).with_typed("orders".into());
    let b = c.clone().back();
    assert_eq!(b.step, BulkStep::Preview);
    assert_eq!(b.preview, c.preview);
    assert_eq!(b.count, c.count);
    assert!(b.typed.is_empty());
}

#[test]
fn back_is_a_noop_in_define_running_and_report() {
    let d = delete_flow();
    assert_eq!(d.clone().back(), d);
    let r = running_flow();
    assert_eq!(r.clone().back(), r);
    let done = r.reported(Ok(BulkOutcome::Unknown { cancelled: false }));
    assert_eq!(done.clone().back(), done);
}

fn typed_confirm() -> BulkFlow {
    confirm(delete_flow()).with_typed("orders".into())
}

#[test]
fn with_all_documents_change_clears_typed() {
    let f = typed_confirm();
    assert!(f.clone().with_all_documents(true).typed.is_empty());
    assert_eq!(f.clone().with_all_documents(false), f);
}

#[test]
fn with_count_change_clears_typed() {
    assert!(
        typed_confirm()
            .with_count(CountState::Known(6))
            .typed
            .is_empty()
    );
}

#[test]
fn same_count_keeps_typed() {
    let f = typed_confirm();
    assert_eq!(f.clone().with_count(CountState::Known(5)), f);
    assert_eq!(f.typed, "orders");
}

#[test]
fn with_preview_change_clears_typed() {
    assert!(
        typed_confirm()
            .with_preview(PreviewState::Failed(CoreError::BulkPreviewUnavailable))
            .typed
            .is_empty()
    );
}

#[test]
fn typed_is_ignored_outside_confirm() {
    let d = delete_flow();
    assert_eq!(d.clone().with_typed("orders".into()), d);
    let p = delete_flow().advanced();
    assert_eq!(p.clone().with_typed("orders".into()), p);
}

#[test]
fn can_run_is_false_outside_confirm() {
    let c = typed_confirm();
    assert!(c.can_run());
    for step in [
        BulkStep::Define,
        BulkStep::Preview,
        BulkStep::Running,
        BulkStep::Report,
    ] {
        assert!(!BulkFlow { step, ..c.clone() }.can_run(), "{step:?}");
    }
}

#[test]
fn can_run_is_false_while_preview_pending() {
    let f = BulkFlow {
        preview: PreviewState::Pending,
        ..typed_confirm()
    };
    assert!(!f.can_run());
}

#[test]
fn updaters_are_inert_while_running_and_after_report() {
    let running = running_flow();
    let done = running_flow().reported(Ok(BulkOutcome::Unknown { cancelled: false }));
    for f in [running, done] {
        assert_eq!(f.clone().with_update_text("{}".into()), f);
        assert_eq!(f.clone().with_upsert(true), f);
        assert_eq!(f.clone().with_all_documents(true), f);
        assert_eq!(f.clone().with_count(CountState::Unavailable), f);
        assert_eq!(
            f.clone()
                .with_preview(PreviewState::Failed(CoreError::BulkPreviewUnavailable)),
            f
        );
        assert_eq!(f.clone().with_typed("x".into()), f);
    }
}

#[test]
fn every_updater_returns_a_new_value_and_leaves_the_original() {
    let orig = update_flow();
    let snapshot = orig.clone();
    let _ = orig.clone().with_update_text("x".into());
    let _ = orig.clone().with_upsert(true);
    let _ = orig.clone().with_all_documents(true);
    let _ = orig.clone().with_count(CountState::Unavailable);
    let _ = orig
        .clone()
        .with_preview(PreviewState::Failed(CoreError::BulkPreviewUnavailable));
    let _ = orig.clone().with_typed("t".into());
    let _ = orig.clone().advanced();
    let _ = orig.clone().back();
    let _ = orig.clone().running(run_id());
    let _ = orig.clone().reported(Err(CoreError::BulkInvalidUpdate));
    assert_eq!(orig, snapshot);
    assert!(orig.clone().with_upsert(true).upsert);
    assert!(!orig.upsert);
}

#[test]
fn a_cancel_note_is_recorded_only_while_running_and_the_latest_wins() {
    let confirm = confirm(delete_flow().with_typed("orders".into()));
    assert_eq!(confirm.cancel_note, CancelNote::None);
    let ignored = confirm.clone().with_cancel_note(CancelNote::Unavailable);
    assert_eq!(ignored.cancel_note, CancelNote::None);
    let running = confirm.with_typed("orders".into()).running(run_id());
    assert_eq!(running.step, BulkStep::Running);
    assert_eq!(running.cancel_note, CancelNote::None);
    let nothing = running.clone().with_cancel_note(CancelNote::NothingToStop);
    assert_eq!(nothing.cancel_note, CancelNote::NothingToStop);
    let requested = nothing.clone().with_cancel_note(CancelNote::Requested);
    assert_eq!(requested.cancel_note, CancelNote::Requested);
    // Immutable: the earlier values did not change.
    assert_eq!(nothing.cancel_note, CancelNote::NothingToStop);
    assert_eq!(running.cancel_note, CancelNote::None);
}

#[test]
fn can_advance_agrees_with_advanced_in_every_step() {
    let pending = BulkFlow::open(BulkKind::Update, ns(), filtered(), false);
    let invalid = pending.clone().with_update_text("{".into());
    let mut flows = vec![pending.clone(), invalid, update_flow(), delete_flow()];
    for flow in [update_flow(), delete_flow()] {
        let preview = flow.clone().advanced();
        flows.push(preview.clone().with_count(CountState::Pending));
        flows.push(preview.clone().with_preview(PreviewState::Pending));
        flows.push(preview.clone());
        flows.push(preview.advanced());
        flows.push(confirm(flow));
    }
    for flow in flows {
        let moved = flow.clone().advanced().step != flow.step;
        assert_eq!(flow.can_advance(), moved, "{:?}", flow.step);
    }
    assert!(!pending.can_advance(), "empty update");
    assert!(update_flow().can_advance());
}

fn invalid_preview() -> PreviewState {
    PreviewState::Failed(CoreError::BulkInvalidUpdate)
}

#[test]
fn a_preview_that_proved_the_update_invalid_blocks_run_even_when_typed() {
    let at_confirm = confirm(update_flow()).with_preview(invalid_preview());
    let typed = at_confirm.with_typed("orders".into());
    assert!(typed.needs_typed());
    assert!(!typed.can_run());
    assert_eq!(typed.clone().running(run_id()), typed);
}

#[test]
fn another_failed_preview_still_runs_once_the_name_is_typed() {
    let unavailable = PreviewState::Failed(CoreError::BulkPreviewUnavailable);
    let f = confirm(update_flow()).with_preview(unavailable);
    assert!(f.needs_typed());
    assert!(!f.can_run());
    assert!(f.with_typed("orders".into()).can_run());
}

#[test]
fn next_from_preview_is_blocked_when_the_update_was_proved_invalid_and_back_stays() {
    let at_preview = update_flow().advanced();
    let invalid = at_preview.clone().with_preview(invalid_preview());
    assert!(!invalid.can_advance());
    assert_eq!(invalid.clone().advanced().step, BulkStep::Preview);
    assert_eq!(invalid.back().step, BulkStep::Define);
    let unavailable =
        at_preview.with_preview(PreviewState::Failed(CoreError::BulkPreviewUnavailable));
    assert!(unavailable.can_advance());
}

#[test]
fn only_the_invalid_update_error_marks_the_preview_as_proved_invalid() {
    assert!(
        update_flow()
            .with_preview(invalid_preview())
            .preview_proved_invalid()
    );
    assert!(!update_flow().preview_proved_invalid());
    let other = update_flow().with_preview(PreviewState::Failed(CoreError::Unreachable));
    assert!(!other.preview_proved_invalid());
}
