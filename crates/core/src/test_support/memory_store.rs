//! In-memory `DocumentStore` that mirrors MongoDB semantics closely enough
//! for UI and controller tests. Supports top-level equality filters (operators
//! yield `Unsupported`), multi-key sort (see `bson_order`), skip/limit and
//! projection. Mutations clone and replace the stored `Vec`.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use bson::{Bson, Document, oid::ObjectId};

use super::bson_order::cmp_bson;
use super::memory_fields::update_fields;
use super::memory_indexes::{self as idx, IndexMap, UsageMap};
use super::memory_options::{self as opts, OptionsMap};
use super::query_eval::{check_filter, compare, matches, parse_sort, project};
use crate::compare::key::{KeySpec, KeyTuple, compare_tuples, extract_key, key_values};
use crate::error::{CoreError, Result};
use crate::indexes::{IndexOptions, IndexSpec};
use crate::store::{
    CollectionIndex, CollectionKind, CollectionOptions, DocumentStore, DuplicateGroup, IndexStatus,
    IndexUsage, Namespace, Page, Query, WriteFailure, WriteOp, WriteOutcome,
};

type Collections = BTreeMap<(String, String), Vec<Document>>;

#[derive(Default)]
pub struct InMemoryStore {
    data: Mutex<Collections>,
    pub(super) indexes: Mutex<IndexMap>,
    pub(super) usage: Mutex<UsageMap>,
    options: Mutex<OptionsMap>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a store seeded with `docs` in `ns` (added to any existing docs).
    pub fn with_docs(self, ns: &Namespace, docs: Vec<Document>) -> Self {
        {
            let mut data = self.data.lock().expect("fresh mutex cannot be poisoned");
            let mut merged = data.get(&key(ns)).cloned().unwrap_or_default();
            merged.extend(docs);
            data.insert(key(ns), merged);
        }
        self
    }

    fn lock(&self) -> Result<MutexGuard<'_, Collections>> {
        self.data.lock().map_err(|_| poisoned())
    }

    fn lock_indexes(&self) -> Result<MutexGuard<'_, IndexMap>> {
        self.indexes.lock().map_err(|_| poisoned())
    }
}

fn poisoned() -> CoreError {
    CoreError::Database("in-memory store lock poisoned".into())
}

fn key(ns: &Namespace) -> (String, String) {
    (ns.db.clone(), ns.collection.clone())
}

fn id_of(doc: &Document) -> Option<&Bson> {
    doc.get("_id")
}

fn position(docs: &[Document], id: &Bson) -> Option<usize> {
    docs.iter()
        .position(|d| id_of(d).is_some_and(|x| cmp_bson(x, id).is_eq()))
}

/// Returns `doc` with `_id` first.
fn with_id(doc: Document, id: Bson) -> Document {
    let mut out = Document::new();
    out.insert("_id", id);
    out.extend(doc.into_iter().filter(|(k, _)| k != "_id"));
    out
}

/// Applies one write to the working copy `docs`.
fn apply_op(docs: &mut Vec<Document>, op: WriteOp) -> WriteOutcome {
    match op {
        WriteOp::Insert { doc } => {
            let id = id_of(&doc)
                .cloned()
                .unwrap_or_else(|| Bson::ObjectId(ObjectId::new()));
            if position(docs, &id).is_some() {
                return Err(WriteFailure::DuplicateKey);
            }
            docs.push(with_id(doc, id));
        }
        WriteOp::Replace { id, doc } => {
            let at = position(docs, &id).ok_or(WriteFailure::Missing)?;
            docs[at] = with_id(doc, id);
        }
        WriteOp::Delete { id } => {
            docs.remove(position(docs, &id).ok_or(WriteFailure::Missing)?);
        }
        WriteOp::UpdateFields { id, set, unset } => {
            let at = position(docs, &id).ok_or(WriteFailure::Missing)?;
            docs[at] = update_fields(&docs[at], &set, &unset)?;
        }
    }
    Ok(())
}

