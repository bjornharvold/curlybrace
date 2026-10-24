//! Index operations of [`crate::mongo_store::MongoStore`]: `listIndexes` / `collStats` /
//! `$indexStats` for reading, `createIndexes` / `dropIndexes` / `collMod` for writing. Every
//! driver error goes through the same `db_err` mapping as the rest of the store, so a server
//! refusal (dropping `_id_`, a unique index over duplicates) comes back as a typed `CoreError`.

use bson::{Bson, Document, doc};
use futures::TryStreamExt;
use mongodb::Database;
use mongodb::error::ErrorKind;

use crate::error::{CoreError, Result};
use crate::indexes::{IndexSpec, number};
use crate::store::{CollectionIndex, IndexUsage, Namespace};

/// Server error code `NamespaceNotFound`: the collection does not exist.
const CODE_NAMESPACE_NOT_FOUND: i32 = 26;
/// Server error code `CommandNotSupportedOnView`: `listIndexes` / `$indexStats` on a view.
const CODE_COMMAND_NOT_SUPPORTED_ON_VIEW: i32 = 166;
/// Server error code `IndexNotFound`: no index has that name.
const CODE_INDEX_NOT_FOUND: i32 = 27;

/// Whether the driver error is a server command error with one of `codes`.
fn has_code(e: &mongodb::error::Error, codes: &[i32]) -> bool {
    matches!(e.kind.as_ref(), ErrorKind::Command(c) if codes.contains(&c.code))
}

/// Whether a server refusal with `code` means "nothing to list": the collection does not exist,
/// or it is a view, which has no indexes of its own.
fn lists_as_empty(code: i32) -> bool {
    matches!(
        code,
        CODE_NAMESPACE_NOT_FOUND | CODE_COMMAND_NOT_SUPPORTED_ON_VIEW
    )
}

/// Whether the driver error is a server refusal that [`lists_as_empty`].
fn is_empty_listing(e: &mongodb::error::Error) -> bool {
    matches!(e.kind.as_ref(), ErrorKind::Command(c) if lists_as_empty(c.code))
}

fn malformed() -> CoreError {
    CoreError::Database("unexpected index reply".into())
}

