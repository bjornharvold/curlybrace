//! [`DocumentStore`] backed by the real MongoDB driver.

use async_trait::async_trait;
use bson::{Bson, Document, doc};
use futures::TryStreamExt;
use mongodb::Client;

use crate::compare::key::{KeySpec, KeyTuple, extract_key};
use crate::compare::keyset::{IndexInfo, index_serves_sort, keyset_filter};
use crate::error::{CoreError, Result};
use crate::store::{CollectionKind, DocumentStore, IndexStatus, Namespace, Page, Query};
use mongodb::options::Collation;

pub struct MongoStore {
    client: Client,
}

impl MongoStore {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    fn collection(&self, ns: &Namespace) -> mongodb::Collection<Document> {
        self.client.database(&ns.db).collection(&ns.collection)
    }
}

/// Driver errors may quote server text; they are surfaced, but never carry
/// credentials because the client never holds them in message form.
fn db_err(e: mongodb::error::Error) -> CoreError {
    crate::driver_error::classify_driver_error(&e)
}

/// Driver limit for a query: `limit + 1` (to detect `has_more`), or `None`
/// when `limit == 0` (MongoDB's "no limit").
pub(crate) fn fetch_limit(q: &Query) -> Option<i64> {
    (q.limit > 0).then(|| {
        i64::try_from(q.limit)
            .unwrap_or(i64::MAX - 1)
            .saturating_add(1)
    })
}

/// The driver takes an unsigned skip, but the wire protocol caps it at i64.
pub(crate) fn skip_as_i64(q: &Query) -> Result<u64> {
    i64::try_from(q.skip)
        .map(|_| q.skip)
        .map_err(|_| CoreError::InvalidDocument("skip is too large".into()))
}

/// Trims the over-fetched row and derives `has_more`.
pub(crate) fn build_page(mut docs: Vec<Document>, q: &Query) -> Page {
    let has_more = q.limit > 0 && docs.len() as u64 > q.limit;
    if has_more {
        docs.truncate(usize::try_from(q.limit).unwrap_or(usize::MAX));
    }
    Page {
        docs,
        skip: q.skip,
        has_more,
    }
}

fn id_filter(id: &Bson) -> Document {
    doc! { "_id": id.clone() }
}

