//! Create/replace/delete of documents from editor text, keeping `_id` intact.

use std::sync::Arc;

use bson::{Bson, Document};

use crate::ejson::editor_json_to_doc;
use crate::error::{CoreError, Result};
use crate::store::{DocumentStore, Namespace};

/// Returns the document's `_id`, or `InvalidDocument` when it has none.
pub fn id_of(doc: &Document) -> Result<&Bson> {
    doc.get("_id")
        .ok_or_else(|| CoreError::InvalidDocument("document has no _id".into()))
}

/// Document mutations on top of a [`DocumentStore`]. `S` may be
/// `dyn DocumentStore`.
pub struct DocumentService<S: DocumentStore + ?Sized> {
    store: Arc<S>,
}

impl<S: DocumentStore + ?Sized> DocumentService<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }

    /// Parses editor JSON and inserts it; returns the stored `_id`.
    pub async fn insert_from_json(&self, ns: &Namespace, text: &str) -> Result<Bson> {
        let doc = editor_json_to_doc(text)?;
        self.store.insert(ns, doc).await
    }

    /// Replaces the document `original_id` with the edited text. The text must
    /// carry an `_id` equal to `original_id` (`_id` is immutable). Equality is
    /// exact `Bson` equality: canonical editor JSON preserves the `_id` kind,
    /// so an unedited `_id` always compares equal.
    pub async fn replace_from_json(
        &self,
        ns: &Namespace,
        original_id: &Bson,
        text: &str,
    ) -> Result<()> {
        let doc = editor_json_to_doc(text)?;
        if id_of(&doc)? != original_id {
            return Err(CoreError::InvalidDocument("_id cannot be changed".into()));
        }
        self.store.replace(ns, original_id, doc).await
    }

    pub async fn delete(&self, ns: &Namespace, id: &Bson) -> Result<()> {
        self.store.delete(ns, id).await
    }

    /// Inserts a copy of `source` without its `_id`; the store assigns a new one.
    pub async fn clone_document(&self, ns: &Namespace, source: &Document) -> Result<Bson> {
        let copy: Document = source
            .iter()
            .filter(|(key, _)| key.as_str() != "_id")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        self.store.insert(ns, copy).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ejson::doc_to_editor_json;
    use crate::store::Query;
    use crate::test_support::InMemoryStore;
    use crate::test_support::mock_objects::{mock_doc_with_id, mock_namespace};
    use bson::oid::ObjectId;
    use bson::spec::BinarySubtype;
    use bson::{Binary, doc};

    fn uuid_id(seed: u8) -> Bson {
        Bson::Binary(Binary {
            subtype: BinarySubtype::Uuid,
            bytes: vec![seed; 16],
        })
    }

    /// (label, first id, second id) for every `_id` kind under test.
    fn id_kinds() -> Vec<(&'static str, Bson, Bson)> {
        vec![
            (
                "objectId",
                Bson::ObjectId(ObjectId::new()),
                Bson::ObjectId(ObjectId::new()),
            ),
            ("string", "a".into(), "b".into()),
            ("int32", Bson::Int32(1), Bson::Int32(2)),
            ("int64", Bson::Int64(1), Bson::Int64(2)),
            ("uuid", uuid_id(1), uuid_id(2)),
            (
                "compound",
                Bson::Document(doc! { "a": 1_i32, "b": "x" }),
                Bson::Document(doc! { "a": 2_i32, "b": "x" }),
            ),
        ]
    }

    fn seeded(first: &Bson, second: &Bson) -> (Arc<InMemoryStore>, DocumentService<InMemoryStore>) {
        let store = Arc::new(InMemoryStore::new().with_docs(
            &mock_namespace(),
            vec![
                mock_doc_with_id(first.clone()),
                mock_doc_with_id(second.clone()),
            ],
        ));
        (store.clone(), DocumentService::new(store))
    }

    async fn all(store: &InMemoryStore) -> Vec<Document> {
        let q = Query {
            limit: 100,
            ..Query::default()
        };
        store.find(&mock_namespace(), &q).await.unwrap().docs
    }

    #[tokio::test]
    async fn replace_changes_only_the_target_for_every_id_kind() {
        for (kind, first, second) in id_kinds() {
            let (store, svc) = seeded(&first, &second);
            let before = all(&store).await;
            let edited = doc! { "_id": first.clone(), "name": "edited", "extra": 7_i64 };
            let text = doc_to_editor_json(&edited);
            svc.replace_from_json(&mock_namespace(), &first, &text)
                .await
                .unwrap_or_else(|e| panic!("{kind}: {e}"));
            let after = all(&store).await;
            assert_eq!(after.len(), 2, "{kind}");
            assert_eq!(after[0], edited, "{kind}: edited doc returned by find");
            assert_eq!(after[1], before[1], "{kind}: other doc untouched");
        }
    }

    #[tokio::test]
    async fn delete_removes_only_the_target_for_every_id_kind() {
        for (kind, first, second) in id_kinds() {
            let (store, svc) = seeded(&first, &second);
            svc.delete(&mock_namespace(), &first)
                .await
                .unwrap_or_else(|e| panic!("{kind}: {e}"));
            let after = all(&store).await;
            assert_eq!(after, vec![mock_doc_with_id(second.clone())], "{kind}");
        }
    }

    #[tokio::test]
    async fn delete_of_missing_id_is_not_found() {
        let (_, svc) = seeded(&Bson::Int32(1), &Bson::Int32(2));
        let r = svc.delete(&mock_namespace(), &Bson::Int32(9)).await;
        assert_eq!(r, Err(CoreError::NotFound));
    }

    #[tokio::test]
    async fn editing_the_id_is_rejected_and_store_unchanged() {
        for (kind, first, second) in id_kinds() {
            let (store, svc) = seeded(&first, &second);
            let before = all(&store).await;
            let text = doc_to_editor_json(&doc! { "_id": second.clone(), "name": "x" });
            let r = svc
                .replace_from_json(&mock_namespace(), &first, &text)
                .await;
            assert!(
                matches!(r, Err(CoreError::InvalidDocument(_))),
                "{kind}: {r:?}"
            );
            assert_eq!(all(&store).await, before, "{kind}");
        }
    }

    #[tokio::test]
    async fn changing_id_width_is_rejected() {
        let (store, svc) = seeded(&Bson::Int32(1), &Bson::Int32(2));
        let before = all(&store).await;
        let text = doc_to_editor_json(&doc! { "_id": 1_i64 });
        let r = svc
            .replace_from_json(&mock_namespace(), &Bson::Int32(1), &text)
            .await;
        assert!(matches!(r, Err(CoreError::InvalidDocument(_))), "{r:?}");
        assert_eq!(all(&store).await, before);
    }

    #[tokio::test]
    async fn replace_without_id_is_rejected() {
        let (store, svc) = seeded(&Bson::Int32(1), &Bson::Int32(2));
        let before = all(&store).await;
        let r = svc
            .replace_from_json(&mock_namespace(), &Bson::Int32(1), "{\"name\": \"x\"}")
            .await;
        assert!(matches!(r, Err(CoreError::InvalidDocument(_))), "{r:?}");
        assert_eq!(all(&store).await, before);
    }

    #[tokio::test]
    async fn replace_with_invalid_json_is_invalid_json() {
        let (_, svc) = seeded(&Bson::Int32(1), &Bson::Int32(2));
        let r = svc
            .replace_from_json(&mock_namespace(), &Bson::Int32(1), "{")
            .await;
        assert!(matches!(r, Err(CoreError::InvalidJson(_))), "{r:?}");
    }

    #[tokio::test]
    async fn replace_of_missing_document_is_not_found() {
        let (_, svc) = seeded(&Bson::Int32(1), &Bson::Int32(2));
        let text = doc_to_editor_json(&doc! { "_id": 9_i32 });
        let r = svc
            .replace_from_json(&mock_namespace(), &Bson::Int32(9), &text)
            .await;
        assert_eq!(r, Err(CoreError::NotFound));
    }

    #[tokio::test]
    async fn clone_copies_everything_but_a_new_id_and_leaves_source_alone() {
        let source =
            doc! { "_id": "keep-me", "n": 5_i64, "nested": doc! { "z": 1_i32, "a": 2_i32 } };
        let original = source.clone();
        let store =
            Arc::new(InMemoryStore::new().with_docs(&mock_namespace(), vec![source.clone()]));
        let svc = DocumentService::new(store.clone());
        let new_id = svc
            .clone_document(&mock_namespace(), &source)
            .await
            .unwrap();
        assert!(matches!(new_id, Bson::ObjectId(_)));
        assert_eq!(source, original);
        let docs = all(&store).await;
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0], original);
        let copy = &docs[1];
        assert_eq!(copy.get("_id"), Some(&new_id));
        let mut without_id = copy.clone();
        without_id.remove("_id");
        let mut expected = original.clone();
        expected.remove("_id");
        assert_eq!(without_id, expected);
    }

    #[tokio::test]
    async fn insert_from_json_returns_the_provided_id() {
        let store = Arc::new(InMemoryStore::new());
        let svc = DocumentService::new(store.clone());
        let text = doc_to_editor_json(&doc! { "_id": 5_i64, "a": 1_i32 });
        let id = svc
            .insert_from_json(&mock_namespace(), &text)
            .await
            .unwrap();
        assert_eq!(id, Bson::Int64(5));
        assert_eq!(all(&store).await, vec![doc! { "_id": 5_i64, "a": 1_i32 }]);
    }

    #[tokio::test]
    async fn insert_from_json_without_id_returns_generated_id() {
        let store = Arc::new(InMemoryStore::new());
        let svc = DocumentService::new(store.clone());
        let id = svc
            .insert_from_json(&mock_namespace(), "{\"a\": 1}")
            .await
            .unwrap();
        assert!(matches!(id, Bson::ObjectId(_)));
        let docs = all(&store).await;
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].get("_id"), Some(&id));
    }

    #[tokio::test]
    async fn insert_from_invalid_json_inserts_nothing() {
        let store = Arc::new(InMemoryStore::new());
        let svc = DocumentService::new(store.clone());
        let r = svc.insert_from_json(&mock_namespace(), "{\"a\":").await;
        assert!(matches!(r, Err(CoreError::InvalidJson(_))), "{r:?}");
        assert!(all(&store).await.is_empty());
    }

    #[tokio::test]
    async fn insert_duplicate_id_surfaces_database_error() {
        let (store, svc) = seeded(&Bson::Int32(1), &Bson::Int32(2));
        let r = svc
            .insert_from_json(&mock_namespace(), "{\"_id\": {\"$numberInt\": \"1\"}}")
            .await;
        assert!(matches!(r, Err(CoreError::Database(_))), "{r:?}");
        assert_eq!(all(&store).await.len(), 2);
    }

    #[test]
    fn id_of_requires_an_id() {
        assert_eq!(id_of(&doc! { "_id": 1_i32 }), Ok(&Bson::Int32(1)));
        assert!(matches!(
            id_of(&doc! { "a": 1_i32 }),
            Err(CoreError::InvalidDocument(_))
        ));
    }

    #[tokio::test]
    async fn works_through_a_trait_object() {
        let store: Arc<dyn DocumentStore> = Arc::new(InMemoryStore::new());
        let svc = DocumentService::new(store);
        let id = svc.insert_from_json(&mock_namespace(), "{}").await.unwrap();
        svc.delete(&mock_namespace(), &id).await.unwrap();
    }
}