/// The indexes of `ns` with their sizes. A missing collection has no indexes (like
/// `estimated_count`, where it counts 0) and neither has a view, so both list as empty rather
/// than as an error.
pub(crate) async fn list_indexes(
    db: &Database,
    ns: &Namespace,
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> Result<Vec<CollectionIndex>> {
    let Some(batch) = listing_batch(db, ns, db_err).await? else {
        return Ok(Vec::new());
    };
    let sizes = index_sizes(db, ns, db_err).await;
    Ok(collection_indexes(&batch, &sizes))
}

/// [`list_indexes`] without the sizes: one `listIndexes` and no `collStats`.
pub(crate) async fn list_index_specs(
    db: &Database,
    ns: &Namespace,
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> Result<Vec<IndexSpec>> {
    let batch = listing_batch(db, ns, db_err).await?;
    Ok(readable_specs(batch.as_deref().unwrap_or_default()))
}

/// The first batch of `listIndexes`; `None` for a collection that is missing or a view.
async fn listing_batch(
    db: &Database,
    ns: &Namespace,
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> Result<Option<Vec<Bson>>> {
    let reply = match db
        .run_command(doc! { "listIndexes": &ns.collection, "cursor": { "batchSize": 1000_i32 } })
        .await
    {
        Ok(reply) => reply,
        Err(e) if is_empty_listing(&e) => return Ok(None),
        Err(e) => return Err(db_err(e)),
    };
    let batch = reply
        .get_document("cursor")
        .and_then(|c| c.get_array("firstBatch"))
        .map_err(|_| malformed())?;
    Ok(Some(batch.clone()))
}

/// The readable entries of a `listIndexes` batch with their sizes (`None` when unknown). An entry the
/// definition model cannot read is skipped with a warning instead of failing the collection.
fn collection_indexes(
    batch: &[Bson],
    sizes: &std::collections::HashMap<String, u64>,
) -> Vec<CollectionIndex> {
    readable_specs(batch)
        .into_iter()
        .map(|spec| {
            let size_bytes = sizes.get(&spec.name).copied();
            CollectionIndex { spec, size_bytes }
        })
        .collect()
}

/// The entries of a `listIndexes` batch the definition model can read; the others are skipped
/// with a warning.
fn readable_specs(batch: &[Bson]) -> Vec<IndexSpec> {
    batch
        .iter()
        .filter_map(|entry| {
            let spec = entry
                .as_document()
                .ok_or_else(malformed)
                .and_then(IndexSpec::from_document);
            spec.inspect_err(|e| {
                log::warn!("skipping an index the listing could not read: {e}: {entry}");
            })
            .ok()
        })
        .collect()
}

/// Index name -> bytes from `collStats.indexSizes`. Sizes are a nicety: when `collStats` is
/// refused (no privilege, a dropped collection) or its reply is odd, every size is unknown
/// (empty map, read as `None`) and the listing is kept.
async fn index_sizes(
    db: &Database,
    ns: &Namespace,
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> std::collections::HashMap<String, u64> {
    let stats = match db.run_command(doc! { "collStats": &ns.collection }).await {
        Ok(stats) => stats,
        Err(e) => {
            // A collection dropped between the two calls is expected; anything else is logged.
            if !has_code(&e, &[CODE_NAMESPACE_NOT_FOUND]) {
                log::warn!(
                    "index sizes of {}.{} are unknown: {}",
                    ns.db,
                    ns.collection,
                    db_err(e)
                );
            }
            return Default::default();
        }
    };
    let Ok(sizes) = stats.get_document("indexSizes") else {
        log::warn!(
            "index sizes of {}.{} are unknown: collStats has no indexSizes",
            ns.db,
            ns.collection
        );
        return Default::default();
    };
    sizes
        .iter()
        .filter_map(|(name, bytes)| {
            let bytes = u64::try_from(number(bytes)?).ok()?;
            Some((name.clone(), bytes))
        })
        .collect()
}

/// `$indexStats` rows for `ns`. Callers must not pass a view: the server answers `$indexStats` on
/// a view with the rows of the collection underneath (the sampler skips views). A time-series
/// collection makes the server refuse the stage with an error that is returned as it is so the
/// caller can classify it.
pub(crate) async fn index_stats(
    db: &Database,
    ns: &Namespace,
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> Result<Vec<IndexUsage>> {
    let cursor = db
        .collection::<Document>(&ns.collection)
        .aggregate(vec![doc! { "$indexStats": {} }])
        .await;
    let cursor = match cursor {
        Ok(cursor) => cursor,
        Err(e) if is_empty_listing(&e) => return Ok(Vec::new()),
        Err(e) => return Err(db_err(e)),
    };
    let rows: Vec<Document> = cursor.try_collect().await.map_err(db_err)?;
    Ok(usages_from_rows(&rows))
}

/// The readable `$indexStats` rows; one that cannot be read is skipped with a warning.
fn usages_from_rows(rows: &[Document]) -> Vec<IndexUsage> {
    rows.iter()
        .filter_map(|row| match usage_from_row(row) {
            Ok(usage) => Some(usage),
            Err(e) => {
                log::warn!("skipping an index usage row that could not be read: {e}: {row}");
                None
            }
        })
        .collect()
}

/// One `$indexStats` row as an [`IndexUsage`].
fn usage_from_row(row: &Document) -> Result<IndexUsage> {
    let accesses = row.get_document("accesses").map_err(|_| malformed())?;
    let ops = accesses.get("ops").and_then(number);
    let since_ms = match accesses.get("since") {
        Some(Bson::DateTime(at)) => Some(at.timestamp_millis()),
        _ => None,
    };
    Ok(IndexUsage {
        name: row.get_str("name").map_err(|_| malformed())?.to_owned(),
        host: row.get_str("host").map_err(|_| malformed())?.to_owned(),
        ops: ops.ok_or_else(malformed)?,
        since_ms: since_ms.ok_or_else(malformed)?,
        spec: IndexSpec::from_document(row.get_document("spec").map_err(|_| malformed())?)?,
    })
}

pub(crate) async fn create_index(
    db: &Database,
    ns: &Namespace,
    spec: &IndexSpec,
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> Result<()> {
    db.run_command(spec.create_command(&ns.collection))
        .await
        .map(|_| ())
        .map_err(db_err)
}

/// `NotFound` when there is no such index (or no such collection).
pub(crate) async fn drop_index(
    db: &Database,
    ns: &Namespace,
    name: &str,
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> Result<()> {
    db.run_command(doc! { "dropIndexes": &ns.collection, "index": name })
        .await
        .map(|_| ())
        .map_err(|e| not_found_or(e, db_err))
}

/// `collMod` of one index: `change` holds the field to set next to the index `name`.
pub(crate) async fn modify_index(
    db: &Database,
    ns: &Namespace,
    name: &str,
    change: (&str, Bson),
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> Result<()> {
    let mut index = doc! { "name": name };
    index.insert(change.0, change.1);
    db.run_command(doc! { "collMod": &ns.collection, "index": index })
        .await
        .map(|_| ())
        .map_err(|e| not_found_or(e, db_err))
}

fn not_found_or(
    e: mongodb::error::Error,
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> CoreError {
    if has_code(&e, &[CODE_NAMESPACE_NOT_FOUND, CODE_INDEX_NOT_FOUND]) {
        CoreError::NotFound
    } else {
        db_err(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::DateTime;

    #[test]
    fn a_stats_row_maps_to_a_usage() {
        let row = doc! {
            "name": "a_1", "key": { "a": 1_i32 }, "host": "h:27017",
            "accesses": { "ops": 7_i64, "since": DateTime::from_millis(1_700_000_000_000) },
            "spec": { "v": 2_i32, "key": { "a": 1_i32 }, "name": "a_1" },
        };
        let usage = usage_from_row(&row).unwrap();
        assert_eq!(
            (usage.name.as_str(), usage.host.as_str()),
            ("a_1", "h:27017")
        );
        assert_eq!((usage.ops, usage.since_ms), (7, 1_700_000_000_000));
        assert_eq!(usage.spec.name, "a_1");
    }

    #[test]
    fn a_malformed_stats_row_is_a_typed_error() {
        assert!(matches!(
            usage_from_row(&doc! { "name": "a_1" }),
            Err(CoreError::Database(_))
        ));
    }

    #[test]
    fn sizes_arrive_as_any_number_type() {
        assert_eq!(number(&Bson::Int32(4)), Some(4));
        assert_eq!(number(&Bson::Int64(5)), Some(5));
        assert_eq!(number(&Bson::Double(6.0)), Some(6));
        assert_eq!(number(&Bson::String("7".into())), None);
    }

    #[test]
    fn views_and_missing_collections_list_as_empty() {
        assert!(lists_as_empty(CODE_NAMESPACE_NOT_FOUND));
        assert!(lists_as_empty(CODE_COMMAND_NOT_SUPPORTED_ON_VIEW));
        assert!(!lists_as_empty(13));
    }

    #[test]
    fn one_unreadable_entry_does_not_hide_the_others() {
        let batch = vec![
            Bson::Document(doc! { "key": { "a": 1_i32 }, "name": "a_1" }),
            Bson::Document(doc! { "key": { "a": { "x": 1_i32 } }, "name": "bad" }),
            Bson::String("junk".into()),
            Bson::Document(doc! { "key": { "loc": "geoHaystack" }, "name": "hay" }),
        ];
        let sizes = std::collections::HashMap::from([("a_1".to_owned(), 8_u64)]);
        let listed = collection_indexes(&batch, &sizes);
        let names: Vec<_> = listed.iter().map(|i| i.spec.name.as_str()).collect();
        assert_eq!(names, ["a_1", "hay"]);
        assert_eq!(
            (listed[0].size_bytes, listed[1].size_bytes),
            (Some(8), None)
        );
    }

    #[test]
    fn stats_rows_that_cannot_be_read_are_skipped() {
        let good = doc! {
            "name": "a_1", "host": "h", "spec": { "key": { "a": 1_i32 }, "name": "a_1" },
            "accesses": { "ops": 1_i64, "since": DateTime::from_millis(1) },
        };
        let usages = usages_from_rows(&[doc! { "name": "x" }, good]);
        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].name, "a_1");
    }
}
