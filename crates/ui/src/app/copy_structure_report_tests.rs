//! The report of options and indexes: pure reducer tests over `ItemDone`.
use super::copy::CopyInput;
use super::copy_state::RunOutcome;
use super::copy_structure_report::{IndexFailureRow, IndexLine, OptionsLine};
use super::copy_tests::{counted, event, input, opened, report, run_of, start_run};
use crate::I18n;
use curlybrace_core::copy::engine::{CopyOutcome, CopyReport};
use curlybrace_core::copy::protocol::CopyEvent;
use curlybrace_core::copy::structure::{
    IndexFailReason, IndexFailure, IndexReport, StructureOutcome, StructureReport,
};

/// A confirmed copy with "also copy indexes and collection options" checked or not.
fn started_with(structure: bool) -> (super::state::AppState, u64) {
    let (s, run) = opened(false);
    let s = counted(s, run);
    let s = match structure {
        true => input(s, CopyInput::ToggleIndexesAndOptions).0,
        false => s,
    };
    let (s, commands) = input(s, CopyInput::Confirm);
    let (copy_run, request) = start_run(&commands);
    assert_eq!(request.with_indexes_and_options, structure);
    let items = super::copy_tests::items_of(&s);
    (
        event(
            s,
            CopyEvent::Planned {
                run: copy_run,
                items,
            },
        )
        .0,
        copy_run,
    )
}

fn with_indexes(outcome: StructureOutcome, indexes: IndexReport) -> CopyReport {
    CopyReport {
        structure: Some(StructureReport { outcome }),
        indexes: Some(indexes),
        ..report(10, CopyOutcome::Completed)
    }
}

fn item_done(index: usize, report: CopyReport, run: u64) -> CopyEvent {
    CopyEvent::ItemDone { run, index, report }
}

#[test]
fn the_index_outcome_is_shown_by_name_with_a_typed_reason() {
    let (s, run) = started_with(true);
    assert!(run_of(&s).structure_requested);
    let first = with_indexes(
        StructureOutcome::Created,
        IndexReport {
            built: vec!["a_1".into(), "b_1".into()],
            ..IndexReport::default()
        },
    );
    let second = with_indexes(
        StructureOutcome::ExistingKept,
        IndexReport {
            built: vec![],
            existing: vec!["a_1".into()],
            failed: vec![IndexFailure {
                name: "u_1".into(),
                reason: IndexFailReason::DuplicateData,
            }],
        },
    );
    let (s, _) = event(s, item_done(0, first, run));
    let (s, _) = event(s, item_done(1, second, run));
    let (s, _) = event(
        s,
        CopyEvent::Done {
            run,
            cancelled: false,
        },
    );
    let run = run_of(&s);
    assert_eq!(run.outcome, Some(RunOutcome::Completed));
    assert_eq!(
        run.options_lines(),
        vec![
            OptionsLine {
                collection: "orders".into(),
                key: "copy.report.options_created"
            },
            OptionsLine {
                collection: "users".into(),
                key: "copy.report.options_kept"
            },
        ]
    );
    assert_eq!(
        run.index_lines(),
        vec![
            IndexLine {
                collection: "orders".into(),
                built: 2,
                existing: 0,
                failed: 0
            },
            IndexLine {
                collection: "users".into(),
                built: 0,
                existing: 1,
                failed: 1
            },
        ]
    );
    assert_eq!(
        run.index_failures(),
        vec![IndexFailureRow {
            collection: "users".into(),
            name: "u_1".into(),
            reason_key: "copy.index_failure.duplicate_data",
        }]
    );
    assert!(!run.indexes_skipped(), "a finished copy built its indexes");
}

#[test]
fn every_message_of_the_index_report_resolves() {
    let i18n = I18n::english();
    let keys = [
        "copy.report.options_created",
        "copy.report.options_kept",
        "copy.report.options_not_plain",
        "copy.report.options_overwrite_missing",
        "copy.report.indexes",
        "copy.report.index_failed",
        "copy.report.indexes_not_built",
        "copy.index_failure.duplicate_data",
        "copy.index_failure.unsupported",
        "copy.index_failure.not_allowed",
        "copy.index_failure.connection_lost",
        "copy.index_failure.rejected",
    ];
    for key in keys {
        assert!(!i18n.t(key).starts_with('⟦'), "{key} is missing");
    }
    let text = i18n.t_with(
        "copy.report.indexes",
        &[
            ("collection", "orders"),
            ("built", "2"),
            ("existing", "1"),
            ("failed", "3"),
        ],
    );
    assert!(text.contains("orders") && text.contains('2') && text.contains('3'));
}

#[test]
fn a_cancelled_copy_says_no_further_indexes_were_built() {
    let (s, run) = started_with(true);
    let (s, _) = event(s, item_done(0, report(5, CopyOutcome::Cancelled), run));
    let (s, _) = event(
        s,
        CopyEvent::Done {
            run,
            cancelled: true,
        },
    );
    let run = run_of(&s);
    assert!(run.indexes_skipped());
    assert!(run.index_lines().is_empty(), "none were built");
}

#[test]
fn without_the_option_nothing_about_indexes_is_shown() {
    let (s, run) = started_with(false);
    assert!(!run_of(&s).structure_requested);
    let (s, _) = event(s, item_done(0, report(5, CopyOutcome::Cancelled), run));
    let (s, _) = event(
        s,
        CopyEvent::Done {
            run,
            cancelled: true,
        },
    );
    let run = run_of(&s);
    assert!(!run.indexes_skipped());
    assert!(run.options_lines().is_empty() && run.index_lines().is_empty());
}
