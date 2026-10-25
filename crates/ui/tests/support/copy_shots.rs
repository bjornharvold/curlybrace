//! Fixtures for the paste dialog and the copy report over the browser.
use super::shots::browsing;
use bson::{Bson, Document};
use curlybrace_core::copy::engine::{CopyFailure, CopyOutcome, CopyProgress, CopyReport};
use curlybrace_core::copy::policy::ConflictPolicy;
use curlybrace_core::copy::preview::PreviewCounts;
use curlybrace_core::copy::protocol::{CopyItem, CopyRequest, CopyScope};
use curlybrace_core::copy::structure::{
    IndexFailReason, IndexFailure, IndexReport, StructureOutcome, StructureReport,
};
use curlybrace_core::store::{Namespace, WriteFailure};
use curlybrace_ui::app::copy_state::{CopyDialog, CopyPhase, CopyRun, RunOutcome};
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::ViewMode;

fn item(name: &str) -> CopyItem {
    CopyItem {
        source: Namespace::new("shop", name),
        target: Namespace::new("archive", name),
        filter: Document::new(),
    }
}

fn items() -> Vec<CopyItem> {
    vec![item("orders"), item("users"), item("invoices")]
}

/// The dialog for three collections pasted onto a database of the open connection.
fn dialog(state: &AppState, policy: ConflictPolicy) -> CopyDialog {
    let session = state.active.expect("browsing opens a session");
    let request = CopyRequest {
        source: session,
        target: session,
        scope: CopyScope::Collections(items()),
        policy,
        with_indexes_and_options: false,
    };
    CopyDialog::opened(request).with_policy(policy)
}

fn counts(new: u64, changed: u64, unchanged: u64) -> Option<PreviewCounts> {
    Some(PreviewCounts {
        new,
        changed,
        unchanged,
    })
}

/// Two collections counted, the third still being counted.
pub fn dialog_counting(state: &mut AppState) {
    browsing(ViewMode::Tree)(state);
    let d = dialog(state, ConflictPolicy::Skip)
        .with_plan(items())
        .counting_run(1)
        .with_count(0, counts(1200, 30, 4500))
        .with_count(1, None)
        .with_scanned(2, 8000);
    state.copy = Some(d);
}

/// Overwrite on a production connection: the target to type is asked for, Copy is off.
pub fn dialog_overwrite_production(state: &mut AppState) {
    super::production_shots::production_root(state);
    let d = dialog(state, ConflictPolicy::Overwrite)
        .with_plan(items())
        .with_count(0, counts(1200, 30, 4500))
        .with_count(1, counts(0, 0, 80))
        .with_count(2, counts(5, 5, 5));
    state.copy = Some(d);
}

fn running_dialog(state: &AppState, run: CopyRun) -> CopyDialog {
    dialog(state, ConflictPolicy::Skip).with_phase(CopyPhase::Running(run))
}

fn report(written: u64, failed: u64, outcome: CopyOutcome, keys: Vec<Bson>) -> CopyReport {
    CopyReport {
        progress: CopyProgress {
            scanned: written + failed,
            written,
            skipped: 0,
            failed,
        },
        failures: keys
            .into_iter()
            .map(|key| CopyFailure {
                key,
                reason: WriteFailure::DuplicateKey,
            })
            .collect(),
        outcome,
        structure: None,
        indexes: None,
    }
}

/// The first collection done, the second halfway, the third waiting.
pub fn progress(state: &mut AppState) {
    browsing(ViewMode::Tree)(state);
    let run = CopyRun::new(2, false)
        .with_plan(&items())
        .with_item_done(0, report(1230, 0, CopyOutcome::Completed, Vec::new()))
        .with_progress(
            1,
            CopyProgress {
                scanned: 4000,
                written: 3900,
                skipped: 80,
                failed: 20,
            },
            Some(10_000),
        );
    state.copy = Some(running_dialog(state, run));
}

/// A finished copy with options and indexes: one collection created with the source's options,
/// one that existed (an index left alone) with a unique index that could not be built.
pub fn report_indexes(state: &mut AppState) {
    browsing(ViewMode::Tree)(state);
    let created = CopyReport {
        structure: Some(StructureReport {
            outcome: StructureOutcome::Created,
        }),
        indexes: Some(IndexReport {
            built: vec!["customer_1".into(), "createdAt_-1".into()],
            ..IndexReport::default()
        }),
        ..report(1230, 0, CopyOutcome::Completed, Vec::new())
    };
    let kept = CopyReport {
        structure: Some(StructureReport {
            outcome: StructureOutcome::ExistingKept,
        }),
        indexes: Some(IndexReport {
            built: vec!["status_1".into()],
            existing: vec!["customer_1".into()],
            failed: vec![IndexFailure {
                name: "email_unique".into(),
                reason: IndexFailReason::DuplicateData,
            }],
        }),
        ..report(80, 0, CopyOutcome::Completed, Vec::new())
    };
    let run = CopyRun::new(2, false)
        .with_structure_requested(true)
        .with_plan(&items()[..2])
        .with_item_done(0, created)
        .with_item_done(1, kept)
        .ended(RunOutcome::Completed);
    state.copy = Some(running_dialog(state, run));
}

/// A cancelled copy: the exact counts, the refused documents by key, incomplete counts.
pub fn report_cancelled(state: &mut AppState) {
    browsing(ViewMode::Tree)(state);
    let keys = vec![Bson::Int32(17), Bson::String("order-9".into())];
    let run = CopyRun::new(2, true)
        .with_plan(&items())
        .with_item_done(0, report(12_300, 2, CopyOutcome::Cancelled, keys))
        .ended(RunOutcome::Cancelled);
    state.copy = Some(running_dialog(state, run));
}