/// Evaluates a query over `docs`. Fetches `limit + 1` rows to compute
/// `has_more`; `limit == 0` means "no limit", as in MongoDB.
fn select(docs: &[Document], q: &Query) -> Result<Page> {
    check_filter(&q.filter)?;
    let sort_keys = q.sort.as_ref().map(parse_sort).transpose()?;
    let mut hits: Vec<&Document> = docs.iter().filter(|d| matches(d, &q.filter)).collect();
    if let Some(keys) = &sort_keys {
        hits.sort_by(|a, b| compare(a, b, keys));
    }
    let window = hits.into_iter().skip(q.skip as usize);
    let (fetched, limit): (Vec<&Document>, usize) = match q.limit {
        0 => (window.collect(), usize::MAX),
        n => (window.take(n as usize + 1).collect(), n as usize),
    };
    let has_more = fetched.len() > limit;
    let docs = fetched
        .into_iter()
        .take(limit)
        .map(|d| match &q.projection {
            Some(p) => project(d, p),
            None => Ok(d.clone()),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Page {
        docs,
        skip: q.skip,
        has_more,
    })
}

#[async_trait]
impl DocumentStore for InMemoryStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        let data = self.lock()?;
        let mut dbs: Vec<String> = data.keys().map(|(db, _)| db.clone()).collect();
        dbs.dedup(); // keys are sorted, so equal dbs are adjacent
        Ok(dbs)
    }

    async fn list_collections(&self, db: &str) -> Result<Vec<String>> {
        let data = self.lock()?;
        Ok(data
            .keys()
            .filter(|(d, _)| d == db)
            .map(|(_, c)| c.clone())
            .collect())
    }

    async fn find(&self, ns: &Namespace, q: &Query) -> Result<Page> {
        let data = self.lock()?;
        select(data.get(&key(ns)).map_or(&[][..], Vec::as_slice), q)
    }

    async fn insert(&self, ns: &Namespace, doc: Document) -> Result<Bson> {
        let mut data = self.lock()?;
        let id = id_of(&doc)
            .cloned()
            .unwrap_or_else(|| Bson::ObjectId(ObjectId::new()));
        let mut docs = data.get(&key(ns)).cloned().unwrap_or_default();
        if position(&docs, &id).is_some() {
            return Err(CoreError::Database(format!("duplicate key: _id {id}")));
        }
        docs.push(with_id(doc, id.clone()));
        data.insert(key(ns), docs);
        Ok(id)
    }

    async fn replace(&self, ns: &Namespace, id: &Bson, doc: Document) -> Result<()> {
        let mut data = self.lock()?;
        let mut docs = data.get(&key(ns)).cloned().unwrap_or_default();
        let at = position(&docs, id).ok_or(CoreError::NotFound)?;
        if id_of(&doc).is_some_and(|x| !cmp_bson(x, id).is_eq()) {
            return Err(CoreError::InvalidDocument("_id is immutable".into()));
        }
        docs[at] = with_id(doc, id.clone());
        data.insert(key(ns), docs);
        Ok(())
    }

    async fn delete(&self, ns: &Namespace, id: &Bson) -> Result<()> {
        let mut data = self.lock()?;
        let mut docs = data.get(&key(ns)).cloned().unwrap_or_default();
        docs.remove(position(&docs, id).ok_or(CoreError::NotFound)?);
        data.insert(key(ns), docs);
        Ok(())
    }

    async fn bulk_write(&self, ns: &Namespace, ops: Vec<WriteOp>) -> Result<Vec<WriteOutcome>> {
        let mut data = self.lock()?;
        let mut docs = data.get(&key(ns)).cloned().unwrap_or_default();
        let outcomes = ops.into_iter().map(|op| apply_op(&mut docs, op)).collect();
        data.insert(key(ns), docs);
        Ok(outcomes)
    }

    async fn estimated_count(&self, ns: &Namespace) -> Result<u64> {
        let guard = self.lock()?;
        Ok(guard.get(&key(ns)).map_or(0, |docs| docs.len() as u64))
    }

    async fn scan_sorted(
        &self,
        ns: &Namespace,
        filter: &Document,
        key_spec: &KeySpec,
        from: Option<&KeyTuple>,
        limit: u32,
    ) -> Result<Vec<Document>> {
        check_filter(filter)?;
        let guard = self.lock()?;
        let docs = guard.get(&key(ns)).map(Vec::as_slice).unwrap_or_default();
        let mut keyed: Vec<(KeyTuple, &Document)> = Vec::new();
        for doc in docs {
            if !matches(doc, filter) {
                continue;
            }
            keyed.push((extract_key(doc, key_spec)?, doc));
        }
        keyed.sort_by(|a, b| compare_tuples(&a.0, &b.0));
        Ok(keyed
            .into_iter()
            .filter(|(k, _)| from.is_none_or(|f| compare_tuples(k, f) != std::cmp::Ordering::Less))
            .take(limit as usize)
            .map(|(_, d)| d.clone())
            .collect())
    }

    async fn key_index_status(&self, _ns: &Namespace, _key: &KeySpec) -> Result<IndexStatus> {
        Ok(IndexStatus::Indexed)
    }

    async fn list_collection_kinds(&self, db: &str) -> Result<Vec<(String, CollectionKind)>> {
        let guard = self.lock()?;
        let mut names: Vec<String> = guard
            .keys()
            .filter(|(d, _)| d == db)
            .map(|(_, c)| c.clone())
            .collect();
        names.sort();
        Ok(names
            .into_iter()
            .map(|n| (n, CollectionKind::Collection))
            .collect())
    }

    async fn duplicate_keys(
        &self,
        ns: &Namespace,
        filter: &Document,
        key_spec: &KeySpec,
        limit: u32,
    ) -> Result<Vec<DuplicateGroup>> {
        check_filter(filter)?;
        if key_spec.has_dollar_component() {
            return Err(CoreError::CompareSetupInvalid);
        }
        let guard = self.lock()?;
        let docs = guard.get(&key(ns)).map(Vec::as_slice).unwrap_or_default();
        let mut keyed: Vec<KeyTuple> = docs
            .iter()
            .filter(|d| matches(d, filter))
            .filter_map(|d| key_values(d, key_spec))
            .collect();
        keyed.sort_by(compare_tuples);
        let mut groups: Vec<DuplicateGroup> = Vec::new();
        for tuple in keyed {
            match groups.last_mut() {
                Some(last) if compare_tuples(&last.key, &tuple).is_eq() => last.count += 1,
                _ => groups.push(DuplicateGroup {
                    key: tuple,
                    count: 1,
                }),
            }
        }
        groups.retain(|g| g.count > 1);
        // Stable sort: equal counts stay in key order.
        groups.sort_by(|a, b| b.count.cmp(&a.count));
        groups.truncate(limit as usize);
        Ok(groups)
    }

    async fn list_indexes(&self, ns: &Namespace) -> Result<Vec<CollectionIndex>> {
        Ok(idx::list(&*self.lock_indexes()?, &key(ns)))
    }

    async fn index_stats(&self, ns: &Namespace) -> Result<Vec<IndexUsage>> {
        let usage = self.usage.lock().map_err(|_| poisoned())?;
        Ok(usage.get(&key(ns)).cloned().unwrap_or_default())
    }

    async fn create_index(&self, ns: &Namespace, spec: &IndexSpec) -> Result<()> {
        idx::create(&mut *self.lock_indexes()?, key(ns), spec)
    }

    async fn drop_index(&self, ns: &Namespace, name: &str) -> Result<()> {
        idx::drop_named(&mut *self.lock_indexes()?, &key(ns), name)
    }

    async fn set_index_hidden(&self, ns: &Namespace, name: &str, hidden: bool) -> Result<()> {
        let change = |o| IndexOptions { hidden, ..o };
        idx::update_options(&mut *self.lock_indexes()?, &key(ns), name, change)
    }

    async fn set_index_ttl(&self, ns: &Namespace, name: &str, seconds: i64) -> Result<()> {
        let change = |o| IndexOptions {
            expire_after_seconds: Some(seconds),
            ..o
        };
        idx::update_options(&mut *self.lock_indexes()?, &key(ns), name, change)
    }

    async fn create_collection(&self, ns: &Namespace, options: &CollectionOptions) -> Result<()> {
        let mut data = self.lock()?;
        let mut stored = self.options.lock().map_err(|_| poisoned())?;
        opts::create(&mut stored, key(ns), data.contains_key(&key(ns)), options)?;
        data.insert(key(ns), Vec::new());
        Ok(())
    }

    async fn collection_options(&self, ns: &Namespace) -> Result<Option<CollectionOptions>> {
        let data = self.lock()?;
        let stored = self.options.lock().map_err(|_| poisoned())?;
        Ok(opts::get(&stored, &key(ns), data.contains_key(&key(ns))))
    }
}

