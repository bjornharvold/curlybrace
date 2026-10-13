use super::*;
use std::path::PathBuf;

fn draft(kind: ExportKind) -> ExportDraft {
    ExportDraft {
        kind,
        plan: PlanOptions::default(),
        report: ReportOptions::default(),
    }
}

fn running(path: Option<&str>) -> ExportUi {
    ExportUi::Configuring(draft(ExportKind::Report))
        .choosing()
        .started(7, path.map(PathBuf::from))
}

#[test]
fn defaults_are_a_script_that_keeps_ids_with_no_label() {
    let plan = PlanOptions::default();
    assert_eq!(plan.format, ExportFormat::Mongosh);
    assert_eq!(plan.id_policy, IdPolicy::Keep);
    assert!(plan.label.is_empty());
    assert_eq!(ExportUi::default(), ExportUi::Idle);
}

#[test]
fn only_choosing_and_running_are_busy() {
    let modal = ExportUi::Configuring(draft(ExportKind::Plan));
    assert!(!ExportUi::Idle.is_busy() && !modal.is_busy());
    assert!(modal.clone().choosing().is_busy());
    assert!(running(Some("/o")).is_busy());
    let done = running(Some("/o")).finished(7, ExportOutcome::default());
    assert!(!done.is_busy());
}

#[test]
fn edits_apply_only_while_configuring() {
    let relabel = |mut d: ExportDraft| {
        d.plan.label = "x".into();
        d
    };
    let modal = ExportUi::Configuring(draft(ExportKind::Plan));
    let edited = modal.clone().edited(relabel);
    assert_eq!(edited.draft().map(|d| d.plan.label.as_str()), Some("x"));
    let locked = modal.choosing();
    assert_eq!(locked.clone().edited(relabel), locked);
    assert_eq!(ExportUi::Idle.edited(relabel), ExportUi::Idle);
}

#[test]
fn started_needs_a_draft_and_keeps_it_for_the_way_back() {
    assert_eq!(ExportUi::Idle.started(1, None), ExportUi::Idle);
    let state = running(Some("/o"));
    assert_eq!(state.running_run(), Some(7));
    let back = state.reconfiguring();
    assert_eq!(back, ExportUi::Configuring(draft(ExportKind::Report)));
    assert_eq!(ExportUi::Idle.reconfiguring(), ExportUi::Idle);
}

#[test]
fn progress_and_cancel_follow_the_run() {
    let state = running(Some("/o")).progressed(7, 2, 5);
    assert!(matches!(
        state,
        ExportUi::Running {
            done: 2,
            total: 5,
            ..
        }
    ));
    assert_eq!(state.clone().progressed(8, 4, 5), state, "another run");
    assert!(state.can_cancel());
    let cancelling = state.cancelling();
    assert!(!cancelling.can_cancel());
    assert_eq!(cancelling.clone().cancelling(), cancelling);
}

#[test]
fn text_is_kept_for_the_clipboard_run_only() {
    let clip = running(None).text_arrived(7, "t".into());
    assert!(matches!(clip, ExportUi::Running { text: Some(_), .. }));
    let file = running(Some("/o"));
    assert_eq!(file.clone().text_arrived(7, "t".into()), file);
    assert_eq!(clip.clone().text_arrived(8, "u".into()), clip);
}

#[test]
fn finishing_a_clipboard_report_hands_the_text_over_once() {
    let outcome = ExportOutcome {
        written: 3,
        ..ExportOutcome::default()
    };
    let copied = running(None)
        .text_arrived(7, "keys".into())
        .finished(7, outcome);
    let (copied, text) = copied.text_taken();
    assert_eq!(text.as_deref(), Some("keys"));
    assert_eq!(
        copied,
        ExportUi::Copied {
            rows: 3,
            text: None
        }
    );
    assert_eq!(copied.clone().text_taken(), (copied, None));
}

#[test]
fn finished_keeps_the_outcome_kind_and_path() {
    let outcome = ExportOutcome {
        cancelled: true,
        file_left: true,
        ..ExportOutcome::default()
    };
    let state = running(Some("/o")).finished(7, outcome);
    assert_eq!(
        state,
        ExportUi::Finished {
            outcome,
            kind: ExportKind::Report,
            path: Some(RedactedPath::new(PathBuf::from("/o"))),
        }
    );
    let stale = running(Some("/o")).finished(8, outcome);
    assert!(stale.is_running(), "another run's end changes nothing");
}

#[test]
fn closing_leaves_a_job_alone() {
    assert_eq!(
        ExportUi::Configuring(draft(ExportKind::Plan)).closed(),
        ExportUi::Idle
    );
    assert!(running(None).closed().is_running());
    let waiting = ExportUi::Configuring(draft(ExportKind::Plan)).choosing();
    assert_eq!(waiting.clone().closed(), waiting);
}

#[test]
fn the_chosen_path_is_not_in_debug_output_but_is_readable() {
    let state = running(Some("/home/jane.doe/out.csv"));
    assert!(!format!("{state:?}").contains("jane.doe"));
    let done = state.finished(7, ExportOutcome::default());
    assert!(!format!("{done:?}").contains("jane.doe"));
    match done {
        ExportUi::Finished { path: Some(p), .. } => {
            assert_eq!(p.as_path(), std::path::Path::new("/home/jane.doe/out.csv"));
            assert_eq!(format!("{p:?}"), "RedactedPath(..)");
        }
        other => panic!("finished with a path, got {other:?}"),
    }
}

#[test]
fn the_clipboard_text_is_not_in_debug_output() {
    let state = running(None).text_arrived(7, "personal-key".into());
    assert!(!format!("{state:?}").contains("personal-key"));
    assert!(format!("{state:?}").contains("ClipboardText"));
}
