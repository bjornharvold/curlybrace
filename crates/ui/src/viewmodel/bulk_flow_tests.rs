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
            .with_preview(PreviewState::Failed)
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

#[test]
fn empty_filter_requires_the_all_documents_tick_before_can_run() {
    let base = BulkFlow {
        filter: doc! {},
        ..delete_flow()
    }
    .with_typed("orders".into());
    assert!(base.needs_all_documents_step());
    assert!(!base.can_run());
    assert!(base.with_all_documents(true).can_run());
    assert!(!delete_flow().needs_all_documents_step());
}

#[test]
fn delete_with_empty_filter_without_tick_cannot_run() {
    let f = BulkFlow {
        filter: doc! {},
        ..delete_flow()
    }
    .with_typed("orders".into());
    assert_eq!(f.scope(), MatchScope::Filtered);
    assert!(!f.can_run());
}

#[test]
fn scope_is_all_documents_only_when_filter_empty_and_ticked() {
    let empty = BulkFlow {
        filter: doc! {},
        ..delete_flow()
    };
    assert_eq!(empty.scope(), MatchScope::Filtered);
    assert_eq!(
        empty.with_all_documents(true).scope(),
        MatchScope::AllDocuments
    );
    assert_eq!(
        delete_flow().with_all_documents(true).scope(),
        MatchScope::Filtered
    );
}

#[test]
fn typed_must_match_collection_name_exactly_trimmed() {
    let f = delete_flow();
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
    .with_typed("".into());
    assert!(!f.can_run());
    assert!(!f.with_typed("  ".into()).can_run());
}

#[test]
fn pending_count_blocks_run() {
    assert!(
        !delete_flow()
            .with_count(CountState::Pending)
            .with_typed("orders".into())
            .can_run()
    );
}

#[test]
fn update_can_run_when_valid_and_no_typed_needed() {
    assert!(update_flow().can_run());
    assert!(!update_flow().with_update_text("nope".into()).can_run());
    assert!(
        !update_flow()
            .with_update_text(r#"{"a": 1}"#.into())
            .can_run()
    );
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
fn running_and_reported_move_the_step() {
    let id = BulkRunId(uuid::Uuid::nil());
    let running = delete_flow().running(id);
    assert_eq!((running.step, running.run), (BulkStep::Running, Some(id)));
    let done = running.reported(Ok(BulkOutcome::Unknown { cancelled: false }));
    assert_eq!(done.step, BulkStep::Report);
}

#[test]
fn reported_records_unknown_outcome() {
    let done = delete_flow().reported(Ok(BulkOutcome::Unknown { cancelled: true }));
    assert_eq!(
        done.report,
        Some(Ok(BulkOutcome::Unknown { cancelled: true }))
    );
    let failed = delete_flow().reported(Err(CoreError::BulkInvalidUpdate));
    assert_eq!(failed.report, Some(Err(CoreError::BulkInvalidUpdate)));
}

#[test]
fn every_updater_returns_a_new_value_and_leaves_the_original() {
    let orig = update_flow();
    let snapshot = orig.clone();
    let _ = orig.clone().with_update_text("x".into());
    let _ = orig.clone().with_upsert(true);
    let _ = orig.clone().with_all_documents(true);
    let _ = orig.clone().with_count(CountState::Unavailable);
    let _ = orig.clone().with_preview(PreviewState::Failed);
    let _ = orig.clone().with_typed("t".into());
    let _ = orig.clone().advanced();
    let _ = orig.clone().running(BulkRunId(uuid::Uuid::nil()));
    let _ = orig.clone().reported(Err(CoreError::BulkInvalidUpdate));
    assert_eq!(orig, snapshot);
    assert!(orig.clone().with_upsert(true).upsert);
    assert!(!orig.upsert);
}