#[cfg(test)]
mod tests {
    use super::super::mock_objects::*;
    use super::*;
    use crate::compare::key::{KeySpec, KeyTuple};
    use crate::error::CoreError;
    use crate::store::{CollectionKind, DocumentStore, DuplicateGroup, IndexStatus, Query};
    use bson::{Bson, doc};

    fn numbered(count: i32) -> Vec<bson::Document> {
        (0..count)
            .map(|i| doc! { "_id": i, "n": i, "a": i % 2, "b": "x" })
            .collect()
    }

    fn store(count: i32) -> InMemoryStore {
        InMemoryStore::new().with_docs(&mock_namespace(), numbered(count))
    }

    #[tokio::test]
    async fn find_limits_and_reports_has_more() {
        let q = Query {
            limit: 3,
            ..Query::default()
        };
        let page = store(5).find(&mock_namespace(), &q).await.unwrap();
        assert_eq!(page.docs.len(), 3);
        assert!(page.has_more);
        let q = Query {
            limit: 5,
            ..Query::default()
        };
        let page = store(5).find(&mock_namespace(), &q).await.unwrap();
        assert_eq!((page.docs.len(), page.has_more), (5, false));
    }

    #[tokio::test]
    async fn skip_offsets_results() {
        let q = Query {
            skip: 3,
            limit: 10,
            ..Query::default()
        };
        let page = store(5).find(&mock_namespace(), &q).await.unwrap();
        assert_eq!(page.skip, 3);
        assert_eq!(
            page.docs
                .iter()
                .map(|d| d.get_i32("n").unwrap())
                .collect::<Vec<_>>(),
            [3, 4]
        );
    }

