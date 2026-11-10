//! Fixed states for the bulk update / delete dialog screenshots: each step of the flow over the
//! browser on `crm.customers`.
use super::production_shots::production_root;
use bson::{Document, doc};
use curlybrace_core::bulk::{
    BulkFailureReason, BulkOutcome, BulkRunId, PreviewDoc, PreviewGap, UndoOffer, UndoPreview,
    UndoPreviewDoc, UndoStatus, UpdatePreview,
};
use curlybrace_core::error::CoreError;
use curlybrace_core::snapshot::UndoReport;
use curlybrace_core::store::Namespace;
use curlybrace_ui::app::bulk_state::BulkState;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::{
    BulkFlow, BulkKind, BulkStep, CountState, PreviewState, UndoFlow, UndoPreviewState,
};

const UPDATE: &str = "{\n  \"$set\": { \"status\": \"archived\" }\n}";

fn ns() -> Namespace {
    Namespace::new("crm", "customers")
}

fn filter() -> Document {
    doc! { "status": "inactive", "plan": "free" }
}

fn sample_before(n: i32) -> Document {
    doc! { "_id": n, "name": format!("Customer {n}"), "status": "inactive", "plan": "free" }
}

fn archived(n: i32) -> Document {
    doc! { "_id": n, "name": format!("Customer {n}"), "status": "archived", "plan": "free" }
}

fn with_after() -> PreviewState {
    let docs = (1..=3)
        .map(|n| PreviewDoc {
            before: sample_before(n),
            after: Some(archived(n)),
        })
        .collect();
    PreviewState::Update(UpdatePreview { docs, gap: None })
}

fn without_after() -> PreviewState {
    let docs = (1..=3)
        .map(|n| PreviewDoc {
            before: sample_before(n),
            after: None,
        })
        .collect();
    PreviewState::Update(UpdatePreview {
        docs,
        gap: Some(PreviewGap::TransactionsUnavailable),
    })
}

fn sample() -> PreviewState {
    PreviewState::Sample((1..=3).map(sample_before).collect())
}

/// The browser with `flow` open as the bulk dialog.
fn open(state: &mut AppState, flow: BulkFlow) {
    let session = state.active.expect("browsing opens a session");
    state.bulk = Some(BulkState {
        flow,
        session,
        epoch: state.latest_requested_epoch,
        count_request: None,
        preview_request: None,
        undo_request: None,
    });
}

fn flow(kind: BulkKind, step: BulkStep) -> BulkFlow {
    BulkFlow {
        step,
        // Fixed, so the picture does not depend on the machine it is taken on.
        undo_folder: Some("~/.config/curlybrace/snapshots".into()),
        ..BulkFlow::open(kind, ns(), filter(), true)
    }
}

fn update_flow(step: BulkStep) -> BulkFlow {
    BulkFlow {
        update_text: UPDATE.into(),
        ..flow(BulkKind::Update, step)
    }
}

pub fn define_update(state: &mut AppState) {
    production_root(state);
    open(state, update_flow(BulkStep::Define));
}

pub fn preview_update(state: &mut AppState) {
    production_root(state);
    let f = BulkFlow {
        count: CountState::Known(42),
        preview: with_after(),
        ..update_flow(BulkStep::Preview)
    };
    open(state, f);
}

pub fn preview_update_degraded(state: &mut AppState) {
    production_root(state);
    let f = BulkFlow {
        count: CountState::Known(42),
        preview: without_after(),
        ..update_flow(BulkStep::Preview)
    };
    open(state, f);
}

pub fn preview_delete(state: &mut AppState) {
    production_root(state);
    let f = BulkFlow {
        count: CountState::Known(42),
        preview: sample(),
        ..flow(BulkKind::Delete, BulkStep::Preview)
    };
    open(state, f);
}

/// An empty filter on a production connection: the all-documents tick and the typed name.
pub fn confirm_all_documents(state: &mut AppState) {
    production_root(state);
    let f = BulkFlow {
        filter: doc! {},
        count: CountState::Known(12_480),
        preview: with_after(),
        production: true,
        typed: "custom".into(),
        all_documents_ticked: true,
        ..update_flow(BulkStep::Confirm)
    };
    open(state, f);
}

pub fn running(state: &mut AppState) {
    production_root(state);
    let f = BulkFlow {
        count: CountState::Known(42),
        preview: with_after(),
        typed: "customers".into(),
        run: Some(BulkRunId(uuid::Uuid::nil())),
        ..update_flow(BulkStep::Running)
    };
    open(state, f);
}

fn reported(report: BulkOutcome) -> impl FnOnce(&mut AppState) {
    move |state| {
        production_root(state);
        let f = BulkFlow {
            count: CountState::Known(42),
            preview: with_after(),
            report: Some(Ok(report)),
            ..update_flow(BulkStep::Report)
        };
        open(state, f);
    }
}

