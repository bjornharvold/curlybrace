//! Administration of [`crate::mongo_store::MongoStore`]: drop, rename, views, duplication and
//! statistics. Names are validated here, in the core, before anything is sent. Driver errors go
//! through the store's `db_err` mapping, except the two codes that have a typed meaning here.

use bson::{Bson, Document, doc};
use futures::TryStreamExt;
use mongodb::Database;

use crate::admin::{
    CollectionStats, DatabaseStats, ensure_user_database, validate_collection_name,
    validate_existing_collection_name, validate_existing_database_name,
};
use crate::driver_error::{
    CODE_COMMAND_NOT_SUPPORTED_ON_VIEW, CODE_NAMESPACE_EXISTS, CODE_NAMESPACE_NOT_FOUND, has_code,
};
use crate::error::{CoreError, Result};
use crate::indexes::{IndexSpec, number};
use crate::store::{CollectionOptions, Namespace};

type DbErr = fn(mongodb::error::Error) -> CoreError;

/// The typed error a server code stands for in this module, if it has one.
pub(crate) fn map_admin_code(code: Option<i32>) -> Option<CoreError> {
    match code? {
        CODE_NAMESPACE_NOT_FOUND => Some(CoreError::NotFound),
        CODE_NAMESPACE_EXISTS => Some(CoreError::NamespaceExists),
        _ => None,
    }
}

fn admin_err(db_err: DbErr) -> impl Fn(mongodb::error::Error) -> CoreError {
    move |e| map_admin_code(crate::driver_error::server_error_code(&e)).unwrap_or_else(|| db_err(e))
}

pub(crate) fn drop_collection_command(collection: &str) -> Document {
    doc! { "drop": collection }
}

pub(crate) fn rename_command(ns: &Namespace, new_name: &str, drop_target: bool) -> Document {
    doc! {
        "renameCollection": format!("{}.{}", ns.db, ns.collection),
        "to": format!("{}.{}", ns.db, new_name),
        "dropTarget": drop_target,
    }
}

pub(crate) fn view_command(view: &str, source: &str, pipeline: &[Document]) -> Document {
    let stages: Vec<Bson> = pipeline.iter().cloned().map(Bson::Document).collect();
    doc! { "create": view, "viewOn": source, "pipeline": stages }
}

fn unsigned(reply: &Document, field: &str) -> u64 {
    reply
        .get(field)
        .and_then(number)
        .and_then(|n| u64::try_from(n).ok())
        .unwrap_or(0)
}

pub(crate) fn collection_stats_from(reply: &Document) -> CollectionStats {
    let index_sizes = reply
        .get_document("indexSizes")
        .map(|sizes| {
            sizes
                .iter()
                .map(|(name, size)| {
                    let bytes = number(size)
                        .and_then(|n| u64::try_from(n).ok())
                        .unwrap_or(0);
                    (name.clone(), bytes)
                })
                .collect()
        })
        .unwrap_or_default();
    CollectionStats {
        document_count: unsigned(reply, "count"),
        data_size: unsigned(reply, "size"),
        storage_size: unsigned(reply, "storageSize"),
        total_index_size: unsigned(reply, "totalIndexSize"),
        index_sizes,
    }
}

pub(crate) fn database_stats_from(reply: &Document) -> DatabaseStats {
    DatabaseStats {
        collections: unsigned(reply, "collections"),
        views: unsigned(reply, "views"),
        document_count: unsigned(reply, "objects"),
        data_size: unsigned(reply, "dataSize"),
        storage_size: unsigned(reply, "storageSize"),
        total_index_size: unsigned(reply, "indexSize"),
    }
}

pub(crate) async fn drop_collection(db: &Database, ns: &Namespace, db_err: DbErr) -> Result<()> {
    ensure_user_database(&ns.db)?;
    validate_existing_collection_name(&ns.collection)?;
    match db
        .run_command(drop_collection_command(&ns.collection))
        .await
    {
        Ok(_) => Ok(()),
        // Older servers refuse to drop what is not there; the user's goal is already met.
        Err(e) if has_code(&e, &[CODE_NAMESPACE_NOT_FOUND]) => Ok(()),
        Err(e) => Err(db_err(e)),
    }
}

