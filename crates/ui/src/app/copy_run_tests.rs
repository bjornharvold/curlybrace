//! The progress, the cancel and the report of a copy: pure reducer tests over the copy events.
use super::copy::CopyInput;
use super::copy_state::CopyPhase;
use super::copy_state::RunOutcome;
use super::copy_tests::{
    B, dialog, event, input, opened, planned, report, run_of, sid, start_run, started,
};
use super::reducer::{Input, update};
use crate::I18n;
use bson::Bson;
use curlybrace_core::compare::apply::MAX_REPORTED_FAILURES;
use curlybrace_core::copy::engine::{CopyFailure, CopyOutcome, CopyProgress};
use curlybrace_core::copy::policy::ConflictPolicy;
use curlybrace_core::copy::protocol::{CopyCommand, CopyEvent};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Command;
use curlybrace_core::store::WriteFailure;

#[test]
fn progress_events_update_the_view() {
    let (s, run) = started(false);
    let progress = CopyProgress {
        scanned: 300,
        written: 250,
        skipped: 40,
        failed: 10,
    };
    let (s, _) = event(
        s,
        CopyEvent::Progress {
            run,
            index: 0,
            progress,
            estimated_total: Some(1000),
        },
    );
    let row = &run_of(&s).rows[0];
    assert_eq!(row.progress, progress);
    assert_eq!(row.estimated_total, Some(1000));
    assert!(!row.finished);
    let (s, _) = event(
        s,
        CopyEvent::ItemDone {
            run,
            index: 0,
            report: report(250, CopyOutcome::Completed),
        },
    );
    assert!(run_of(&s).rows[0].finished);
    let (s, _) = event(
        s,
        CopyEvent::Done {
            run,
            cancelled: false,
        },
    );
    assert_eq!(run_of(&s).outcome, Some(RunOutcome::Completed));
    // Close ends the dialog; Cancel on a finished run is a plain close.
    let (s, commands) = input(s, CopyInput::Close);
    assert!(s.copy.is_none() && commands.is_empty());
}

#[test]
fn cancel_sends_cancel_and_shows_exact_written_counts() {
    let (s, run) = started(false);
    let (s, commands) = input(s, CopyInput::Cancel);
    assert!(matches!(
        commands.as_slice(),
        [Command::Copy(CopyCommand::Cancel { run: r })] if *r == run
    ));
    assert!(run_of(&s).cancel_requested);
    // A second Cancel sends nothing.
    let (s, commands) = input(s, CopyInput::Cancel);
    assert!(commands.is_empty());
    let (s, _) = event(
        s,
        CopyEvent::ItemDone {
            run,
            index: 0,
            report: report(12_300, CopyOutcome::Cancelled),
        },
    );
    let (s, _) = event(
        s,
        CopyEvent::Done {
            run,
            cancelled: true,
        },
    );
    let run = run_of(&s);
    assert_eq!(run.outcome, Some(RunOutcome::Cancelled));
    let lines = run.written_lines();
    assert_eq!(lines[0], ("orders".to_string(), Some(12_300)));
    assert_eq!(lines[1], ("users".to_string(), None), "never started");
    assert!(!run.nothing_written());
}

#[test]
fn superseded_means_the_session_was_lost_and_keeps_the_written_counts() {
    let (s, run) = started(false);
    let (s, _) = event(
        s,
        CopyEvent::ItemDone {
            run,
            index: 0,
            report: report(7, CopyOutcome::Failed(CoreError::Superseded)),
        },
    );
    let (s, _) = event(
        s,
        CopyEvent::Failed {
            run,
            error: CoreError::Superseded,
        },
    );
    let run = run_of(&s);
    assert_eq!(run.outcome, Some(RunOutcome::Failed(CoreError::Superseded)));
    assert!(run.session_lost());
    assert_eq!(run.written_lines()[0].1, Some(7));
}

#[test]
fn closing_a_session_closes_a_dialog_that_wrote_nothing_but_keeps_a_running_copy() {
    let (s, _) = opened(false);
    let (s, _) = update(s, Input::Disconnect(sid(B)));
    assert!(s.copy.is_none());
    let (s, _) = started(false);
    let (s, _) = update(s, Input::Disconnect(sid(B)));
    // A running copy stays on screen: the runtime ends it with Superseded and the report says so.
    assert!(run_of(&s).outcome.is_none());
}

