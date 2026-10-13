//! The file lifecycle of an export: cancel, close, panic, a write error, the shared job slot,
//! permissions and overwriting.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::apply_tests::{Gate, assert_silent, barrier, op, send_apply, within};
use super::export_tests::{
    c2_pair, collect_export, export, outcome_of, request, rig, rig_with, send_export, standard_ops,
    text,
};
use super::tests::{collect_run, same_pair, send_start};
use crate::compare::apply::ApplyKind;
use crate::compare::protocol::{CompareCommand, CompareEvent, ExportFormat, ExportOutcome};
use crate::error::CoreError;
use crate::runtime::Command;

use super::apply_tests::{ApplyStore, source_store, target_store};

#[tokio::test]
async fn cancel_mid_export_removes_the_file_reports_it_and_frees_the_slot() {
    let mut r = rig().await;
    let gate = Arc::new(Gate::default());
    r.source.park_next_scan(&gate);
    let path = r.path("cancelled.js");
    send_export(
        &r.tx,
        5,
        request(ExportFormat::Mongosh, standard_ops()),
        &path,
    );
    within(gate.entered.notified()).await;
    assert!(path.exists(), "the file exists while the export runs");
    r.tx.send(Command::Compare(CompareCommand::Cancel {
        session: 1,
        run: 5,
    }))
    .unwrap();
    barrier(&r.tx, &mut r.h).await;
    gate.release.notify_one();
    let events = within(collect_export(&mut r.h)).await;
    let outcome = outcome_of(&events);
    assert!(outcome.cancelled && outcome.partial_removed);
    assert_eq!(outcome.bytes, 0);
    assert!(
        outcome.not_run > 0,
        "the pairs after the cancel did not run"
    );
    assert!(!path.exists());
    assert_silent(&mut r.h).await;
    let ok = r.path("ok.js");
    let events = export(
        &mut r,
        6,
        request(ExportFormat::Mongosh, standard_ops()),
        &ok,
    )
    .await;
    assert_eq!(outcome_of(&events).written, 3);
}

#[tokio::test]
async fn a_panicking_store_removes_the_file_and_still_ends_with_export_done() {
    let mut r = rig().await;
    r.source.panic_scan.store(true, Ordering::SeqCst);
    let path = r.path("panic.js");
    let events = export(
        &mut r,
        5,
        request(ExportFormat::Mongosh, standard_ops()),
        &path,
    )
    .await;
    let outcome = outcome_of(&events);
    assert!(outcome.partial_removed && !outcome.cancelled);
    assert_eq!((outcome.written, outcome.bytes), (0, 0));
    assert_eq!(outcome.not_run, 3);
    assert!(!path.exists());
    r.source.panic_scan.store(false, Ordering::SeqCst);
    let ok = r.path("ok.js");
    let events = export(
        &mut r,
        6,
        request(ExportFormat::Mongosh, standard_ops()),
        &ok,
    )
    .await;
    assert_eq!(outcome_of(&events).written, 3, "the slot was freed");
}

#[tokio::test]
async fn a_panicking_destination_check_is_a_refusal_and_creates_no_file() {
    let mut r = rig().await;
    r.target.panic_kinds.store(true, Ordering::SeqCst);
    let path = r.path("kinds.js");
    let events = export(
        &mut r,
        5,
        request(ExportFormat::Mongosh, standard_ops()),
        &path,
    )
    .await;
    assert!(matches!(
        events.as_slice(),
        [CompareEvent::Failed {
            context: "compare_export",
            ..
        }]
    ));
    assert!(!path.exists());
}