pub fn report_applied(state: &mut AppState) {
    let outcome = BulkOutcome::Applied {
        matched: 42,
        modified: 40,
        upserted: vec![],
        deleted: 0,
    };
    reported(outcome)(state);
}

pub fn report_unknown(state: &mut AppState) {
    reported(BulkOutcome::Unknown { cancelled: true })(state);
}

pub fn report_rejected(state: &mut AppState) {
    let reason = BulkFailureReason::DuplicateKey;
    reported(BulkOutcome::Rejected { reason })(state);
}

pub fn report_unconfirmed(state: &mut AppState) {
    let outcome = BulkOutcome::Unconfirmed {
        matched: 42,
        modified: 40,
        upserted: vec![],
        deleted: 0,
    };
    reported(outcome)(state);
}

// ------------------------------------------------------------------ undo

fn applied() -> BulkOutcome {
    BulkOutcome::Applied {
        matched: 42,
        modified: 40,
        upserted: vec![],
        deleted: 0,
    }
}

fn offer(uncovered: u64) -> UndoStatus {
    UndoStatus::Offered(UndoOffer {
        documents: 42,
        uncovered,
    })
}

/// A finished update whose undo copy can restore it.
pub fn report_undo_offered(state: &mut AppState) {
    production_root(state);
    let f = BulkFlow {
        count: CountState::Known(42),
        preview: with_after(),
        report: Some(Ok(applied())),
        undo: offer(3),
        run: Some(BulkRunId(uuid::Uuid::nil())),
        ..update_flow(BulkStep::Report)
    };
    open(state, f);
}

/// The undo copy could not be saved: nothing ran, and the person may go on without undo.
pub fn report_snapshot_refused(state: &mut AppState) {
    production_root(state);
    let f = BulkFlow {
        count: CountState::Known(42),
        preview: with_after(),
        report: Some(Err(CoreError::SnapshotTooLarge)),
        ..update_flow(BulkStep::Report)
    };
    open(state, f);
}

fn undo_preview() -> UndoPreview {
    let docs = (1..=3)
        .map(|n| UndoPreviewDoc {
            original: sample_before(n),
            current: (n != 3).then(|| archived(n)),
        })
        .collect();
    UndoPreview {
        documents: 42,
        docs,
    }
}

fn undoing(undo: UndoFlow) -> impl FnOnce(&mut AppState) {
    move |state| {
        production_root(state);
        let f = BulkFlow {
            count: CountState::Known(42),
            preview: with_after(),
            report: Some(Ok(applied())),
            undo: offer(0),
            run: Some(undo.run),
            undo_flow: Some(undo),
            ..update_flow(BulkStep::Report)
        };
        open(state, f);
    }
}

fn undo_flow() -> UndoFlow {
    UndoFlow::open(BulkRunId(uuid::Uuid::nil()), "customers".into())
}

/// The preview of the restore and the typed collection name, half typed.
pub fn undo_confirm(state: &mut AppState) {
    let flow = undo_flow()
        .with_preview(UndoPreviewState::Ready(undo_preview()))
        .with_typed("custom".into());
    undoing(flow)(state);
}

pub fn undo_report(state: &mut AppState) {
    let report = UndoReport {
        restored: 38,
        skipped_changed: 2,
        skipped_present: 0,
        unchanged: 2,
        errors: 0,
    };
    let flow = undo_flow()
        .with_preview(UndoPreviewState::Ready(undo_preview()))
        .with_typed("customers".into())
        .running()
        .reported(Ok(report));
    undoing(flow)(state);
}

/// The list of saved undo copies, with one of them being asked about.
pub fn saved_copies_list(state: &mut AppState) {
    use curlybrace_core::snapshot::{SnapshotInfo, SnapshotOp};
    use curlybrace_ui::app::saved_copies::SavedCopies;
    production_root(state);
    let copy = |n: u128, collection: &str, op, bytes| SnapshotInfo {
        run: BulkRunId(uuid::Uuid::from_u128(n)),
        ns: Namespace::new("crm", collection),
        op,
        created_unix: 1_700_000_000 + n as i64 * 3_600,
        bytes,
    };
    let list = vec![
        copy(1, "customers", SnapshotOp::Update, 48_213),
        copy(2, "invoices", SnapshotOp::Delete, 2_411_007),
        copy(3, "customers", SnapshotOp::Delete, 912),
    ];
    state.snapshot_dir = Some(std::path::PathBuf::from("/snapshots"));
    state.copies = Some(SavedCopies {
        list: Some(list),
        awaiting: None,
        asking: Some(BulkRunId(uuid::Uuid::from_u128(2))),
    });
}