#[async_trait]
impl DocumentStore for MongoStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        self.client.list_database_names().await.map_err(db_err)
    }

    async fn list_collections(&self, db: &str) -> Result<Vec<String>> {
        self.client
            .database(db)
            .list_collection_names()
            .await
            .map_err(db_err)
    }

    async fn find(&self, ns: &Namespace, q: &Query) -> Result<Page> {
        let collection = self.collection(ns);
        let skip = skip_as_i64(q)?;
        let mut action = collection.find(q.filter.clone()).skip(skip);
        if let Some(limit) = fetch_limit(q) {
            action = action.limit(limit);
        }
        if let Some(sort) = &q.sort {
            action = action.sort(sort.clone());
        }
        if let Some(projection) = &q.projection {
            action = action.projection(projection.clone());
        }
        let cursor = action.await.map_err(db_err)?;
        let docs: Vec<Document> = cursor.try_collect().await.map_err(db_err)?;
        Ok(build_page(docs, q))
    }

    async fn insert(&self, ns: &Namespace, doc: Document) -> Result<Bson> {
        let result = self.collection(ns).insert_one(doc).await.map_err(db_err)?;
        Ok(result.inserted_id)
    }

    async fn replace(&self, ns: &Namespace, id: &Bson, doc: Document) -> Result<()> {
        let result = self
            .collection(ns)
            .replace_one(id_filter(id), doc)
            .await
            .map_err(db_err)?;
        if result.matched_count == 0 {
            return Err(CoreError::NotFound);
        }
        Ok(())
    }

    async fn delete(&self, ns: &Namespace, id: &Bson) -> Result<()> {
        let result = self
            .collection(ns)
            .delete_one(id_filter(id))
            .await
            .map_err(db_err)?;
        if result.deleted_count == 0 {
            return Err(CoreError::NotFound);
        }
        Ok(())
    }

    async fn estimated_count(&self, ns: &Namespace) -> Result<u64> {
        self.collection(ns)
            .estimated_document_count()
            .await
            .map_err(db_err)
    }

    async fn scan_sorted(
        &self,
        ns: &Namespace,
        filter: &Document,
        key: &KeySpec,
        from: Option<&KeyTuple>,
        limit: u32,
    ) -> Result<Vec<Document>> {
        let sort: Document = key
            .fields
            .iter()
            .map(|f| (f.clone(), Bson::Int32(1)))
            .collect();
        let cursor = self
            .collection(ns)
            .find(keyset_filter(filter, key, from))
            .sort(sort)
            .limit(i64::from(limit))
            .collation(Collation::builder().locale("simple").build())
            .allow_disk_use(true)
            .await
            .map_err(db_err)?;
        let docs: Vec<Document> = cursor.try_collect().await.map_err(db_err)?;
        // A document without a usable key would sort as null and be returned again at the head
        // of every page, so it fails the scan instead.
        for doc in &docs {
            extract_key(doc, key)?;
        }
        Ok(docs)
    }

    async fn key_index_status(&self, ns: &Namespace, key: &KeySpec) -> Result<IndexStatus> {
        use mongodb::results::CollectionType;
        let db = self.client.database(&ns.db);
        // A missing collection has nothing to sort.
        let mut specs = db
            .list_collections()
            .filter(doc! { "name": &ns.collection })
            .await
            .map_err(db_err)?;
        let Some(spec) = specs.try_next().await.map_err(db_err)? else {
            return Ok(IndexStatus::Indexed);
        };
        if spec.collection_type == CollectionType::View {
            return Ok(IndexStatus::NotIndexed);
        }
        let default_is_simple = spec
            .options
            .collation
            .as_ref()
            .is_none_or(|c| c.locale == "simple");
        let mut indexes = self.collection(ns).list_indexes().await.map_err(db_err)?;
        while let Some(model) = indexes.try_next().await.map_err(db_err)? {
            let options = model.options.unwrap_or_default();
            let info = IndexInfo {
                keys: model.keys,
                sparse: options.sparse.unwrap_or(false),
                has_partial_filter: options.partial_filter_expression.is_some(),
                collation_is_simple: options
                    .collation
                    .as_ref()
                    .is_none_or(|c| c.locale == "simple"),
            };
            if index_serves_sort(&info, default_is_simple, key) {
                return Ok(IndexStatus::Indexed);
            }
        }
        Ok(IndexStatus::NotIndexed)
    }

    async fn list_collection_kinds(&self, db: &str) -> Result<Vec<(String, CollectionKind)>> {
        use mongodb::results::CollectionType;
        let mut specs = self
            .client
            .database(db)
            .list_collections()
            .await
            .map_err(db_err)?;
        let mut out = Vec::new();
        while let Some(spec) = specs.try_next().await.map_err(db_err)? {
            let kind = match spec.collection_type {
                CollectionType::View => CollectionKind::View,
                CollectionType::Timeseries => CollectionKind::Timeseries,
                _ => CollectionKind::Collection,
            };
            out.push((spec.name, kind));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mock_objects::mock_doc_with_id;

    fn docs(n: i32) -> Vec<Document> {
        (0..n).map(mock_doc_with_id).collect()
    }

    #[test]
    fn fetch_limit_is_limit_plus_one_and_none_for_zero() {
        let q = Query {
            limit: 50,
            ..Query::default()
        };
        assert_eq!(fetch_limit(&q), Some(51));
        assert_eq!(
            fetch_limit(&Query {
                limit: 0,
                ..Query::default()
            }),
            None
        );
        assert_eq!(
            fetch_limit(&Query {
                limit: u64::MAX,
                ..Query::default()
            }),
            Some(i64::MAX)
        );
    }

    #[test]
    fn skip_beyond_i64_is_a_typed_error() {
        let ok = Query {
            skip: i64::MAX as u64,
            ..Query::default()
        };
        assert_eq!(skip_as_i64(&ok), Ok(i64::MAX as u64));
        let bad = Query {
            skip: i64::MAX as u64 + 1,
            ..Query::default()
        };
        assert_eq!(
            skip_as_i64(&bad),
            Err(CoreError::InvalidDocument("skip is too large".into()))
        );
    }

    #[test]
    fn build_page_trims_overfetch_and_sets_has_more() {
        let q = Query {
            limit: 3,
            skip: 6,
            ..Query::default()
        };
        let p = build_page(docs(4), &q);
        assert_eq!((p.docs.len(), p.has_more, p.skip), (3, true, 6));
        let p = build_page(docs(3), &q);
        assert_eq!((p.docs.len(), p.has_more), (3, false));
        let p = build_page(
            docs(9),
            &Query {
                limit: 0,
                ..Query::default()
            },
        );
        assert_eq!((p.docs.len(), p.has_more), (9, false));
    }
}
