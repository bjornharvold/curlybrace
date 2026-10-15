//! [`DocumentStore`] backed by the real MongoDB driver.

use std::time::Duration;

use async_trait::async_trait;
use bson::{Bson, Document, doc};
use futures::TryStreamExt;
use mongodb::Client;
use mongodb::action::Action;

use crate::compare::key::{KeySpec, KeyTuple, compare_tuples, extract_key};
use crate::compare::keyset::{IndexInfo, index_serves_sort, keyset_filter};
use crate::error::{CoreError, Result};
use crate::store::{
    CollectionKind, DocumentStore, DuplicateGroup, IndexStatus, Namespace, Page, Query, Topology,
    WriteOp, WriteOutcome,
};
use mongodb::options::{AggregateOptions, Collation, ReadPreference, SelectionCriteria};

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

    /// The key-ordered scan; `criteria` overrides the connection's read preference (`None`
    /// keeps it).
    async fn sorted_scan(
        &self,
        ns: &Namespace,
        filter: &Document,
        key: &KeySpec,
        from: Option<&KeyTuple>,
        limit: u32,
        criteria: Option<SelectionCriteria>,
    ) -> Result<Vec<Document>> {
        let query = SortedScanQuery::new(filter, key, from, limit);
        let cursor = self
            .collection(ns)
            .find(query.filter)
            .sort(query.sort)
            .limit(query.limit)
            .collation(Collation::builder().locale("simple").build())
            .allow_disk_use(true)
            .optional(criteria, |action, c| action.selection_criteria(c))
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
}

/// How long past the client-side duplicate bound the server keeps working before it gives up.
const DUPLICATE_SERVER_GRACE: Duration = Duration::from_secs(10);

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

/// The exact query `MongoStore::scan_sorted` sends: the keyset filter, the ascending key sort and
/// the batch limit, always under simple collation. Public so tier-1 tests can `explain` it.
#[derive(Clone, Debug, PartialEq)]
pub struct SortedScanQuery {
    pub filter: Document,
    pub sort: Document,
    pub limit: i64,
}

impl SortedScanQuery {
    pub fn new(filter: &Document, key: &KeySpec, from: Option<&KeyTuple>, limit: u32) -> Self {
        Self {
            filter: keyset_filter(filter, key, from),
            sort: key
                .fields
                .iter()
                .map(|f| (f.clone(), Bson::Int32(1)))
                .collect(),
            limit: i64::from(limit),
        }
    }

    /// The equivalent `find` command for `collection` (e.g. to wrap in an `explain`).
    pub fn find_command(&self, collection: &str) -> Document {
        doc! {
            "find": collection,
            "filter": self.filter.clone(),
            "sort": self.sort.clone(),
            "limit": self.limit,
            "collation": { "locale": "simple" },
            "allowDiskUse": true,
        }
    }
}

fn id_filter(id: &Bson) -> Document {
    doc! { "_id": id.clone() }
}

#[async_trait]
impl DocumentStore for MongoStore {
    async fn ping(&self) -> Result<()> {
        self.client
            .database("admin")
            .run_command(doc! { "ping": 1 })
            .await
            .map(|_| ())
            .map_err(db_err)
    }

    async fn topology(&self) -> Result<Option<Topology>> {
        let reply = self
            .client
            .database("admin")
            .run_command(doc! { "hello": 1 })
            .await;
        // Informational: a driver error is `None` and its text is not logged.
        Ok(reply.ok().as_ref().and_then(classify_hello))
    }

    async fn server_version(&self) -> Result<Option<String>> {
        let info = self
            .client
            .database("admin")
            .run_command(doc! { "buildInfo": 1 })
            .await;
        // Informational: a driver error is `None` and its text is not logged.
        Ok(info
            .ok()
            .and_then(|d| d.get_str("version").ok().map(str::to_owned)))
    }

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

    async fn bulk_write(&self, ns: &Namespace, ops: Vec<WriteOp>) -> Result<Vec<WriteOutcome>> {
        let db = self.client.database(&ns.db);
        crate::mongo_write::bulk_write(&db, &ns.collection, ops, db_err).await
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
        self.sorted_scan(ns, filter, key, from, limit, None).await
    }