#[test]
fn failure_report_shows_keys_only_and_is_bounded() {
    let (s, run) = started(false);
    let failures: Vec<CopyFailure> = (0..MAX_REPORTED_FAILURES as i32)
        .map(|n| CopyFailure {
            key: Bson::Int32(n),
            reason: WriteFailure::Rejected(121),
        })
        .collect();
    let mut first = report(0, CopyOutcome::Completed);
    first.progress.failed = failures.len() as u64;
    first.failures = failures;
    let (s, _) = event(
        s,
        CopyEvent::ItemDone {
            run,
            index: 0,
            report: first,
        },
    );
    let mut second = report(0, CopyOutcome::Completed);
    second.progress.failed = 5;
    second.failures = vec![
        CopyFailure {
            key: Bson::String("secret value".into()),
            reason: WriteFailure::DuplicateKey,
        };
        5
    ];
    let (s, _) = event(
        s,
        CopyEvent::ItemDone {
            run,
            index: 1,
            report: second,
        },
    );
    let run = run_of(&s);
    let rows = run.failure_rows();
    assert_eq!(rows.len(), MAX_REPORTED_FAILURES, "bounded across items");
    assert!(run.more_failures, "the rest is only counted");
    assert_eq!(rows[0].collection, "orders");
    assert_eq!(rows[0].reason_key, "copy.failure.rejected");
    let i18n = I18n::english();
    // A reason is a key, never the server's code or text.
    assert!(!i18n.t(rows[0].reason_key).contains("121"));
    assert!(!i18n.t(rows[0].reason_key).starts_with('⟦'));
    assert_eq!(rows[0].key, "0");
}

#[test]
fn read_only_error_shows_the_localized_message() {
    let (s, run) = started(false);
    // The core refused it (the connection was locked after the dialog opened).
    let (s, _) = event(
        s,
        CopyEvent::Failed {
            run,
            error: CoreError::ReadOnly,
        },
    );
    let run = run_of(&s);
    let Some(RunOutcome::Failed(error)) = &run.outcome else {
        panic!("{:?}", run.outcome)
    };
    let text = I18n::english().error(error);
    assert_eq!(text, I18n::english().t("errors.read_only"));
    assert!(run.nothing_written());
}

#[test]
fn a_preview_that_fails_keeps_copy_enabled_and_names_the_reason() {
    let (s, run) = opened(false);
    let (s, _) = event(
        s,
        CopyEvent::Failed {
            run,
            error: CoreError::Unreachable,
        },
    );
    let d = dialog(&s);
    assert!(!d.counting);
    assert_eq!(d.preview_error, Some(CoreError::Unreachable));
    assert!(d.can_confirm(false));
}

#[test]
fn a_new_session_state_drops_the_dialog() {
    let (s, _) = opened(false);
    let s = s.session_cleared();
    assert!(s.copy.is_none());
}

/// Confirmed while counting: the preview is being cancelled and the copy waits for its end.
fn starting() -> (super::state::AppState, u64) {
    let (s, run) = opened(false);
    let s = planned(s, run);
    let (s, _) = input(s, CopyInput::Confirm);
    assert!(matches!(dialog(&s).phase, CopyPhase::Starting));
    (s, run)
}

#[test]
fn cancel_in_starting_then_the_late_end_of_the_preview_sends_no_start() {
    let (s, run) = starting();
    let (s, commands) = input(s, CopyInput::Cancel);
    assert!(s.copy.is_none());
    assert!(matches!(
        commands.as_slice(),
        [Command::Copy(CopyCommand::Cancel { .. })]
    ));
    let (s, commands) = event(
        s,
        CopyEvent::Done {
            run,
            cancelled: true,
        },
    );
    assert!(s.copy.is_none() && commands.is_empty());
}

#[test]
fn a_preview_that_fails_in_starting_still_starts_the_copy_flagged_incomplete() {
    let (s, run) = starting();
    let (s, commands) = event(
        s,
        CopyEvent::Failed {
            run,
            error: CoreError::Unreachable,
        },
    );
    let (_, request) = start_run(&commands);
    assert_eq!(request.policy, ConflictPolicy::Skip);
    assert!(run_of(&s).counts_incomplete);
}

#[test]
fn a_second_confirm_in_starting_sends_nothing() {
    let (s, _) = starting();
    let (s, commands) = input(s, CopyInput::Confirm);
    assert!(commands.is_empty());
    assert!(matches!(dialog(&s).phase, CopyPhase::Starting));
}

#[test]
fn a_session_closing_in_starting_drops_the_dialog_and_sends_no_start() {
    let (s, run) = starting();
    let (s, _) = update(s, Input::Disconnect(sid(B)));
    assert!(s.copy.is_none());
    let (s, commands) = event(
        s,
        CopyEvent::Failed {
            run,
            error: CoreError::Superseded,
        },
    );
    assert!(s.copy.is_none() && commands.is_empty());
}

