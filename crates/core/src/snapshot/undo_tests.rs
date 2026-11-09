use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;
use bson::{Bson, Document, doc};
use tempfile::{TempDir, tempdir};

use super::file::{SnapshotHeader, SnapshotKind, SnapshotWriter};
use super::undo::{
    ItemResult, UndoItem, UndoReport, after_path_for, capture_after, id_key, run_undo,
};
use crate::bulk::{BulkRunId, UpdateSpec};
use crate::error::{CoreError, Result};
use crate::store::{DocumentStore, Namespace, Page, Query};

/// An in-memory collection with the same compare-and-swap rules as the Mongo store.
struct MemStore {
    docs: Mutex<BTreeMap<String, Document>>,
}

impl MemStore {
    fn with(docs: &[Document]) -> Self {
        let map = docs
            .iter()
            .map(|d| (id_key(d.get("_id").unwrap()), d.clone()))
            .collect();
        Self {
            docs: Mutex::new(map),
        }
    }

    fn get(&self, id: i32) -> Option<Document> {
        self.docs
            .lock()
            .unwrap()
            .get(&id_key(&Bson::Int32(id)))
            .cloned()
    }

    fn set(&self, d: Document) {
        self.docs
            .lock()
            .unwrap()
            .insert(id_key(d.get("_id").unwrap()), d);
    }
}

#[async_trait]
impl DocumentStore for MemStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
    async fn find(&self, _ns: &Namespace, _q: &Query) -> Result<Page> {
        unimplemented!()
    }
    async fn insert(&self, _ns: &Namespace, _doc: Document) -> Result<Bson> {
        unimplemented!()
    }
    async fn replace(&self, _ns: &Namespace, _id: &Bson, _doc: Document) -> Result<()> {
        unimplemented!()
    }
    async fn delete(&self, _ns: &Namespace, _id: &Bson) -> Result<()> {
        unimplemented!()
    }
    fn is_read_only(&self) -> bool {
        false
    }
    async fn ping(&self) -> Result<()> {
        Ok(())
    }
    async fn find_by_ids(&self, _ns: &Namespace, ids: &[Bson]) -> Result<Vec<Document>> {
        let docs = self.docs.lock().unwrap();
        Ok(ids
            .iter()
            .filter_map(|i| docs.get(&id_key(i)).cloned())
            .collect())
    }
    async fn apply_undo(&self, _ns: &Namespace, items: &[UndoItem]) -> Result<Vec<ItemResult>> {
        let mut docs = self.docs.lock().unwrap();
        Ok(items.iter().map(|item| apply(&mut docs, item)).collect())
    }
}

fn apply(docs: &mut BTreeMap<String, Document>, item: &UndoItem) -> ItemResult {
    match item {
        UndoItem::Reinsert { original } => {
            let key = id_key(original.get("_id").unwrap());
            if docs.contains_key(&key) {
                return ItemResult::SkippedPresent;
            }
            docs.insert(key, original.clone());
            ItemResult::Done
        }
        UndoItem::Restore { original, expected } => {
            let key = id_key(original.get("_id").unwrap());
            match docs.get(&key) {
                Some(cur) if cur == expected => {
                    docs.insert(key, original.clone());
                    ItemResult::Done
                }
                Some(cur) if cur == original => ItemResult::Done,
                _ => ItemResult::SkippedChanged,
            }
        }
        UndoItem::Remove { expected } => {
            let key = id_key(expected.get("_id").unwrap());
            if docs.get(&key) == Some(expected) {
                docs.remove(&key);
                ItemResult::Done
            } else {
                ItemResult::SkippedChanged
            }
        }
    }
}

fn ns() -> Namespace {
    Namespace::new("shop", "orders")
}

fn header(kind: SnapshotKind) -> SnapshotHeader {
    SnapshotHeader {
        run: BulkRunId(uuid::Uuid::new_v4()),
        connection: uuid::Uuid::nil(),
        ns: ns(),
        kind,
        created_unix: 0,
    }
}

fn snapshot(dir: &TempDir, kind: SnapshotKind, docs: &[Document]) -> PathBuf {
    let mut w = SnapshotWriter::create(dir.path(), header(kind), u64::MAX).unwrap();
    for d in docs {
        w.push(d).unwrap();
    }
    w.finish().unwrap()
}

fn update_kind() -> SnapshotKind {
    SnapshotKind::Update {
        filter: doc! {},
        update: UpdateSpec::Operators(doc! { "$set": { "v": 2 } }),
        upsert: true,
    }
}