    async fn scan_sorted_primary(
        &self,
        ns: &Namespace,
        filter: &Document,
        key: &KeySpec,
        from: Option<&KeyTuple>,
        limit: u32,
    ) -> Result<Vec<Document>> {
        let primary = SelectionCriteria::ReadPreference(ReadPreference::Primary);
        self.sorted_scan(ns, filter, key, from, limit, Some(primary))
            .await
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
                hidden: options.hidden == Some(true),
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

    async fn duplicate_keys(
        &self,
        ns: &Namespace,
        filter: &Document,
        key: &KeySpec,
        limit: u32,
    ) -> Result<Vec<DuplicateGroup>> {
        // The server rejects `$limit: 0`.
        if limit == 0 {
            return Ok(Vec::new());
        }
        // The aggregation cannot reference a `$`-prefixed field (e.g. a DBRef `owner.$id`).
        if key.has_dollar_component() {
            return Err(CoreError::CompareSetupInvalid);
        }
        let cursor = self
            .collection(ns)
            .aggregate(duplicate_keys_pipeline(filter, key, limit))
            .with_options(duplicate_keys_options())
            .await
            .map_err(db_err)?;
        let rows: Vec<Document> = cursor.try_collect().await.map_err(db_err)?;
        let mut groups = rows
            .iter()
            .map(|row| group_from_row(row, key.fields.len()))
            .collect::<Result<Vec<_>>>()?;
        // Normaliser: most frequent first, ties in key order, like the in-memory store.
        groups.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| compare_tuples(&a.key, &b.key))
        });
        Ok(groups)
    }
}

/// `$expr` terms excluding documents where a key path crosses an array. On the server `a.b` with
/// an array `a` is "present" when any element has `b` and groups as an array of values, whereas
/// `key_values` (and the scanner) treat such a path as missing; this keeps the stores agreeing.
fn no_array_prefix_guards(key: &KeySpec) -> Vec<Document> {
    key.fields
        .iter()
        .flat_map(|path| {
            path.match_indices('.')
                .map(|(at, _)| path[..at].to_owned())
                .collect::<Vec<_>>()
        })
        .map(|prefix| doc! { "$not": [{ "$isArray": format!("${prefix}") }] })
        .collect()
}

/// The aggregation behind `duplicate_keys`: documents missing a key field are dropped, the rest
/// are grouped on aliased fields (`k0`, `k1`, ...) so any path is a valid group key, and only
/// groups of two or more survive. Read-only (no `$out` / `$merge`).
pub(crate) fn duplicate_keys_pipeline(
    filter: &Document,
    key: &KeySpec,
    limit: u32,
) -> Vec<Document> {
    let mut group_id = Document::new();
    let mut matcher = filter.clone();
    for (i, path) in key.fields.iter().enumerate() {
        group_id.insert(format!("k{i}"), format!("${path}"));
        let present = doc! { path.as_str(): { "$exists": true } };
        matcher = if matcher.is_empty() {
            present
        } else {
            doc! { "$and": [matcher, present] }
        };
    }
    let guards = no_array_prefix_guards(key);
    if !guards.is_empty() {
        matcher = doc! { "$and": [matcher, { "$expr": { "$and": guards } }] };
    }
    vec![
        doc! { "$match": matcher },
        doc! { "$group": { "_id": group_id, "n": { "$sum": 1_i64 } } },
        doc! { "$match": { "n": { "$gt": 1_i64 } } },
        // Count descending, then key order (every `_id` has the same fields k0..kN in order), so
        // ties at the limit are cut deterministically, like the in-memory store.
        doc! { "$sort": { "n": -1_i32, "_id": 1_i32 } },
        doc! { "$limit": i64::from(limit) },
    ]
}

/// Reads the deployment kind out of a `hello` reply: a `mongos` says `msg: "isdbgrid"`, a replica
/// set member carries a string `setName`, anything else is a standalone server. The unknown
/// (`None`) case is a failed or timed-out read and lives at the call site, not here.
pub fn classify_hello(reply: &Document) -> Option<Topology> {
    if reply.get_str("msg") == Ok("isdbgrid") {
        Some(Topology::Sharded)
    } else if reply.get_str("setName").is_ok() {
        Some(Topology::ReplicaSet)
    } else {
        Some(Topology::Direct)
    }
}

