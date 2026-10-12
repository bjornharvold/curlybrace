//! In-memory `DocumentStore` that mirrors MongoDB semantics closely enough
//! for UI and controller tests. Supports top-level equality filters (operators
//! yield `Unsupported`), multi-key sort (see `bson_order`), skip/limit and
//! projection. Mutations clone and replace the stored `Vec`.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use bson::{Bson, Document, oid::ObjectId};

use super::bson_order::cmp_bson;
use super::query_eval::{check_filter, compare, matches, parse_sort, project};
use crate::error::{CoreError, Result};
use crate::store::{DocumentStore, Namespace, Page, Query};

type Collections = BTreeMap<(String, String), Vec<Document>>;

#[derive(Default)]
pub struct InMemoryStore {
    data: Mutex<Collections>,
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
        self.data
            .lock()
            .map_err(|_| CoreError::Database("in-memory store lock poisoned".into()))
    }
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
}

#[cfg(test)]
mod tests {
    use super::super::mock_objects::*;
    use super::*;
    use crate::error::CoreError;
    use crate::store::{DocumentStore, Query};
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
}
