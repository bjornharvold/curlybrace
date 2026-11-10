use bson::{Bson, Document, doc, oid::ObjectId};
use tempfile::tempdir;

use super::file::{SnapshotHeader, SnapshotKind, SnapshotReader, SnapshotWriter};
use crate::bulk::BulkRunId;
use crate::error::CoreError;
use crate::store::Namespace;

fn header() -> SnapshotHeader {
    SnapshotHeader {
        run: BulkRunId(uuid::Uuid::new_v4()),
        connection: uuid::Uuid::nil(),
        ns: Namespace {
            db: "shop".into(),
            collection: "orders".into(),
        },
        kind: SnapshotKind::Delete {
            filter: doc! { "status": "stale" },
        },
        created_unix: 1_700_000_000,
    }
}

fn docs() -> Vec<Document> {
    vec![
        doc! { "_id": ObjectId::new(), "n": 1_i64, "when": bson::DateTime::from_millis(5) },
        doc! { "_id": 2_i32, "nested": { "z": 1, "a": [1.5, Bson::Null] } },
    ]
}

#[test]
fn documents_round_trip_with_their_types_and_order() {
    let dir = tempdir().unwrap();
    let h = header();
    let mut w = SnapshotWriter::create(dir.path(), h.clone(), u64::MAX).unwrap();
    let originals = docs();
    for d in &originals {
        w.push(d).unwrap();
    }
    let path = w.finish().unwrap();

    let mut r = SnapshotReader::open(&path).unwrap();
    assert_eq!(r.header(), &h);
    let back: Vec<Document> = r.by_ref().collect::<Result<_, _>>().unwrap();
    assert_eq!(back, originals);
    assert_eq!(
        back[1]
            .get_document("nested")
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["z", "a"]
    );
}

#[test]
fn update_header_keeps_filter_update_and_upsert() {
    let dir = tempdir().unwrap();
    let mut h = header();
    h.kind = SnapshotKind::Update {
        filter: doc! { "a": 1 },
        update: crate::bulk::UpdateSpec::Pipeline(vec![doc! { "$set": { "b": 2 } }]),
        upsert: true,
    };
    let path = SnapshotWriter::create(dir.path(), h.clone(), u64::MAX)
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(SnapshotReader::open(&path).unwrap().header(), &h);
}

#[cfg(unix)]
#[test]
fn file_and_directory_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir().unwrap();
    let sub = dir.path().join("snapshots");
    let path = SnapshotWriter::create(&sub, header(), u64::MAX)
        .unwrap()
        .finish()
        .unwrap();
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(&sub), 0o700);
}

#[test]
fn exceeding_the_cap_fails_and_removes_the_partial_file() {
    let dir = tempdir().unwrap();
    let mut w = SnapshotWriter::create(dir.path(), header(), 400).unwrap();
    let big = doc! { "_id": 1, "blob": "x".repeat(1000) };
    assert_eq!(w.push(&big), Err(CoreError::SnapshotTooLarge));
    drop(w);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn an_unwritable_directory_is_a_write_failure_without_the_path() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("not-a-dir");
    std::fs::write(&file, b"x").unwrap();
    let err = SnapshotWriter::create(&file.join("sub"), header(), u64::MAX)
        .err()
        .unwrap();
    assert_eq!(err, CoreError::SnapshotWriteFailed);
    assert!(!err.to_string().contains("not-a-dir"));
}

#[test]
fn a_file_that_is_not_a_snapshot_is_unreadable() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("x.jsonl");
    std::fs::write(&path, "hunter2\n").unwrap();
    let err = SnapshotReader::open(&path).err().unwrap();
    assert_eq!(err, CoreError::SnapshotUnreadable);
    assert!(!err.to_string().contains("hunter2"));
}

#[test]
fn a_torn_final_line_is_an_error_not_a_silent_stop() {
    let dir = tempdir().unwrap();
    let mut w = SnapshotWriter::create(dir.path(), header(), u64::MAX).unwrap();
    w.push(&docs()[1]).unwrap();
    let path = w.finish().unwrap();
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("{\"_id\": {\"$num");
    std::fs::write(&path, text).unwrap();
    let results: Vec<_> = SnapshotReader::open(&path).unwrap().collect();
    assert!(results[0].is_ok());
    assert_eq!(results[1], Err(CoreError::SnapshotUnreadable));
}