    #[tokio::test]
    async fn sort_descending_and_multi_key() {
        let q = Query {
            sort: Some(doc! { "n": -1 }),
            ..Query::default()
        };
        let page = store(4).find(&mock_namespace(), &q).await.unwrap();
        let ns: Vec<_> = page.docs.iter().map(|d| d.get_i32("n").unwrap()).collect();
        assert_eq!(ns, [3, 2, 1, 0]);
        let q = Query {
            sort: Some(doc! { "a": 1, "n": -1 }),
            ..Query::default()
        };
        let page = store(4).find(&mock_namespace(), &q).await.unwrap();
        let ns: Vec<_> = page.docs.iter().map(|d| d.get_i32("n").unwrap()).collect();
        assert_eq!(ns, [2, 0, 3, 1]);
    }

    #[tokio::test]
    async fn projection_inclusion_keeps_id() {
        let q = Query {
            projection: Some(doc! { "a": 1 }),
            ..Query::default()
        };
        let page = store(2).find(&mock_namespace(), &q).await.unwrap();
        assert_eq!(page.docs[1], doc! { "_id": 1, "a": 1 });
        let q = Query {
            projection: Some(doc! { "a": 1, "_id": 0 }),
            ..Query::default()
        };
        let page = store(2).find(&mock_namespace(), &q).await.unwrap();
        assert_eq!(page.docs[1], doc! { "a": 1 });
    }

    #[tokio::test]
    async fn equality_filter_matches_top_level_and_rejects_operators() {
        let q = Query {
            filter: doc! { "a": 1, "b": "x" },
            ..Query::default()
        };
        let page = store(6).find(&mock_namespace(), &q).await.unwrap();
        assert_eq!(page.docs.len(), 3);
        let q = Query {
            filter: doc! { "n": { "$gt": 2 } },
            ..Query::default()
        };
        let err = store(6).find(&mock_namespace(), &q).await.unwrap_err();
        assert!(matches!(err, CoreError::Unsupported(_)));
    }

    #[tokio::test]
    async fn in_filter_matches_any_listed_value_with_bson_equality() {
        let q = Query {
            filter: doc! { "_id": { "$in": [1_i64, 3.0_f64, 99_i32] } },
            ..Query::default()
        };
        let page = store(6).find(&mock_namespace(), &q).await.unwrap();
        let ids: Vec<i32> = page
            .docs
            .iter()
            .map(|d| d.get_i32("_id").unwrap())
            .collect();
        assert_eq!(ids, vec![1, 3]);
        let none = Query {
            filter: doc! { "_id": { "$in": [] } },
            ..Query::default()
        };
        let page = store(6).find(&mock_namespace(), &none).await.unwrap();
        assert!(page.docs.is_empty());
    }