pub(crate) async fn drop_database(db: &Database, name: &str, db_err: DbErr) -> Result<()> {
    validate_existing_database_name(name)?;
    match db.drop().await {
        Ok(()) => Ok(()),
        // Same as a collection: what is not there is already gone.
        Err(e) if has_code(&e, &[CODE_NAMESPACE_NOT_FOUND]) => Ok(()),
        Err(e) => Err(db_err(e)),
    }
}

/// `renameCollection` runs against `admin` (`admin_db`) and names both namespaces in full.
pub(crate) async fn rename_collection(
    admin_db: &Database,
    ns: &Namespace,
    new_name: &str,
    drop_target: bool,
    db_err: DbErr,
) -> Result<()> {
    ensure_user_database(&ns.db)?;
    validate_existing_collection_name(&ns.collection)?;
    validate_collection_name(new_name)?;
    admin_db
        .run_command(rename_command(ns, new_name, drop_target))
        .await
        .map(|_| ())
        .map_err(admin_err(db_err))
}

pub(crate) async fn create_view(
    db: &Database,
    ns: &Namespace,
    source: &str,
    pipeline: &[Document],
    db_err: DbErr,
) -> Result<()> {
    ensure_user_database(&ns.db)?;
    validate_collection_name(&ns.collection)?;
    validate_existing_collection_name(source)?;
    db.run_command(view_command(&ns.collection, source, pipeline))
        .await
        .map(|_| ())
        .map_err(admin_err(db_err))
}

pub(crate) async fn collection_stats(
    db: &Database,
    ns: &Namespace,
    db_err: DbErr,
) -> Result<CollectionStats> {
    let reply = db
        .run_command(doc! { "collStats": &ns.collection })
        .await
        .map_err(|e| {
            // A view has no storage of its own, so the server refuses its statistics.
            if has_code(&e, &[CODE_COMMAND_NOT_SUPPORTED_ON_VIEW]) {
                CoreError::Unsupported("statistics are not available for a view".into())
            } else {
                admin_err(db_err)(e)
            }
        })?;
    Ok(collection_stats_from(&reply))
}

pub(crate) async fn database_stats(db: &Database, db_err: DbErr) -> Result<DatabaseStats> {
    let reply = db
        .run_command(doc! { "dbStats": 1 })
        .await
        .map_err(admin_err(db_err))?;
    Ok(database_stats_from(&reply))
}

/// Copies `source` into the new collection `target`: creates it with `options`, merges every
/// document in (a duplicate `_id` fails rather than overwriting), then rebuilds `indexes` (the
/// implicit `_id_` index is skipped). The caller has already checked that `target` is absent.
///
/// Once the create has succeeded the target is ours, so a later failure drops it again: left
/// behind, a half-copied target would make every retry answer `NamespaceExists`. A failed create
/// is not rolled back, since then the target may be someone else's.
pub(crate) async fn copy_collection(
    db: &Database,
    source: &Namespace,
    target: &str,
    options: &CollectionOptions,
    indexes: &[IndexSpec],
    db_err: DbErr,
) -> Result<()> {
    ensure_user_database(&source.db)?;
    validate_collection_name(target)?;
    let target_ns = Namespace::new(&source.db, target);
    crate::mongo_collection_options::create_collection(db, &target_ns, options, admin_err(db_err))
        .await?;
    let Err(error) = fill_copy(db, source, &target_ns, indexes, db_err).await else {
        return Ok(());
    };
    if let Err(rollback) = drop_collection(db, &target_ns, db_err).await {
        log::error!(
            "duplicate of {}.{} failed and the half-made copy {} could not be dropped: {rollback}",
            source.db,
            source.collection,
            target_ns.collection
        );
    }
    Err(error)
}

/// The documents and indexes of a copy whose target collection exists.
async fn fill_copy(
    db: &Database,
    source: &Namespace,
    target_ns: &Namespace,
    indexes: &[IndexSpec],
    db_err: DbErr,
) -> Result<()> {
    let merge = doc! { "$merge": {
        "into": &target_ns.collection, "whenMatched": "fail", "whenNotMatched": "insert"
    } };
    let mut cursor = db
        .collection::<Document>(&source.collection)
        .aggregate(vec![merge])
        .await
        .map_err(db_err)?;
    while cursor.try_next().await.map_err(db_err)?.is_some() {}
    for spec in indexes.iter().filter(|s| s.name != "_id_") {
        crate::mongo_indexes::create_index(db, target_ns, spec, db_err).await?;
    }
    Ok(())
}