/// Options of the duplicate-key aggregation: binary collation, spill to disk, and a server-side
/// time limit (the runtime's client-side bound only abandons the call; this stops the work).
/// The server limit is the client bound plus a grace, so the client timeout normally wins.
pub(crate) fn duplicate_keys_options() -> AggregateOptions {
    AggregateOptions::builder()
        .collation(Collation::builder().locale("simple").build())
        .allow_disk_use(true)
        .max_time(crate::runtime::DUPLICATE_TIMEOUT + DUPLICATE_SERVER_GRACE)
        .build()
}

/// One `$group` row as a [`DuplicateGroup`].
fn group_from_row(row: &Document, fields: usize) -> Result<DuplicateGroup> {
    let malformed = || CoreError::Database("unexpected aggregation result".into());
    let id = row.get_document("_id").map_err(|_| malformed())?;
    let values = (0..fields)
        .map(|i| id.get(format!("k{i}")).cloned().ok_or_else(malformed))
        .collect::<Result<Vec<_>>>()?;
    let count = match row.get("n") {
        Some(Bson::Int32(n)) => u64::try_from(*n).ok(),
        Some(Bson::Int64(n)) => u64::try_from(*n).ok(),
        _ => None,
    };
    Ok(DuplicateGroup {
        key: KeyTuple(values),
        count: count.ok_or_else(malformed)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mock_objects::mock_doc_with_id;

    #[test]
    fn classify_hello_tells_sharded_replica_set_and_direct_apart() {
        assert_eq!(
            classify_hello(&doc! { "msg": "isdbgrid" }),
            Some(Topology::Sharded)
        );
        assert_eq!(
            classify_hello(&doc! { "setName": "rs0", "isWritablePrimary": true }),
            Some(Topology::ReplicaSet)
        );
        assert_eq!(
            classify_hello(&doc! { "isWritablePrimary": true }),
            Some(Topology::Direct)
        );
        assert_eq!(classify_hello(&doc! {}), Some(Topology::Direct));
        assert_eq!(
            classify_hello(&doc! { "setName": 1 }),
            Some(Topology::Direct)
        );
    }

    #[test]
    fn duplicate_keys_pipeline_is_read_only_ordered_and_guards_array_prefixes() {
        let key = KeySpec::new(vec!["x".into(), "a.b.c".into()]).unwrap();
        let filter = doc! { "$or": [{ "t": 1 }, { "t": 2 }] };
        let pipeline = duplicate_keys_pipeline(&filter, &key, 7);
        let text = format!("{pipeline:?}");
        assert!(!text.contains("$out") && !text.contains("$merge"));
        let matcher = pipeline[0].get_document("$match").unwrap();
        assert!(
            format!("{matcher:?}").contains(&format!("{filter:?}")),
            "the user filter stays intact inside $and: {matcher:?}"
        );
        let guards = no_array_prefix_guards(&key);
        let prefixes: Vec<String> = guards.iter().map(|g| format!("{g:?}")).collect();
        assert_eq!(prefixes.len(), 2);
        assert!(prefixes[0].contains("$a\"") && prefixes[1].contains("$a.b\""));
        let id = pipeline[1]
            .get_document("$group")
            .unwrap()
            .get_document("_id")
            .unwrap();
        assert_eq!(id.keys().collect::<Vec<_>>(), ["k0", "k1"]);
        assert_eq!(id.get_str("k1"), Ok("$a.b.c"));
        let sort = pipeline[3].get_document("$sort").unwrap();
        assert_eq!(sort.keys().collect::<Vec<_>>(), ["n", "_id"]);
        assert_eq!(pipeline[4].get_i64("$limit"), Ok(7));
    }

    #[test]
    fn duplicate_keys_options_carry_the_server_side_time_limit() {
        let options = duplicate_keys_options();
        assert_eq!(
            options.max_time,
            Some(crate::runtime::DUPLICATE_TIMEOUT + DUPLICATE_SERVER_GRACE)
        );
        assert_eq!(options.allow_disk_use, Some(true));
        assert!(options.collation.is_some());
    }

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
