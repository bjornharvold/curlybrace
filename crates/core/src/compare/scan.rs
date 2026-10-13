//! Streams a collection's documents in key order, one batch at a time, and checks the stream.
//!
//! The scanner never trusts the server: every document must have a usable key, and consecutive
//! keys must be strictly increasing under the client comparator. Equal keys are a duplicate
//! (`CompareDuplicateKey`); a decrease is `CompareKeyOrder`. Because `scan_sorted` includes the
//! start key, each fetch after the first begins with the row already handed out; it is
//! discarded, and a SECOND row with the same key is a duplicate straddling the batch boundary.

use std::cmp::Ordering;
use std::collections::VecDeque;
use std::sync::Arc;

use bson::Document;

use super::key::{KeySpec, KeyTuple, compare_tuples, extract_key};
use crate::error::{CoreError, Result};
use crate::store::{DocumentStore, Namespace};

pub struct KeyScanner {
    store: Arc<dyn DocumentStore>,
    ns: Namespace,
    filter: Document,
    key: KeySpec,
    batch: u32,
    buffer: VecDeque<(KeyTuple, Document)>,
    /// The key of the last row handed out (or buffered): the next fetch starts here.
    last: Option<KeyTuple>,
    exhausted: bool,
    scanned: u64,
}

impl KeyScanner {
    pub fn new(
        store: Arc<dyn DocumentStore>,
        ns: Namespace,
        filter: Document,
        key: KeySpec,
        batch: u32,
    ) -> Self {
        Self {
            store,
            ns,
            filter,
            key,
            batch: batch.max(1),
            buffer: VecDeque::new(),
            last: None,
            exhausted: false,
            scanned: 0,
        }
    }

    /// Documents handed out so far.
    pub fn scanned(&self) -> u64 {
        self.scanned
    }

    pub async fn peek(&mut self) -> Result<Option<&(KeyTuple, Document)>> {
        self.fill().await?;
        Ok(self.buffer.front())
    }

    pub async fn next(&mut self) -> Result<Option<(KeyTuple, Document)>> {
        self.fill().await?;
        let row = self.buffer.pop_front();
        if row.is_some() {
            self.scanned += 1;
        }
        Ok(row)
    }

    async fn fill(&mut self) -> Result<()> {
        while self.buffer.is_empty() && !self.exhausted {
            self.fetch().await?;
        }
        Ok(())
    }

    async fn fetch(&mut self) -> Result<()> {
        // After the first fetch the start key comes back as an overlap row, hence one extra.
        let limit = if self.last.is_some() {
            self.batch.saturating_add(1)
        } else {
            self.batch
        };
        let docs = self
            .store
            .scan_sorted(&self.ns, &self.filter, &self.key, self.last.as_ref(), limit)
            .await?;
        self.exhausted = docs.len() < limit as usize;
        let mut skipped_overlap = self.last.is_none();
        for doc in docs {
            let key = extract_key(&doc, &self.key)?;
            if let Some(previous) = &self.last {
                match compare_tuples(previous, &key) {
                    Ordering::Less => {}
                    Ordering::Equal if !skipped_overlap => {
                        skipped_overlap = true;
                        continue;
                    }
                    Ordering::Equal => return Err(CoreError::CompareDuplicateKey),
                    Ordering::Greater => return Err(CoreError::CompareKeyOrder),
                }
            }
            skipped_overlap = true;
            self.last = Some(key.clone());
            self.buffer.push_back((key, doc));
        }
        Ok(())
    }
}

/// The document with exactly `key`, if any (the scan start is inclusive, so the first document
/// at or after the key is kept only when its key compares equal).
pub(crate) async fn fetch_one(
    store: &Arc<dyn DocumentStore>,
    ns: &Namespace,
    filter: &bson::Document,
    spec: &KeySpec,
    key: &KeyTuple,
) -> Result<Option<bson::Document>> {
    let found = store.scan_sorted(ns, filter, spec, Some(key), 1).await?;
    Ok(found
        .into_iter()
        .next()
        .filter(|d| extract_key(d, spec).is_ok_and(|k| compare_tuples(&k, key).is_eq())))
}

/// How many documents carry exactly one key.
#[derive(Debug, PartialEq)]
pub(crate) enum Unique {
    Absent,
    One(bson::Document),
    /// The key is no longer unique (a duplicate appeared).
    Several,
}

