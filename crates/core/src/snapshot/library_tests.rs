use bson::doc;
use tempfile::tempdir;

use super::file::{SnapshotHeader, SnapshotKind, SnapshotWriter};
use super::library::{delete, list, purge_older_than, snapshot_path};
use crate::bulk::BulkRunId;
use crate::store::Namespace;

fn write(dir: &std::path::Path, created: i64, collection: &str) -> BulkRunId {
    let run = BulkRunId(uuid::Uuid::new_v4());
    let header = SnapshotHeader {
        run,
        connection: uuid::Uuid::nil(),
        ns: Namespace::new("shop", collection),
        kind: SnapshotKind::Delete { filter: doc! {} },
        created_unix: created,
    };
    let mut w = SnapshotWriter::create(dir, header, u64::MAX).unwrap();
    w.push(&doc! { "_id": 1 }).unwrap();
    w.finish().unwrap();
    // The after-state file of the run.
    std::fs::write(
        snapshot_path(dir, run).with_extension("after.jsonl"),
        b"x\n",
    )
    .unwrap();
    run
}

#[test]
fn lists_snapshots_newest_first_without_reading_their_documents() {
    let dir = tempdir().unwrap();
    let old = write(dir.path(), 100, "a");
    let new = write(dir.path(), 200, "b");
    let found = list(dir.path());
    assert_eq!(found.iter().map(|s| s.run).collect::<Vec<_>>(), [new, old]);
    assert_eq!(found[0].ns.collection, "b");
    assert!(found[0].bytes > 0);
}

#[test]
fn a_missing_directory_lists_nothing() {
    let dir = tempdir().unwrap();
    assert!(list(&dir.path().join("nope")).is_empty());
}

#[test]
fn unrelated_and_damaged_files_are_ignored() {
    let dir = tempdir().unwrap();
    write(dir.path(), 1, "a");
    std::fs::write(dir.path().join("notes.txt"), "hi").unwrap();
    std::fs::write(
        dir.path().join(format!("{}.jsonl", uuid::Uuid::new_v4())),
        "junk\n",
    )
    .unwrap();
    assert_eq!(list(dir.path()).len(), 1);
}

#[test]
fn delete_removes_the_snapshot_and_its_after_state() {
    let dir = tempdir().unwrap();
    let run = write(dir.path(), 1, "a");
    let keep = write(dir.path(), 2, "b");
    delete(dir.path(), run).unwrap();
    assert_eq!(
        list(dir.path()).iter().map(|s| s.run).collect::<Vec<_>>(),
        [keep]
    );
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        2,
        "only the kept run's two files"
    );
}

#[test]
fn deleting_an_unknown_run_is_not_an_error() {
    let dir = tempdir().unwrap();
    assert!(delete(dir.path(), BulkRunId(uuid::Uuid::new_v4())).is_ok());
}

#[test]
fn purge_removes_only_snapshots_older_than_the_limit() {
    let dir = tempdir().unwrap();
    write(dir.path(), 1_000, "old");
    let fresh = write(dir.path(), 9_500, "fresh");
    let removed = purge_older_than(dir.path(), 10_000, 1_000);
    assert_eq!(removed, 1);
    assert_eq!(
        list(dir.path()).iter().map(|s| s.run).collect::<Vec<_>>(),
        [fresh]
    );
}

fn age_file(path: &std::path::Path, secs_ago: u64) {
    let when = std::time::SystemTime::now() - std::time::Duration::from_secs(secs_ago);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

fn now() -> i64 {
    crate::clock::now_secs()
}

#[test]
fn purge_also_removes_old_orphans_and_torn_files_by_their_age() {
    let dir = tempdir().unwrap();
    let orphan_after = dir
        .path()
        .join(format!("{}.after.jsonl", uuid::Uuid::new_v4()));
    let torn = dir.path().join(format!("{}.jsonl", uuid::Uuid::new_v4()));
    std::fs::write(&orphan_after, "{\"x\":1}\n").unwrap();
    std::fs::write(&torn, "{\"format\":").unwrap();
    age_file(&orphan_after, 5_000);
    age_file(&torn, 5_000);

    let removed = purge_older_than(dir.path(), now(), 1_000);

    assert_eq!(removed, 2);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn purge_keeps_recent_orphans_and_foreign_files() {
    let dir = tempdir().unwrap();
    let recent = dir.path().join(format!("{}.jsonl", uuid::Uuid::new_v4()));
    let foreign = dir.path().join("notes.txt");
    std::fs::write(&recent, "{\"format\":").unwrap();
    std::fs::write(&foreign, "mine").unwrap();
    age_file(&foreign, 999_999);

    assert_eq!(purge_older_than(dir.path(), now(), 1_000), 0);
    assert!(recent.exists() && foreign.exists());
}
