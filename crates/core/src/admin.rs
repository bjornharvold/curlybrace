//! Administration of databases and collections: name rules and the statistics shapes. The
//! store methods live on [`crate::store::DocumentStore`]; the rules are checked here, in the
//! core, so no front end can skip them.

use bson::{Bson, Document};

use crate::error::{CoreError, Result};

/// Databases the server owns; administering them from here would break the deployment.
pub const RESERVED_DATABASES: [&str; 3] = ["admin", "local", "config"];

/// Characters MongoDB refuses in a database name (whitespace is refused too).
const FORBIDDEN_DB_CHARS: [char; 10] = ['/', '\\', '.', '"', '$', '*', '<', '>', ':', '|'];

/// A database name must be shorter than this many bytes.
const MAX_DB_NAME_BYTES: usize = 64;

/// Checks a collection or view name: not blank, no `$` or NUL, not a `system.` collection.
pub fn validate_collection_name(name: &str) -> Result<()> {
    let bad = name.trim().is_empty() || name.contains(['$', '\0']) || name.starts_with("system.");
    if bad {
        Err(CoreError::InvalidNamespace)
    } else {
        Ok(())
    }
}

/// Checks a database name, and refuses the server-owned databases.
pub fn validate_database_name(name: &str) -> Result<()> {
    let bad = name.is_empty()
        || name.len() >= MAX_DB_NAME_BYTES
        || name.contains('\0')
        || name.contains(char::is_whitespace)
        || name.contains(FORBIDDEN_DB_CHARS)
        || name.contains('?');
    if bad {
        return Err(CoreError::InvalidNamespace);
    }
    if RESERVED_DATABASES.contains(&name) {
        return Err(CoreError::ReservedDatabase);
    }
    Ok(())
}

/// Size and count figures of one collection, as the server reports them.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CollectionStats {
    pub document_count: u64,
    /// Uncompressed size of the documents, in bytes.
    pub data_size: u64,
    /// Bytes on disk (compressed).
    pub storage_size: u64,
    pub total_index_size: u64,
    /// `(index name, bytes)`, in the server's order.
    pub index_sizes: Vec<(String, u64)>,
}

/// Size and count figures of one database.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DatabaseStats {
    pub collections: u64,
    pub views: u64,
    pub document_count: u64,
    pub data_size: u64,
    pub storage_size: u64,
    pub total_index_size: u64,
}

/// Parses the text of a view pipeline: a JSON array whose every element is a stage document
/// (extended JSON). Not valid JSON is `InvalidJson`; valid JSON of another shape is
/// `InvalidDocument`. An empty pipeline is allowed (a view that mirrors its source).
pub fn parse_pipeline(text: &str) -> Result<Vec<Document>> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| CoreError::InvalidJson(e.to_string()))?;
    let serde_json::Value::Array(items) = value else {
        return Err(CoreError::InvalidDocument(
            "a pipeline is a JSON array of stages".into(),
        ));
    };
    items
        .into_iter()
        .map(|item| match Bson::try_from(item) {
            Ok(Bson::Document(stage)) => Ok(stage),
            Ok(_) => Err(CoreError::InvalidDocument(
                "every pipeline stage is a document".into(),
            )),
            Err(e) => Err(CoreError::InvalidDocument(e.to_string())),
        })
        .collect()
}
