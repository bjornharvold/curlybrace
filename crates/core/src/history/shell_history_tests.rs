//! The shell tab's run history: kept per connection, newest first, last 200 only.

use super::db::HistoryDb;
use super::shell_history::{
    MAX_STORED_SCRIPT_BYTES, SHELL_HISTORY_LIMIT, ShellHistoryEntry, ShellRunStatus,
};
use crate::shell::Outcome;
use uuid::Uuid;

const NOW: i64 = 1_800_000_000;

fn open() -> (tempfile::TempDir, HistoryDb) {
    let dir = tempfile::tempdir().unwrap();
    let (db, recovery) = HistoryDb::open(&dir.path().join("history.sqlite")).unwrap();
    assert!(recovery.is_none());
    (dir, db)
}

fn conn(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn scripts(db: &HistoryDb, connection: Uuid) -> Vec<String> {
    db.shell_history(connection)
        .unwrap()
        .into_iter()
        .map(|e| e.script)
        .collect()
}

#[test]
fn a_run_is_recorded_and_read_back_newest_first() {
    let (_dir, db) = open();
    let c = conn(1);
    db.record_shell_run(c, "db.a.find()", ShellRunStatus::Succeeded, NOW)
        .unwrap();
    db.record_shell_run(c, "db.b.find()", ShellRunStatus::Failed, NOW + 5)
        .unwrap();
    let rows = db.shell_history(c).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].script, "db.b.find()");
    assert_eq!(rows[0].status, ShellRunStatus::Failed);
    assert_eq!(rows[0].ran_at_secs, NOW + 5);
    assert_eq!(rows[1].script, "db.a.find()");
    assert_eq!(rows[1].status, ShellRunStatus::Succeeded);
    assert_eq!(rows[1].ran_at_secs, NOW);
    assert!(rows[0].id > rows[1].id);
}

#[test]
fn only_the_last_200_are_kept() {
    let (_dir, db) = open();
    let c = conn(1);
    for i in 0..(SHELL_HISTORY_LIMIT + 5) {
        db.record_shell_run(c, &format!("run {i}"), ShellRunStatus::Succeeded, NOW)
            .unwrap();
    }
    let kept = scripts(&db, c);
    assert_eq!(kept.len(), SHELL_HISTORY_LIMIT);
    assert_eq!(kept[0], format!("run {}", SHELL_HISTORY_LIMIT + 4));
    assert_eq!(kept[SHELL_HISTORY_LIMIT - 1], "run 5");
    // The trim is physical, not only a limit on the read.
    let stored: i64 = db
        .lock()
        .query_row("SELECT COUNT(*) FROM shell_history", [], |r| r.get(0))
        .unwrap();
    assert_eq!(stored, SHELL_HISTORY_LIMIT as i64);
}

#[test]
fn an_identical_directly_preceding_script_is_replaced_not_added() {
    let (_dir, db) = open();
    let c = conn(1);
    db.record_shell_run(c, "same", ShellRunStatus::Failed, NOW)
        .unwrap();
    db.record_shell_run(c, "same", ShellRunStatus::Succeeded, NOW + 10)
        .unwrap();
    let rows = db.shell_history(c).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, ShellRunStatus::Succeeded);
    assert_eq!(rows[0].ran_at_secs, NOW + 10);

    // A different script in between keeps both copies.
    db.record_shell_run(c, "other", ShellRunStatus::Succeeded, NOW + 20)
        .unwrap();
    db.record_shell_run(c, "same", ShellRunStatus::Succeeded, NOW + 30)
        .unwrap();
    assert_eq!(scripts(&db, c), ["same", "other", "same"]);
}

#[test]
fn another_connections_identical_script_does_not_replace_mine() {
    let (_dir, db) = open();
    db.record_shell_run(conn(1), "same", ShellRunStatus::Succeeded, NOW)
        .unwrap();
    db.record_shell_run(conn(2), "same", ShellRunStatus::Succeeded, NOW)
        .unwrap();
    assert_eq!(scripts(&db, conn(1)), ["same"]);
    assert_eq!(scripts(&db, conn(2)), ["same"]);
}

