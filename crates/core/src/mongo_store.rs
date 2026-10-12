//! [`DocumentStore`] backed by the real MongoDB driver.

use async_trait::async_trait;
use bson::{Bson, Document, doc};
use futures::TryStreamExt;
use mongodb::Client;

use crate::error::{CoreError, Result};
use crate::store::{DocumentStore, Namespace, Page, Query};

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
    CoreError::Database(e.to_string())
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
        let mut action = collection.find(q.filter.clone()).skip(q.skip);
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