#[test]
fn confirm_with_a_rename_restart_pending_cancels_once_and_starts_with_the_latest_names() {
    let (s, run) = opened(false);
    let s = planned(s, run);
    let (s, commands) = input(
        s,
        CopyInput::SetName {
            index: 0,
            name: "orders2".into(),
        },
    );
    assert_eq!(commands.len(), 1, "the rename cancelled the preview");
    let (s, commands) = input(s, CopyInput::Confirm);
    assert!(
        commands.is_empty(),
        "the preview is already being cancelled"
    );
    assert!(matches!(dialog(&s).phase, CopyPhase::Starting));
    let (s, commands) = event(
        s,
        CopyEvent::Done {
            run,
            cancelled: true,
        },
    );
    let (_, request) = start_run(&commands);
    let curlybrace_core::copy::protocol::CopyScope::Collections(items) = request.scope else {
        panic!()
    };
    assert_eq!(items[0].target.collection, "orders2");
    assert!(!dialog(&s).restart_preview, "no preview follows the copy");
    assert!(run_of(&s).counts_incomplete);
}

#[test]
fn a_read_only_refusal_stops_the_preview_in_flight() {
    let (s, run) = opened(false);
    let mut s = planned(s, run);
    s.parked.get_mut(&sid(B)).unwrap().read_only = true;
    let (_, commands) = input(s, CopyInput::Confirm);
    assert!(matches!(
        commands.as_slice(),
        [Command::Copy(CopyCommand::Cancel { run: r })] if *r == run
    ));
}

#[test]
fn a_rename_then_a_read_only_target_sends_no_second_cancel_and_no_stray_preview() {
    let (s, run) = opened(false);
    let s = planned(s, run);
    let (s, commands) = input(
        s,
        CopyInput::SetName {
            index: 0,
            name: "o".into(),
        },
    );
    assert_eq!(commands.len(), 1, "the rename cancelled the preview");
    let mut s = s;
    s.parked.get_mut(&sid(B)).unwrap().read_only = true;
    let (s, commands) = input(s, CopyInput::Confirm);
    assert!(
        commands.is_empty(),
        "the preview is already being cancelled"
    );
    assert!(!dialog(&s).restart_preview);
    let (s, commands) = event(
        s,
        CopyEvent::Done {
            run,
            cancelled: true,
        },
    );
    assert!(commands.is_empty(), "no preview follows the refusal");
    assert_eq!(
        run_of(&s).outcome,
        Some(RunOutcome::Failed(CoreError::ReadOnly))
    );
}

#[test]
fn edits_are_ignored_unless_the_dialog_is_editing() {
    let edits = [
        CopyInput::SetPolicy(ConflictPolicy::Overwrite),
        CopyInput::ToggleIndexesAndOptions,
        CopyInput::TypeConfirmation("crm".into()),
        CopyInput::SetName {
            index: 0,
            name: "other".into(),
        },
    ];
    let makers: [fn() -> super::state::AppState; 2] = [|| starting().0, || started(false).0];
    for make in makers {
        for edit in &edits {
            let state = make();
            let before = state.copy.clone();
            let (after, commands) = input(state, edit.clone());
            assert_eq!(after.copy, before, "{edit:?}");
            assert!(commands.is_empty(), "{edit:?}");
        }
    }
}

#[test]
fn the_copy_is_not_sent_when_the_confirmation_no_longer_passes() {
    let (s, run) = starting();
    // The target became a production connection while the preview was being cancelled.
    let mut s = s;
    s.parked.get_mut(&sid(B)).unwrap().production = true;
    let (s, commands) = event(
        s,
        CopyEvent::Done {
            run,
            cancelled: true,
        },
    );
    assert!(commands.is_empty(), "{commands:?}");
    assert!(matches!(dialog(&s).phase, CopyPhase::Editing));
}

#[test]
fn the_report_counts_skipped_and_failed_documents_per_collection_and_in_total() {
    let (s, run) = started(false);
    let mut first = report(5, CopyOutcome::Completed);
    first.progress.skipped = 3;
    first.progress.failed = 2;
    let mut second = report(4, CopyOutcome::Completed);
    second.progress.failed = 250;
    let (s, _) = event(
        s,
        CopyEvent::ItemDone {
            run,
            index: 0,
            report: first,
        },
    );
    let (s, _) = event(
        s,
        CopyEvent::ItemDone {
            run,
            index: 1,
            report: second,
        },
    );
    let run = run_of(&s);
    let lines = run.not_written_lines();
    assert_eq!(
        lines
            .iter()
            .map(|l| (l.collection.as_str(), l.skipped, l.failed))
            .collect::<Vec<_>>(),
        [("orders", 3, 2), ("users", 0, 250)]
    );
    assert_eq!(run.failed_total(), 252);
}

#[test]
fn a_clean_collection_has_no_skipped_or_failed_line() {
    let (s, run) = started(false);
    let (s, _) = event(
        s,
        CopyEvent::ItemDone {
            run,
            index: 0,
            report: report(5, CopyOutcome::Completed),
        },
    );
    assert!(run_of(&s).not_written_lines().is_empty());
    assert_eq!(run_of(&s).failed_total(), 0);
}