#[test]
fn a_script_over_the_size_cap_is_not_stored() {
    let (_dir, db) = open();
    let c = conn(1);
    let at_cap = "x".repeat(MAX_STORED_SCRIPT_BYTES);
    let over = "x".repeat(MAX_STORED_SCRIPT_BYTES + 1);
    db.record_shell_run(c, &over, ShellRunStatus::Succeeded, NOW)
        .unwrap();
    assert!(db.shell_history(c).unwrap().is_empty());
    db.record_shell_run(c, &at_cap, ShellRunStatus::Succeeded, NOW)
        .unwrap();
    assert_eq!(db.shell_history(c).unwrap().len(), 1);
}

#[test]
fn a_script_with_a_nul_and_multibyte_text_round_trips_exactly() {
    let (_dir, db) = open();
    let c = conn(1);
    let script = "print('a\0b')\n// héllo wörld 日本語 \u{1F600}\r\n\ttrailing  ";
    db.record_shell_run(c, script, ShellRunStatus::Succeeded, NOW)
        .unwrap();
    assert_eq!(scripts(&db, c), [script]);
}

#[test]
fn history_is_per_connection() {
    let (_dir, db) = open();
    db.record_shell_run(conn(1), "one", ShellRunStatus::Succeeded, NOW)
        .unwrap();
    db.record_shell_run(conn(2), "two", ShellRunStatus::Cancelled, NOW)
        .unwrap();
    assert_eq!(scripts(&db, conn(1)), ["one"]);
    assert_eq!(scripts(&db, conn(2)), ["two"]);
    assert!(db.shell_history(conn(3)).unwrap().is_empty());
}

#[test]
fn clear_removes_only_that_connections_rows() {
    let (_dir, db) = open();
    db.record_shell_run(conn(1), "one", ShellRunStatus::Succeeded, NOW)
        .unwrap();
    db.record_shell_run(conn(2), "two", ShellRunStatus::Succeeded, NOW)
        .unwrap();
    db.clear_shell_history(conn(1)).unwrap();
    assert!(db.shell_history(conn(1)).unwrap().is_empty());
    assert_eq!(scripts(&db, conn(2)), ["two"]);
    // Clearing is not a purge: the connection can record again.
    db.record_shell_run(conn(1), "again", ShellRunStatus::Succeeded, NOW)
        .unwrap();
    assert_eq!(scripts(&db, conn(1)), ["again"]);
}

#[test]
fn copy_makes_an_independent_deep_copy_capped_at_200() {
    let (_dir, db) = open();
    let (from, to) = (conn(1), conn(2));
    for i in 0..(SHELL_HISTORY_LIMIT + 3) {
        db.record_shell_run(
            from,
            &format!("run {i}"),
            ShellRunStatus::Incomplete,
            NOW + i as i64,
        )
        .unwrap();
    }
    db.copy_shell_history(from, to).unwrap();
    let copied = db.shell_history(to).unwrap();
    let original = db.shell_history(from).unwrap();
    assert_eq!(copied.len(), SHELL_HISTORY_LIMIT);
    let strip = |rows: &[ShellHistoryEntry]| -> Vec<(String, i64, ShellRunStatus)> {
        rows.iter()
            .map(|e| (e.script.clone(), e.ran_at_secs, e.status))
            .collect()
    };
    assert_eq!(strip(&copied), strip(&original));

    // Independent afterwards: neither side follows the other.
    db.clear_shell_history(from).unwrap();
    assert_eq!(db.shell_history(to).unwrap().len(), SHELL_HISTORY_LIMIT);
    db.record_shell_run(to, "only to", ShellRunStatus::Succeeded, NOW)
        .unwrap();
    assert!(db.shell_history(from).unwrap().is_empty());
}