    #[tokio::test]
    async fn in_filter_is_honoured_by_scan_sorted_and_combines_with_equality() {
        let filter = doc! { "a": 1, "_id": { "$in": [1, 2, 3] } };
        let docs = store(6)
            .scan_sorted(&mock_namespace(), &filter, &KeySpec::id(), None, 10)
            .await
            .unwrap();
        let ids: Vec<i32> = docs.iter().map(|d| d.get_i32("_id").unwrap()).collect();
        assert_eq!(ids, vec![1, 3]);
    }

    #[tokio::test]
    async fn in_filter_needs_an_array_and_other_operators_stay_unsupported() {
        for filter in [
            doc! { "_id": { "$in": 1 } },
            doc! { "_id": { "$in": [1], "$gt": 0 } },
            doc! { "_id": { "$nin": [1] } },
        ] {
            let q = Query {
                filter,
                ..Query::default()
            };
            let err = store(6).find(&mock_namespace(), &q).await.unwrap_err();
            assert!(matches!(err, CoreError::Unsupported(_)));
        }
    }

    #[tokio::test]
    async fn insert_returns_given_or_generated_id() {
        let s = InMemoryStore::new();
        let ns = mock_namespace();
        assert_eq!(
            s.insert(&ns, doc! { "_id": 9 }).await.unwrap(),
            Bson::Int32(9)
        );
        let id = s.insert(&ns, doc! { "x": 1 }).await.unwrap();
        assert!(matches!(id, Bson::ObjectId(_)));
        let q = Query {
            filter: doc! { "_id": id },
            ..Query::default()
        };
        assert_eq!(s.find(&ns, &q).await.unwrap().docs.len(), 1);
    }

    #[tokio::test]
    async fn replace_and_delete_work_and_report_not_found() {
        let s = store(2);
        let ns = mock_namespace();
        s.replace(&ns, &Bson::Int32(1), doc! { "_id": 1, "n": 100 })
            .await
            .unwrap();
        let q = Query {
            filter: doc! { "_id": 1 },
            ..Query::default()
        };
        assert_eq!(s.find(&ns, &q).await.unwrap().docs[0].get_i32("n"), Ok(100));
        s.delete(&ns, &Bson::Int32(1)).await.unwrap();
        assert_eq!(
            s.delete(&ns, &Bson::Int32(1)).await,
            Err(CoreError::NotFound)
        );
        let missing = s.replace(&ns, &Bson::Int32(77), doc! { "n": 1 }).await;
        assert_eq!(missing, Err(CoreError::NotFound));
    }

    #[tokio::test]
    async fn unusual_namespaces_round_trip() {
        let s = InMemoryStore::new();
        for (db, col) in [
            ("my db", "col.with.dots"),
            ("my db", "коллекция"),
            ("数据", "a b"),
        ] {
            s.insert(&crate::store::Namespace::new(db, col), doc! { "x": 1 })
                .await
                .unwrap();
        }
        assert_eq!(s.list_databases().await.unwrap(), ["my db", "数据"]);
        assert_eq!(
            s.list_collections("my db").await.unwrap(),
            ["col.with.dots", "коллекция"]
        );
        assert_eq!(
            s.list_collections("nope").await.unwrap(),
            Vec::<String>::new()
        );
    }

    fn keyed(values: &[i32]) -> InMemoryStore {
        let ns = Namespace::new("db", "c");
        InMemoryStore::new().with_docs(
            &ns,
            values
                .iter()
                .map(
                    |v| doc! { "_id": *v, "n": *v, "tag": if v % 2 == 0 { "even" } else { "odd" } },
                )
                .collect(),
        )
    }

    #[tokio::test]
    async fn scan_sorted_returns_documents_in_key_order_from_an_inclusive_start() {
        let store = keyed(&[5, 1, 3, 2, 4]);
        let ns = Namespace::new("db", "c");
        let all = store
            .scan_sorted(&ns, &doc! {}, &KeySpec::id(), None, 100)
            .await
            .unwrap();
        let ids: Vec<i32> = all.iter().map(|d| d.get_i32("_id").unwrap()).collect();
        assert_eq!(ids, [1, 2, 3, 4, 5]);
        let from = KeyTuple(vec![bson::Bson::Int32(3)]);
        let tail = store
            .scan_sorted(&ns, &doc! {}, &KeySpec::id(), Some(&from), 2)
            .await
            .unwrap();
        let ids: Vec<i32> = tail.iter().map(|d| d.get_i32("_id").unwrap()).collect();
        assert_eq!(
            ids,
            [3, 4],
            "the start key itself is included, then the limit applies"
        );
    }

