//! Collection options of [`crate::mongo_store::MongoStore`]: read from `listCollections`,
//! applied with `create`. Driver errors go through the store's `db_err` mapping; nothing the
//! server says is logged or carried.

use bson::{Document, doc};
use mongodb::Database;

use crate::error::{CoreError, Result};
use crate::indexes::number;
use crate::store::{CappedSpec, CollectionOptions, Namespace};

fn malformed() -> CoreError {
    CoreError::Database("unexpected collection options reply".into())
}

/// The options of `ns` from a single-collection `listCollections`; `None` when it is absent.
pub(crate) async fn collection_options(
    db: &Database,
    ns: &Namespace,
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> Result<Option<CollectionOptions>> {
    let reply = db
        .run_command(doc! { "listCollections": 1, "filter": { "name": &ns.collection } })
        .await
        .map_err(db_err)?;
    let batch = reply
        .get_document("cursor")
        .and_then(|c| c.get_array("firstBatch"))
        .map_err(|_| malformed())?;
    let Some(entry) = batch.first() else {
        return Ok(None);
    };
    let entry = entry.as_document().ok_or_else(malformed)?;
    options_from_entry(entry).map(Some)
}

pub(crate) async fn create_collection(
    db: &Database,
    ns: &Namespace,
    options: &CollectionOptions,
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> Result<()> {
    db.run_command(create_command(&ns.collection, options))
        .await
        .map(|_| ())
        .map_err(db_err)
}

/// Reads the `options` sub-document of one `listCollections` entry (absent means a plain
/// collection).
fn options_from_entry(entry: &Document) -> Result<CollectionOptions> {
    let Ok(options) = entry.get_document("options") else {
        return Ok(CollectionOptions::default());
    };
    let capped = match options.get_bool("capped") {
        Ok(true) => Some(capped_spec(options)?),
        _ => None,
    };
    Ok(CollectionOptions {
        capped,
        validator: options.get_document("validator").ok().cloned(),
        validation_level: options.get_str("validationLevel").ok().map(str::to_owned),
        validation_action: options.get_str("validationAction").ok().map(str::to_owned),
        collation: options.get_document("collation").ok().cloned(),
    })
}

fn capped_spec(options: &Document) -> Result<CappedSpec> {
    let unsigned = |field: &str| {
        options
            .get(field)
            .and_then(number)
            .and_then(|n| u64::try_from(n).ok())
    };
    Ok(CappedSpec {
        size: unsigned("size").ok_or_else(malformed)?,
        max: unsigned("max"),
    })
}

/// The `create` command for `collection` with every set option.
fn create_command(collection: &str, options: &CollectionOptions) -> Document {
    let mut cmd = doc! { "create": collection };
    if let Some(capped) = &options.capped {
        cmd.insert("capped", true);
        cmd.insert("size", to_i64(capped.size));
        if let Some(max) = capped.max {
            cmd.insert("max", to_i64(max));
        }
    }
    if let Some(validator) = &options.validator {
        cmd.insert("validator", validator.clone());
    }
    if let Some(level) = &options.validation_level {
        cmd.insert("validationLevel", level.as_str());
    }
    if let Some(action) = &options.validation_action {
        cmd.insert("validationAction", action.as_str());
    }
    if let Some(collation) = &options.collation {
        cmd.insert("collation", collation.clone());
    }
    cmd
}

fn to_i64(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rich() -> CollectionOptions {
        CollectionOptions {
            capped: Some(CappedSpec {
                size: 4096,
                max: Some(10),
            }),
            validator: Some(doc! { "a": { "$exists": true } }),
            validation_level: Some("moderate".into()),
            validation_action: Some("warn".into()),
            collation: Some(doc! { "locale": "fr" }),
        }
    }

    #[test]
    fn the_create_command_carries_every_option() {
        let cmd = create_command("c", &rich());
        assert_eq!(
            cmd,
            doc! { "create": "c", "capped": true, "size": 4096_i64, "max": 10_i64,
            "validator": { "a": { "$exists": true } }, "validationLevel": "moderate",
            "validationAction": "warn", "collation": { "locale": "fr" } }
        );
    }

    #[test]
    fn the_create_command_of_a_plain_collection_has_no_options() {
        let cmd = create_command("c", &CollectionOptions::default());
        assert_eq!(cmd, doc! { "create": "c" });
    }

    #[test]
    fn a_listing_entry_parses_back_to_the_options_it_was_created_with() {
        let entry = doc! { "name": "c", "type": "collection", "options": {
        "capped": true, "size": 4096, "max": 10_i64,
        "validator": { "a": { "$exists": true } }, "validationLevel": "moderate",
        "validationAction": "warn", "collation": { "locale": "fr" } } };
        assert_eq!(options_from_entry(&entry), Ok(rich()));
    }

    #[test]
    fn an_entry_without_options_is_a_plain_collection() {
        let entry = doc! { "name": "c", "type": "collection" };
        assert_eq!(options_from_entry(&entry), Ok(CollectionOptions::default()));
        let empty = doc! { "name": "c", "options": {} };
        assert_eq!(options_from_entry(&empty), Ok(CollectionOptions::default()));
    }

    #[test]
    fn a_capped_entry_without_a_size_is_malformed() {
        let entry = doc! { "options": { "capped": true } };
        assert!(options_from_entry(&entry).is_err());
    }
}
