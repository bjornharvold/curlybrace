//! Screenshot states for field-level apply, scripted through the real reducer on the three-pair
//! run of `compare_apply_states::shots`: `customers` has the different rows 17, 42 and 108.
use super::compare_apply_states::shots::{confirm_pending, customers_open, report_of};
use super::compare_field_states::{CITY, NAME, changes, path_of};
use super::compare_multi_states as multi;
use super::compare_states as cs;
use bson::doc;
use curlybrace_core::compare::apply::ApplyReport;
use curlybrace_core::compare::protocol::CompareEvent;
use curlybrace_ui::app::compare::CompareAction;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::compare::RowAction;

/// Document 17 of `customers` open with its changes (an addressable few, a `$` field, a field
/// with a dot in its name and an unordered-array element), nothing ticked.
fn detail_17(state: &mut AppState) {
    customers_open(state);
    open_detail(state, 17);
}

fn open_detail(state: &mut AppState, n: i32) {
    cs::act(state, CompareAction::Select(cs::key(n)));
    let detail = CompareEvent::Detail {
        session: cs::SESSION,
        pair: multi::pair_of(state, 0),
        key: cs::key(n),
        source: Some(doc! {"_id": n, "name": "Ada Lovelace", "address": {"city": "London"}}),
        target: Some(doc! {"_id": n, "name": "Ada King", "address": {"city": "Paris"}}),
        changes: changes(),
    };
    cs::event(state, detail);
}

fn tick(state: &mut AppState, text: &str) {
    cs::act(state, CompareAction::ToggleField(path_of(text)));
}

/// Two fields ticked, the `$` and dotted fields blocked, the queue buttons visible.
pub fn detail_with_field_checkboxes(state: &mut AppState) {
    detail_17(state);
    tick(state, NAME);
    tick(state, CITY);
}

fn queue_17(state: &mut AppState) {
    detail_with_field_checkboxes(state);
    cs::act(
        state,
        CompareAction::AddFieldsToPending(RowAction::CopyFieldsToTarget),
    );
}

/// `customers`: two fields of document 17 queued next to a whole-document copy of 42. Back on
/// the overview.
pub fn pending_with_fields(state: &mut AppState) {
    queue_17(state);
    cs::act(state, CompareAction::ToggleRow(cs::key(42)));
    cs::act(state, CompareAction::AddToPending(RowAction::CopyToTarget));
    cs::act(state, CompareAction::BackToOverview);
}

/// The review of fields next to a whole-document copy: the Fields column, the fields line and
/// the warning, with no typed confirmation.
pub fn review_with_fields(state: &mut AppState) {
    pending_with_fields(state);
    cs::act(
        state,
        CompareAction::ReviewApply {
            word: "delete".into(),
        },
    );
    let review = state.compare.apply.review().expect("the review is open");
    assert!(review.required_text("delete").is_none(), "no typed word");
}

/// The fields and the document applied: 2 written, 1 stale.
pub fn finished_with_field_updates(state: &mut AppState) {
    pending_with_fields(state);
    let run = confirm_pending(state);
    let report = ApplyReport {
        applied: 1,
        stale: 1,
        ..ApplyReport::default()
    };
    report_of(state, run, 0, report);
    cs::event(
        state,
        CompareEvent::ApplyDone {
            session: cs::SESSION,
            run,
        },
    );
}