fn delete_kind() -> SnapshotKind {
    SnapshotKind::Delete { filter: doc! {} }
}

#[tokio::test]
async fn undoing_a_delete_reinserts_missing_documents_and_skips_present_ones() {
    let dir = tempdir().unwrap();
    let originals = [doc! { "_id": 1, "v": 1 }, doc! { "_id": 2, "v": 1 }];
    let path = snapshot(&dir, delete_kind(), &originals);
    // Document 2 was re-created by someone since the delete.
    let store = MemStore::with(&[doc! { "_id": 2, "v": 9 }]);

    let report = run_undo(&store, &path, None).await.unwrap();

    assert_eq!(
        report,
        UndoReport {
            restored: 1,
            skipped_present: 1,
            ..UndoReport::default()
        }
    );
    assert_eq!(store.get(1), Some(doc! { "_id": 1, "v": 1 }));
    assert_eq!(
        store.get(2),
        Some(doc! { "_id": 2, "v": 9 }),
        "present doc untouched"
    );
}

#[tokio::test]
async fn undoing_an_update_restores_only_what_the_update_produced() {
    let dir = tempdir().unwrap();
    let originals = [
        doc! { "_id": 1, "v": 1 },
        doc! { "_id": 2, "v": 1 },
        doc! { "_id": 3, "v": 1 },
        doc! { "_id": 4, "v": 1 },
    ];
    let path = snapshot(&dir, update_kind(), &originals);
    // After the update: 1, 2, 3 changed; 4 was not touched by it.
    let store = MemStore::with(&[
        doc! { "_id": 1, "v": 2 },
        doc! { "_id": 2, "v": 2 },
        doc! { "_id": 3, "v": 2 },
        doc! { "_id": 4, "v": 1 },
    ]);
    capture_after(&store, &path, &[], u64::MAX).await.unwrap();
    // Later: someone edits 2 and deletes 3.
    store.set(doc! { "_id": 2, "v": 777 });
    store.docs.lock().unwrap().remove(&id_key(&Bson::Int32(3)));

    let report = run_undo(&store, &path, Some(&after_path_for(&path)))
        .await
        .unwrap();

    assert_eq!(store.get(1), Some(doc! { "_id": 1, "v": 1 }));
    assert_eq!(
        store.get(2),
        Some(doc! { "_id": 2, "v": 777 }),
        "a concurrent edit must not be overwritten"
    );
    assert_eq!(store.get(3), None);
    assert_eq!(
        report,
        UndoReport {
            restored: 1,
            skipped_changed: 2,
            unchanged: 1,
            ..UndoReport::default()
        }
    );
}

#[tokio::test]
async fn upserted_documents_are_removed_only_when_unchanged() {
    let dir = tempdir().unwrap();
    let path = snapshot(&dir, update_kind(), &[]);
    let store = MemStore::with(&[doc! { "_id": 10, "n": 1 }, doc! { "_id": 11, "n": 1 }]);
    capture_after(&store, &path, &[Bson::Int32(10), Bson::Int32(11)], u64::MAX)
        .await
        .unwrap();
    store.set(doc! { "_id": 11, "n": 2 });

    let report = run_undo(&store, &path, Some(&after_path_for(&path)))
        .await
        .unwrap();

    assert_eq!(store.get(10), None);
    assert_eq!(store.get(11), Some(doc! { "_id": 11, "n": 2 }));
    assert_eq!(
        report,
        UndoReport {
            restored: 1,
            skipped_changed: 1,
            ..UndoReport::default()
        }
    );
}

#[tokio::test]
async fn undoing_an_update_needs_the_after_state() {
    let dir = tempdir().unwrap();
    let path = snapshot(&dir, update_kind(), &[doc! { "_id": 1 }]);
    let store = MemStore::with(&[]);
    assert_eq!(
        run_undo(&store, &path, None).await,
        Err(CoreError::SnapshotUnreadable)
    );
}

#[tokio::test]
async fn undo_works_across_more_than_one_batch() {
    let dir = tempdir().unwrap();
    let originals: Vec<Document> = (0..1200).map(|i| doc! { "_id": i, "v": 1 }).collect();
    let path = snapshot(&dir, delete_kind(), &originals);
    let store = MemStore::with(&[]);
    let report = run_undo(&store, &path, None).await.unwrap();
    assert_eq!(report.restored, 1200);
    assert_eq!(store.docs.lock().unwrap().len(), 1200);
}