/// Like [`fetch_one`], but looks at two rows so a duplicate key is noticed. Used to re-verify
/// before a write, so it reads from the PRIMARY (`scan_sorted_primary`): a lagging secondary
/// must not make a stale document look current.
pub(crate) async fn fetch_unique(
    store: &Arc<dyn DocumentStore>,
    ns: &Namespace,
    filter: &bson::Document,
    spec: &KeySpec,
    key: &KeyTuple,
) -> Result<Unique> {
    let found = store
        .scan_sorted_primary(ns, filter, spec, Some(key), 2)
        .await?;
    let mut same = found
        .into_iter()
        .filter(|d| extract_key(d, spec).is_ok_and(|k| compare_tuples(&k, key).is_eq()));
    Ok(match (same.next(), same.next()) {
        (None, _) => Unique::Absent,
        (Some(doc), None) => Unique::One(doc),
        (Some(_), Some(_)) => Unique::Several,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::InMemoryStore;
    use bson::{Bson, doc};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn ns() -> Namespace {
        Namespace::new("db", "c")
    }

    fn scanner(docs: Vec<Document>, key: KeySpec, batch: u32) -> KeyScanner {
        let store = InMemoryStore::new().with_docs(&ns(), docs);
        KeyScanner::new(Arc::new(store), ns(), doc! {}, key, batch)
    }

    async fn drain(s: &mut KeyScanner) -> Result<Vec<Document>> {
        let mut out = Vec::new();
        while let Some((_, d)) = s.next().await? {
            out.push(d);
        }
        Ok(out)
    }

    #[tokio::test]
    async fn yields_every_document_once_in_key_order_across_batches() {
        for batch in [1, 2, 3, 7, 100] {
            let docs: Vec<Document> = [5, 3, 9, 1, 7, 2, 8]
                .iter()
                .map(|n| doc! { "_id": *n })
                .collect();
            let mut s = scanner(docs, KeySpec::id(), batch);
            let ids: Vec<i32> = drain(&mut s)
                .await
                .unwrap()
                .iter()
                .map(|d| d.get_i32("_id").unwrap())
                .collect();
            assert_eq!(ids, [1, 2, 3, 5, 7, 8, 9], "batch {batch}");
            assert_eq!(s.scanned(), 7);
        }
    }

    #[tokio::test]
    async fn an_empty_collection_yields_nothing() {
        let mut s = scanner(vec![], KeySpec::id(), 3);
        assert!(s.peek().await.unwrap().is_none());
        assert!(s.next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn peek_does_not_consume() {
        let mut s = scanner(vec![doc! { "_id": 1 }, doc! { "_id": 2 }], KeySpec::id(), 1);
        assert_eq!(
            s.peek().await.unwrap().unwrap().0,
            KeyTuple(vec![Bson::Int32(1)])
        );
        assert_eq!(
            s.peek().await.unwrap().unwrap().0,
            KeyTuple(vec![Bson::Int32(1)])
        );
        assert_eq!(
            s.next().await.unwrap().unwrap().0,
            KeyTuple(vec![Bson::Int32(1)])
        );
        assert_eq!(
            s.next().await.unwrap().unwrap().0,
            KeyTuple(vec![Bson::Int32(2)])
        );
        assert!(s.next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_duplicate_key_is_an_error_wherever_it_sits() {
        // Within a batch, straddling a batch boundary (batch 1 and 2), as the first and as the last rows.
        let dup = |extra: Vec<Document>| {
            let mut d = vec![doc! { "_id": 1, "k": "a" }, doc! { "_id": 2, "k": "a" }];
            d.extend(extra);
            d
        };
        let spec = KeySpec::new(vec!["k".into()]).unwrap();
        for batch in [1, 2, 3, 100] {
            for docs in [
                dup(vec![]),
                dup(vec![
                    doc! { "_id": 3, "k": "b" },
                    doc! { "_id": 4, "k": "c" },
                ]),
                {
                    let mut d = vec![doc! { "_id": 7, "k": "0" }, doc! { "_id": 8, "k": "1" }];
                    d.extend(dup(vec![]));
                    d
                },
            ] {
                let mut s = scanner(docs, spec.clone(), batch);
                assert_eq!(
                    drain(&mut s).await,
                    Err(CoreError::CompareDuplicateKey),
                    "batch {batch}"
                );
            }
        }
    }

    #[tokio::test]
    async fn keys_equal_across_numeric_types_are_duplicates() {
        let docs = vec![doc! { "_id": 1_i32 }, doc! { "_id": 1_i64 }];
        let mut s = scanner(docs, KeySpec::id(), 1);
        assert_eq!(drain(&mut s).await, Err(CoreError::CompareDuplicateKey));
    }

    #[tokio::test]
    async fn a_missing_key_field_is_an_error() {
        let mut s = scanner(
            vec![doc! { "_id": 1 }],
            KeySpec::new(vec!["k".into()]).unwrap(),
            5,
        );
        assert_eq!(drain(&mut s).await, Err(CoreError::CompareKeyMissing));
    }

    /// A store that ignores the requested order, to prove the scanner does not trust the server.
    struct Misordered(AtomicUsize);

    #[async_trait::async_trait]
    impl DocumentStore for Misordered {
        async fn list_databases(&self) -> Result<Vec<String>> {
            Ok(vec![])
        }
        async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
            Ok(vec![])
        }
        async fn find(
            &self,
            _ns: &Namespace,
            _q: &crate::store::Query,
        ) -> Result<crate::store::Page> {
            Err(CoreError::Unsupported("n/a".into()))
        }
        async fn insert(&self, _ns: &Namespace, _d: Document) -> Result<Bson> {
            Err(CoreError::Unsupported("n/a".into()))
        }
        async fn replace(&self, _ns: &Namespace, _id: &Bson, _d: Document) -> Result<()> {
            Err(CoreError::Unsupported("n/a".into()))
        }
        async fn delete(&self, _ns: &Namespace, _id: &Bson) -> Result<()> {
            Err(CoreError::Unsupported("n/a".into()))
        }
        async fn scan_sorted(
            &self,
            _: &Namespace,
            _: &Document,
            _: &KeySpec,
            _: Option<&KeyTuple>,
            _: u32,
        ) -> Result<Vec<Document>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(vec![doc! { "_id": 5 }, doc! { "_id": 2 }])
        }
    }

    #[tokio::test]
    async fn an_out_of_order_stream_is_refused() {
        let mut s = KeyScanner::new(
            Arc::new(Misordered(AtomicUsize::new(0))),
            ns(),
            doc! {},
            KeySpec::id(),
            10,
        );
        assert_eq!(drain(&mut s).await, Err(CoreError::CompareKeyOrder));
    }

    #[tokio::test]
    async fn it_never_requests_more_than_one_batch_plus_the_overlap_row() {
        struct Counting {
            inner: InMemoryStore,
            max_limit: AtomicUsize,
        }
        #[async_trait::async_trait]
        impl DocumentStore for Counting {
            async fn list_databases(&self) -> Result<Vec<String>> {
                self.inner.list_databases().await
            }
            async fn list_collections(&self, db: &str) -> Result<Vec<String>> {
                self.inner.list_collections(db).await
            }
            async fn find(
                &self,
                ns: &Namespace,
                q: &crate::store::Query,
            ) -> Result<crate::store::Page> {
                self.inner.find(ns, q).await
            }
            async fn insert(&self, ns: &Namespace, d: Document) -> Result<Bson> {
                self.inner.insert(ns, d).await
            }
            async fn replace(&self, ns: &Namespace, id: &Bson, d: Document) -> Result<()> {
                self.inner.replace(ns, id, d).await
            }
            async fn delete(&self, ns: &Namespace, id: &Bson) -> Result<()> {
                self.inner.delete(ns, id).await
            }
            async fn scan_sorted(
                &self,
                ns: &Namespace,
                f: &Document,
                k: &KeySpec,
                from: Option<&KeyTuple>,
                limit: u32,
            ) -> Result<Vec<Document>> {
                self.max_limit.fetch_max(limit as usize, Ordering::SeqCst);
                self.inner.scan_sorted(ns, f, k, from, limit).await
            }
        }
        let docs: Vec<Document> = (0..500).map(|n| doc! { "_id": n }).collect();
        let store = Arc::new(Counting {
            inner: InMemoryStore::new().with_docs(&ns(), docs),
            max_limit: AtomicUsize::new(0),
        });
        let mut s = KeyScanner::new(store.clone(), ns(), doc! {}, KeySpec::id(), 50);
        assert_eq!(drain(&mut s).await.unwrap().len(), 500);
        assert_eq!(store.max_limit.load(Ordering::SeqCst), 51);
    }
}
