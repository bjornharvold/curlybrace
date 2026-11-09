//! Fixed states for the bulk update / delete dialog screenshots: each step of the flow over the
//! browser on `crm.customers`.
use super::production_shots::production_root;
use bson::{Document, doc};
use curlybrace_core::bulk::{BulkOutcome, BulkRunId, PreviewDoc, PreviewGap, UpdatePreview};
use curlybrace_core::store::Namespace;
use curlybrace_ui::app::bulk_state::BulkState;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::{BulkFlow, BulkKind, BulkStep, CountState, PreviewState};

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
    });
}

fn flow(kind: BulkKind, step: BulkStep) -> BulkFlow {
    BulkFlow {
        step,
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