#[test]
fn copying_from_a_connection_without_history_copies_nothing() {
    let (_dir, db) = open();
    db.copy_shell_history(conn(1), conn(2)).unwrap();
    assert!(db.shell_history(conn(2)).unwrap().is_empty());
}

#[test]
fn purge_connection_removes_shell_history_and_blocks_later_records() {
    let (_dir, db) = open();
    let (gone, kept) = (conn(1), conn(2));
    db.record_shell_run(gone, "old", ShellRunStatus::Succeeded, NOW)
        .unwrap();
    db.record_shell_run(kept, "keep", ShellRunStatus::Succeeded, NOW)
        .unwrap();
    db.purge_connection(gone).unwrap();
    assert!(db.shell_history(gone).unwrap().is_empty());
    assert_eq!(scripts(&db, kept), ["keep"]);

    // A run queued before the delete must not bring it back.
    db.record_shell_run(gone, "late", ShellRunStatus::Succeeded, NOW)
        .unwrap();
    assert!(db.shell_history(gone).unwrap().is_empty());
    // Nor a copy into it.
    db.copy_shell_history(kept, gone).unwrap();
    assert!(db.shell_history(gone).unwrap().is_empty());

    db.allow_recording(gone);
    db.record_shell_run(gone, "new", ShellRunStatus::Succeeded, NOW)
        .unwrap();
    assert_eq!(scripts(&db, gone), ["new"]);
}

#[test]
fn from_outcome_maps_every_variant() {
    use ShellRunStatus::*;
    let cases = [
        (Outcome::Exited(0), Some(Succeeded)),
        (Outcome::Exited(1), Some(Failed)),
        (Outcome::Exited(-1), Some(Failed)),
        (Outcome::ScriptErrors, Some(ShellRunStatus::ScriptErrors)),
        (Outcome::Incomplete, Some(ShellRunStatus::Incomplete)),
        (Outcome::Signalled, Some(Failed)),
        (Outcome::Cancelled, Some(ShellRunStatus::Cancelled)),
        (Outcome::CapReached, Some(ShellRunStatus::CapReached)),
        (Outcome::ConnectFailed, None),
        (Outcome::StartFailed, None),
    ];
    for (outcome, expected) in cases {
        assert_eq!(
            ShellRunStatus::from_outcome(&outcome),
            expected,
            "{outcome:?}"
        );
    }
}

#[test]
fn status_text_round_trips_and_unknown_text_is_refused() {
    use ShellRunStatus::*;
    for status in [
        Succeeded,
        ScriptErrors,
        Incomplete,
        Cancelled,
        CapReached,
        Failed,
    ] {
        assert_eq!(ShellRunStatus::parse(status.as_str()), Some(status));
    }
    assert_eq!(ShellRunStatus::parse("nope"), None);
    assert_eq!(ShellRunStatus::parse(""), None);
}

#[test]
fn a_row_with_an_unknown_status_is_skipped_on_read() {
    let (_dir, db) = open();
    let c = conn(1);
    db.record_shell_run(c, "good", ShellRunStatus::Succeeded, NOW)
        .unwrap();
    db.exec_for_tests(&format!(
        "INSERT INTO shell_history (connection, ran_at_secs, script, status)
         VALUES ('{c}', {NOW}, 'from the future', 'brand_new')"
    ));
    assert_eq!(scripts(&db, c), ["good"]);
}

#[test]
fn debug_output_never_shows_the_script() {
    let entry = ShellHistoryEntry {
        id: 7,
        ran_at_secs: NOW,
        script: "db.auth('admin', 'hunter2-secret')".to_owned(),
        status: ShellRunStatus::Succeeded,
    };
    let shown = format!("{entry:?}");
    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(!shown.contains("admin"), "{shown}");
    assert!(shown.contains("ShellHistoryEntry"));
    assert!(shown.contains('7'));
    assert_eq!(entry.clone(), entry);
}