    #[tokio::test]
    async fn scan_sorted_applies_the_filter_and_compound_keys() {
        let store = keyed(&[1, 2, 3, 4]);
        let ns = Namespace::new("db", "c");
        let evens = store
            .scan_sorted(&ns, &doc! { "tag": "even" }, &KeySpec::id(), None, 100)
            .await
            .unwrap();
        assert_eq!(evens.len(), 2);
        let compound = KeySpec::new(vec!["tag".into(), "n".into()]).unwrap();
        let ordered = store
            .scan_sorted(&ns, &doc! {}, &compound, None, 100)
            .await
            .unwrap();
        let ns_order: Vec<i32> = ordered.iter().map(|d| d.get_i32("n").unwrap()).collect();
        assert_eq!(ns_order, [2, 4, 1, 3], "tag then n");
    }

    #[tokio::test]
    async fn scan_sorted_orders_mixed_types_like_the_server() {
        let ns = Namespace::new("db", "c");
        let store = InMemoryStore::new().with_docs(
            &ns,
            vec![
                doc! { "_id": "b" },
                doc! { "_id": 2_i64 },
                doc! { "_id": bson::oid::ObjectId::from_bytes([1; 12]) },
                doc! { "_id": 1.5_f64 },
                doc! { "_id": "a" },
            ],
        );
        let all = store
            .scan_sorted(&ns, &doc! {}, &KeySpec::id(), None, 100)
            .await
            .unwrap();
        let ids: Vec<bson::Bson> = all.iter().map(|d| d.get("_id").unwrap().clone()).collect();
        assert_eq!(ids[0], bson::Bson::Double(1.5));
        assert_eq!(ids[1], bson::Bson::Int64(2));
        assert_eq!(ids[2], bson::Bson::String("a".into()));
        assert_eq!(ids[3], bson::Bson::String("b".into()));
        assert!(matches!(ids[4], bson::Bson::ObjectId(_)));
    }

    #[tokio::test]
    async fn scan_sorted_refuses_missing_and_invalid_keys() {
        let ns = Namespace::new("db", "c");
        let store = InMemoryStore::new()
            .with_docs(&ns, vec![doc! { "_id": 1 }, doc! { "_id": 2, "k": [1] }]);
        let spec = KeySpec::new(vec!["k".into()]).unwrap();
        assert_eq!(
            store.scan_sorted(&ns, &doc! {}, &spec, None, 10).await,
            Err(CoreError::CompareKeyMissing)
        );
    }

