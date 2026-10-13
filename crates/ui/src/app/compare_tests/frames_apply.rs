//! Headless pseudo-locale frames of the pending panel, the review dialog and the running and
//! finished panels: every string comes from the translation layer.
use super::confirm::{key, reviewing, with_delete_pending, with_pending};
use super::frames::{assert_translated, has, workspace};
use super::*;
use crate::viewmodel::compare::{ApplyRow, ApplyUi, CompareState};
use curlybrace_core::compare::apply::{ApplyReport, OpFailure};

fn with_apply(state: AppState, apply: ApplyUi) -> AppState {
    AppState {
        compare: CompareState {
            apply,
            ..state.compare
        },
        ..state
    }
}

fn failing(n: i32, error: CoreError) -> OpFailure {
    OpFailure { key: key(n), error }
}

#[test]
fn the_pending_panel_is_translated() {
    let texts = assert_translated(&workspace(with_pending()), "pending panel");
    assert!(has(&texts, "Pending changes"));
    assert!(has(&texts, "Copy to"));
    assert!(has(&texts, "Documents in total: 2"));
    assert!(has(&texts, "Review changes…"));
    assert!(has(&texts, "Clear"));
}

#[test]
fn a_pending_panel_whose_review_is_closed_names_the_reason() {
    let mut state = workspace(with_pending());
    state.compare.overview = state.compare.overview.clone().marked_applied(RUN_PAIR);
    let texts = assert_translated(&state, "review closed");
    assert!(has(&texts, "Run it again first"));
}

#[test]
fn the_review_dialog_is_translated() {
    let texts = assert_translated(&workspace(reviewing(with_pending())), "review dialog");
    assert!(has(&texts, "Review before applying"));
    assert!(has(&texts, "Documents to write: 2"));
    assert!(has(&texts, "Apply"));
}

#[test]
fn the_dialog_with_inserts_warnings_and_a_confirmation_is_translated() {
    let state = reviewing(with_delete_pending());
    let texts = assert_translated(&workspace(state), "delete dialog");
    assert!(has(&texts, "To confirm the deletes, type"));
    let inserts = super::confirm::pending_for(Category::OnlyInSource, RowAction::CopyToTarget);
    let (inserts, _) = update(
        reviewing(inserts),
        Input::Compare(CompareAction::SetIdPolicy(
            curlybrace_core::compare::apply::IdPolicy::Generate,
        )),
    );
    let texts = assert_translated(&workspace(inserts), "insert dialog");
    assert!(has(&texts, "Keep the original _id values"));
    assert!(has(&texts, "New _id values are generated"));
}

#[test]
fn the_running_panel_is_translated() {
    let rows = vec![ApplyRow {
        pair: RUN_PAIR,
        entry: ENTRY,
        done: 1,
        total: 2,
        report: None,
    }];
    let running = ApplyUi::Running {
        run: 9,
        total: 2,
        rows,
        cancel_requested: true,
    };
    let texts = assert_translated(&workspace(with_apply(with_pending(), running)), "running");
    assert!(has(&texts, "Applying changes"));
    assert!(has(&texts, "1 of 2"));
    assert!(has(&texts, "Cancel apply"));
    assert!(has(&texts, "Cancelling…"));
}

#[test]
fn the_finished_panel_is_translated() {
    let report = ApplyReport {
        applied: 1,
        failed: 2,
        failures: vec![
            failing(1, CoreError::NotFound),
            failing(2, CoreError::ApplyOutcomeUnknown),
        ],
        more_failures: true,
        ..ApplyReport::default()
    };
    let finished = ApplyUi::Finished {
        reports: vec![(RUN_PAIR, report)],
        cancelled: true,
        requested: 4,
    };
    let texts = assert_translated(&workspace(with_apply(with_pending(), finished)), "finished");
    assert!(has(&texts, "Apply finished"));
    assert!(has(&texts, "Applied: 1"));
    assert!(has(
        &texts,
        "Skipped, the document changed since the compare: 0"
    ));
    assert!(has(&texts, "Not run: 1"));
    assert!(has(&texts, "The apply was cancelled"));
    assert!(has(&texts, "More failures are not shown"));
    assert!(has(&texts, "did not confirm the write"));
    assert!(has(&texts, "Run these pairs again to see what is left"));
    assert!(has(&texts, "Dismiss"));
}