#[tokio::test]
async fn closing_the_session_mid_export_emits_nothing_more_and_removes_the_file() {
    let mut r = rig().await;
    let gate = Arc::new(Gate::default());
    r.source.park_next_scan(&gate);
    let path = r.path("closed.js");
    send_export(
        &r.tx,
        5,
        request(ExportFormat::Mongosh, standard_ops()),
        &path,
    );
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
async fn while_an_export_runs_every_other_job_is_refused_and_one_afterwards_is_accepted() {
    let mut r = rig().await;
    let gate = Arc::new(Gate::default());
    r.source.park_next_scan(&gate);
    let path = r.path("busy.js");
    send_export(
        &r.tx,
        5,
        request(ExportFormat::Mongosh, standard_ops()),
        &path,
    );
    within(gate.entered.notified()).await;
    let second = r.path("second.js");
    send_export(
        &r.tx,
        6,
        request(ExportFormat::Mongosh, standard_ops()),
        &second,
    );
    send_start(&r.tx, 7, vec![same_pair(1)]);
    send_apply(&r.tx, 8, vec![op(1, 1, ApplyKind::Insert)]);
    let mut refused = Vec::new();
    for _ in 0..3 {
        match within(super::tests::next_compare(&mut r.h)).await {
            CompareEvent::Failed { context, error, .. } => {
                assert_eq!(
                    error,
                    CoreError::Unsupported("a comparison is already running".into())
                );
                refused.push(context);
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(
        refused,
        ["compare_export", "compare_start", "compare_apply"]
    );
    assert!(!second.exists());
    gate.release.notify_one();
    let events = within(collect_export(&mut r.h)).await;
    assert_eq!(outcome_of(&events).written, 3);
    send_start(&r.tx, 9, vec![c2_pair()]);
    assert_eq!(within(collect_run(&mut r.h)).await.len(), 1);
    assert_eq!((r.source.writes(), r.target.writes()), (0, 0));
}

#[cfg(unix)]
#[tokio::test]
async fn the_file_is_owner_only_and_an_existing_file_is_truncated_and_restricted() {
    use std::os::unix::fs::PermissionsExt;
    let mut r = rig().await;
    let fresh = r.path("fresh.js");
    export(
        &mut r,
        5,
        request(ExportFormat::Mongosh, standard_ops()),
        &fresh,
    )
    .await;
    let mode = std::fs::metadata(&fresh).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let old = r.path("old.js");
    std::fs::write(&old, "OLD CONTENT ".repeat(10_000)).unwrap();
    std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o644)).unwrap();
    export(
        &mut r,
        6,
        request(ExportFormat::Mongosh, standard_ops()),
        &old,
    )
    .await;
    assert!(!text(&old).contains("OLD CONTENT"));
    assert_eq!(text(&old), text(&fresh));
    assert_eq!(
        std::fs::metadata(&old).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlink_at_the_path_is_refused_and_its_target_untouched() {
    let mut r = rig().await;
    let real = r.path("real.txt");
    std::fs::write(&real, "KEEP").unwrap();
    let link = r.path("link.js");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    send_export(
        &r.tx,
        5,
        request(ExportFormat::Mongosh, standard_ops()),
        &link,
    );
    match super::tests::next_compare(&mut r.h).await {
        CompareEvent::Failed { error, .. } => {
            assert_eq!(error, CoreError::Unsupported("invalid export path".into()));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(text(&real), "KEEP");
}

#[cfg(unix)]
#[tokio::test]
async fn a_write_error_ends_with_a_typed_failure_no_file_and_a_free_slot() {
    use std::os::unix::fs::PermissionsExt;
    let mut r = rig().await;
    let locked = r.path("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
    let probe = locked.join("probe");
    if std::fs::File::create(&probe).is_ok() {
        // Running with rights that ignore directory permissions: nothing to test.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        return;
    }
    let path = locked.join("out.js");
    let events = export(
        &mut r,
        5,
        request(ExportFormat::Mongosh, standard_ops()),
        &path,
    )
    .await;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(
        events.as_slice(),
        [CompareEvent::Failed {
            context: "compare_export",
            error: CoreError::ExportWriteFailed,
            ..
        }]
    ));
    assert!(!path.exists());
    let ok = r.path("ok.js");
    let events = export(
        &mut r,
        6,
        request(ExportFormat::Mongosh, standard_ops()),
        &ok,
    )
    .await;
    assert_eq!(outcome_of(&events).written, 3);
}

#[tokio::test]
async fn an_export_before_the_session_is_open_or_after_close_is_refused() {
    let mut r = rig_with(
        ApplyStore::new(source_store()),
        ApplyStore::new(target_store()),
        vec![same_pair(1)],
    )
    .await;
    r.tx.send(Command::Compare(CompareCommand::Close { session: 1 }))
        .unwrap();
    let path = r.path("late.js");
    send_export(
        &r.tx,
        5,
        request(ExportFormat::Mongosh, standard_ops()),
        &path,
    );
    match super::tests::next_compare(&mut r.h).await {
        CompareEvent::Failed {
            context: "compare_export",
            ..
        } => {}
        other => panic!("{other:?}"),
    }
    assert!(!path.exists());
    let _ = ExportOutcome::default();
}