    #[tokio::test]
    async fn a_missing_namespace_behaves_as_empty() {
        let store = InMemoryStore::new();
        let ns = Namespace::new("nope", "nothing");
        assert_eq!(store.estimated_count(&ns).await.unwrap(), 0);
        assert!(
            store
                .scan_sorted(&ns, &doc! {}, &KeySpec::id(), None, 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store.key_index_status(&ns, &KeySpec::id()).await.unwrap(),
            IndexStatus::Indexed
        );
    }

    #[tokio::test]
    async fn counts_and_kinds_reflect_the_stored_collections() {
        let store = keyed(&[1, 2, 3]);
        let ns = Namespace::new("db", "c");
        assert_eq!(store.estimated_count(&ns).await.unwrap(), 3);
        assert_eq!(
            store.list_collection_kinds("db").await.unwrap(),
            vec![("c".to_string(), CollectionKind::Collection)]
        );
    }

    fn dup_store() -> InMemoryStore {
        let ns = Namespace::new("db", "c");
        InMemoryStore::new().with_docs(
            &ns,
            vec![
                doc! { "_id": 1, "k": 1_i32, "t": "x" },
                doc! { "_id": 2, "k": 1_i64, "t": "x" },
                doc! { "_id": 3, "k": 1.0_f64, "t": "y" },
                doc! { "_id": 4, "k": "a" },
                doc! { "_id": 5, "k": "a", "t": "x" },
                doc! { "_id": 6, "k": "b" },
                doc! { "_id": 7, "t": "x" },
                doc! { "_id": 8, "t": "x" },
            ],
        )
    }

    #[tokio::test]
    async fn duplicate_keys_groups_numbers_equal_across_types_and_orders_worst_first() {
        let ns = Namespace::new("db", "c");
        let spec = KeySpec::new(vec!["k".into()]).unwrap();
        let groups = dup_store()
            .duplicate_keys(&ns, &doc! {}, &spec, 10)
            .await
            .unwrap();
        assert_eq!(
            groups,
            vec![
                DuplicateGroup {
                    key: KeyTuple(vec![Bson::Int32(1)]),
                    count: 3
                },
                DuplicateGroup {
                    key: KeyTuple(vec![Bson::String("a".into())]),
                    count: 2
                },
            ],
            "Int32 1, Int64 1 and Double 1.0 are one key; 'b' is unique; documents without k are not counted"
        );
    }

    #[tokio::test]
    async fn duplicate_keys_refuses_dollar_prefixed_key_paths() {
        let ns = Namespace::new("db", "c");
        let spec = KeySpec::new(vec!["owner.$id".into()]).unwrap();
        let got = dup_store().duplicate_keys(&ns, &doc! {}, &spec, 10).await;
        assert_eq!(got, Err(CoreError::CompareSetupInvalid));
    }

    #[tokio::test]
    async fn duplicate_keys_respects_the_filter_the_limit_and_compound_keys() {
        let ns = Namespace::new("db", "c");
        let spec = KeySpec::new(vec!["k".into()]).unwrap();
        let store = dup_store();
        let only_x = store
            .duplicate_keys(&ns, &doc! { "t": "x" }, &spec, 10)
            .await
            .unwrap();
        assert_eq!(
            only_x,
            vec![DuplicateGroup {
                key: KeyTuple(vec![Bson::Int32(1)]),
                count: 2
            }],
            "under t == 'x' only _id 1 and 2 share a key; the filter is applied before grouping"
        );
        let limited = store.duplicate_keys(&ns, &doc! {}, &spec, 1).await.unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].count, 3);
        let compound = KeySpec::new(vec!["k".into(), "t".into()]).unwrap();
        let groups = store
            .duplicate_keys(&ns, &doc! {}, &compound, 10)
            .await
            .unwrap();
        assert_eq!(groups.len(), 1, "only (1, 'x') repeats: _id 1 and 2");
        assert_eq!(groups[0].count, 2);
    }

    #[tokio::test]
    async fn duplicate_keys_on_a_missing_collection_is_empty_and_nan_collides_with_nan() {
        let spec = KeySpec::new(vec!["k".into()]).unwrap();
        let none = InMemoryStore::new();
        assert!(
            none.duplicate_keys(&Namespace::new("x", "y"), &doc! {}, &spec, 5)
                .await
                .unwrap()
                .is_empty()
        );
        let ns = Namespace::new("db", "c");
        let nans = InMemoryStore::new()
            .with_docs(&ns, vec![doc! { "k": f64::NAN }, doc! { "k": f64::NAN }]);
        assert_eq!(
            nans.duplicate_keys(&ns, &doc! {}, &spec, 5).await.unwrap()[0].count,
            2
        );
    }

    #[tokio::test]
    async fn duplicate_keys_cuts_ties_at_the_limit_by_smallest_key_and_zero_limit_is_empty() {
        let ns = Namespace::new("db", "c");
        let spec = KeySpec::new(vec!["k".into()]).unwrap();
        let store = InMemoryStore::new().with_docs(
            &ns,
            vec![
                doc! { "k": "b" },
                doc! { "k": "b" },
                doc! { "k": 5 },
                doc! { "k": 5 },
                doc! { "k": "a" },
                doc! { "k": "a" },
            ],
        );
        let one = store.duplicate_keys(&ns, &doc! {}, &spec, 1).await.unwrap();
        assert_eq!(
            one,
            vec![DuplicateGroup {
                key: KeyTuple(vec![Bson::Int32(5)]),
                count: 2
            }],
            "numbers sort before strings, so 5 wins the tie"
        );
        assert!(
            store
                .duplicate_keys(&ns, &doc! {}, &spec, 0)
                .await
                .unwrap()
                .is_empty()
        );
    }
}
