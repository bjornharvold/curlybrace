//! The file lifecycle of a report: cancel, close, panic, a write error, the shared job slot and
//! permissions. Gates on the in-memory store make the timing deterministic.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::apply_tests::{Gate, assert_silent, barrier, within};
use super::export_sink::test_hooks::fail_writes_after;
use super::export_tests::{collect_export, outcome_of, request, send_export, standard_ops};
use super::report_tests::{
    file_report, file_target, report, report_request, report_rig, send_report, text_of,
};
use crate::compare::protocol::{CompareCommand, CompareEvent, ExportFormat, ReportTarget};
use crate::error::CoreError;
use crate::runtime::Command;

#[tokio::test]
async fn cancel_while_reading_paths_removes_the_file_and_frees_the_slot() {
    let mut r = report_rig().await;
    let gate = Arc::new(Gate::default());
    r.source.park_next_scan(&gate);
    let (target, path) = file_target(&r, "cancelled.csv");
    send_report(&r.tx, 5, report_request(vec![1, 2], true), target);
    within(gate.entered.notified()).await;
    assert!(path.exists(), "the file exists while the report runs");
    r.tx.send(Command::Compare(CompareCommand::Cancel {
        session: 1,
        run: 5,
    }))
    .unwrap();
    barrier(&r.tx, &mut r.h).await;
    gate.release.notify_one();
    let events = within(collect_export(&mut r.h)).await;
    let outcome = outcome_of(&events);
    assert!(outcome.cancelled && outcome.partial_removed && !outcome.file_left);
    assert_eq!(
        (
            outcome.written,
            outcome.skipped,
            outcome.failed,
            outcome.bytes
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(outcome.not_run, 11, "nothing counts as exported");
    assert!(!path.exists());
    assert_silent(&mut r.h).await;
    let (_, again) = file_report(&mut r, 6, vec![1], false).await;
    assert_eq!(again.written, 8, "the slot was freed");
}

#[tokio::test]
async fn cancelling_a_clipboard_report_sends_no_text() {
    let mut r = report_rig().await;
    let gate = Arc::new(Gate::default());
    r.source.park_next_scan(&gate);
    send_report(
        &r.tx,
        5,
        report_request(vec![1], true),
        ReportTarget::Clipboard,
    );
    within(gate.entered.notified()).await;
    r.tx.send(Command::Compare(CompareCommand::Cancel {
        session: 1,
        run: 5,
    }))
    .unwrap();
    barrier(&r.tx, &mut r.h).await;
    gate.release.notify_one();
    let events = within(collect_export(&mut r.h)).await;
    assert!(matches!(
        events.as_slice(),
        [CompareEvent::ExportDone { .. }]
    ));
    let outcome = outcome_of(&events);
    assert!(outcome.cancelled && outcome.partial_removed && !outcome.file_left);
    assert_eq!((outcome.written, outcome.bytes, outcome.not_run), (0, 0, 8));
}

#[tokio::test]
async fn closing_the_session_mid_report_emits_nothing_more_and_removes_the_file() {
    let mut r = report_rig().await;
    let gate = Arc::new(Gate::default());
    r.source.park_next_scan(&gate);
    let (target, path) = file_target(&r, "closed.csv");
    send_report(&r.tx, 5, report_request(vec![1], true), target);
    within(gate.entered.notified()).await;
    r.tx.send(Command::Compare(CompareCommand::Close { session: 1 }))
        .unwrap();
    barrier(&r.tx, &mut r.h).await;
    gate.release.notify_one();
    assert_silent(&mut r.h).await;
    for _ in 0..200 {
        if !path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the file of a closed session was not removed");
}

#[tokio::test]
async fn a_panicking_store_removes_the_file_and_still_ends_with_export_done() {
    let mut r = report_rig().await;
    r.source.panic_scan.store(true, Ordering::SeqCst);
    let (target, path) = file_target(&r, "panic.csv");
    let events = report(&mut r, 5, report_request(vec![1], true), target).await;
    let outcome = outcome_of(&events);
    assert!(outcome.partial_removed && !outcome.file_left && !outcome.cancelled);
    assert_eq!((outcome.written, outcome.bytes, outcome.not_run), (0, 0, 8));
    assert!(!path.exists());
    r.source.panic_scan.store(false, Ordering::SeqCst);
    let (_, again) = file_report(&mut r, 6, vec![1], false).await;
    assert_eq!(again.written, 8, "the slot was freed");
}

#[tokio::test]
async fn a_panicking_clipboard_report_sends_no_text_and_frees_the_slot() {
    let mut r = report_rig().await;
    r.source.panic_scan.store(true, Ordering::SeqCst);
    let events = report(
        &mut r,
        5,
        report_request(vec![1], true),
        ReportTarget::Clipboard,
    )
    .await;
    assert!(matches!(
        events.as_slice(),
        [CompareEvent::ExportDone { .. }]
    ));
    assert_eq!(outcome_of(&events).not_run, 8);
    r.source.panic_scan.store(false, Ordering::SeqCst);
    let events = report(
        &mut r,
        6,
        report_request(vec![1], false),
        ReportTarget::Clipboard,
    )
    .await;
    assert!(!text_of(&events).is_empty());
}

#[tokio::test]
async fn a_write_error_is_a_failure_without_done_and_removes_the_file() {
    let mut r = report_rig().await;
    let (target, path) = file_target(&r, "full.csv");
    fail_writes_after(&path, 0);
    let events = report(&mut r, 5, report_request(vec![1], false), target).await;
    let (last, before) = events.split_last().unwrap();
    match last {
        CompareEvent::Failed {
            context: "compare_export",
            error,
            ..
        } => assert_eq!(*error, CoreError::ExportWriteFailed),
        other => panic!("{other:?}"),
    }
    assert!(
        before
            .iter()
            .all(|e| matches!(e, CompareEvent::ExportProgress { .. })),
        "no ExportDone after a failure"
    );
    assert!(!path.exists());
    let (_, again) = file_report(&mut r, 6, vec![1], false).await;
    assert_eq!(again.written, 8, "the slot was freed");
}

#[tokio::test]
async fn the_report_shares_the_job_slot_with_exports() {
    let mut r = report_rig().await;
    let gate = Arc::new(Gate::default());
    r.source.park_next_scan(&gate);
    let (target, path) = file_target(&r, "busy.csv");
    send_report(&r.tx, 5, report_request(vec![1], true), target);
    within(gate.entered.notified()).await;
    let busy = CoreError::Unsupported("a comparison is already running".into());
    send_report(
        &r.tx,
        6,
        report_request(vec![2], false),
        ReportTarget::Clipboard,
    );
    let other = r.path("other.js");
    send_export(
        &r.tx,
        7,
        request(ExportFormat::Mongosh, standard_ops()),
        &other,
    );
    for _ in 0..2 {
        match within(collect_export(&mut r.h)).await.as_slice() {
            [CompareEvent::Failed { error, .. }] => assert_eq!(*error, busy),
            other => panic!("{other:?}"),
        }
    }
    assert!(!other.exists());
    gate.release.notify_one();
    let events = within(collect_export(&mut r.h)).await;
    assert_eq!(outcome_of(&events).written, 8);
    assert!(path.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn the_file_is_owner_only_even_when_it_existed_with_wider_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let mut r = report_rig().await;
    let (target, path) = file_target(&r, "perm.csv");
    std::fs::write(
        &path,
        "old contents that are longer than the new ones? no matter",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let events = report(&mut r, 5, report_request(vec![3], false), target).await;
    let outcome = outcome_of(&events);
    let meta = std::fs::metadata(&path).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    assert_eq!(meta.len(), outcome.bytes);
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("old contents")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_target_is_refused_and_its_destination_is_left_alone() {
    let mut r = report_rig().await;
    let victim = r.path("victim.txt");
    std::fs::write(&victim, "precious").unwrap();
    let link = r.path("link.csv");
    std::os::unix::fs::symlink(&victim, &link).unwrap();
    let events = report(
        &mut r,
        5,
        report_request(vec![1], false),
        ReportTarget::File(link),
    )
    .await;
    assert!(matches!(events.as_slice(), [CompareEvent::Failed { .. }]));
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious");
}
