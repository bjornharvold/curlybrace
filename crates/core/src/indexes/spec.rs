//! The typed index definition, read from a server index document and written back as a
//! `createIndexes` command.

use bson::{Bson, Document, doc};
use sha2::{Digest, Sha256};

use crate::ejson::doc_to_view_json;
use crate::error::{CoreError, Result};

/// Fields of an index document that `IndexSpec` models; the rest of `raw` is carried along.
const MODELLED_FIELDS: [&str; 10] = [
    "key",
    "name",
    "unique",
    "sparse",
    "hidden",
    "expireAfterSeconds",
    "partialFilterExpression",
    "collation",
    "wildcardProjection",
    // Server bookkeeping that is not part of a definition.
    "v",
];

#[derive(Clone, Debug, PartialEq)]
pub enum KeyKind {
    Asc,
    Desc,
    Text,
    Geo2dSphere,
    Wildcard,
    Hashed,
    Geo2d,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndexKey {
    pub field: String,
    pub kind: KeyKind,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct IndexOptions {
    pub unique: bool,
    pub sparse: bool,
    pub hidden: bool,
    pub expire_after_seconds: Option<i64>,
    pub partial_filter: Option<Document>,
    pub collation: Option<Document>,
    pub wildcard_projection: Option<Document>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndexSpec {
    pub name: String,
    pub keys: Vec<IndexKey>,
    pub options: IndexOptions,
    /// The server's original document, so fields this type does not model survive a copy.
    pub raw: Document,
}

impl KeyKind {
    fn from_bson(field: &str, value: &Bson) -> Result<Self> {
        if field.ends_with("$**") {
            return Ok(Self::Wildcard);
        }
        match value {
            Bson::String(s) => match s.as_str() {
                "text" => Ok(Self::Text),
                "2dsphere" => Ok(Self::Geo2dSphere),
                "hashed" => Ok(Self::Hashed),
                "2d" => Ok(Self::Geo2d),
                other => Err(invalid(format!("unknown index key type {other:?}"))),
            },
            Bson::Int32(n) => Ok(direction(f64::from(*n))),
            Bson::Int64(n) => Ok(direction(*n as f64)),
            Bson::Double(n) => Ok(direction(*n)),
            _ => Err(invalid(format!(
                "index key {field:?} has an unusable value"
            ))),
        }
    }

    fn to_bson(&self) -> Bson {
        match self {
            Self::Asc | Self::Wildcard => Bson::Int32(1),
            Self::Desc => Bson::Int32(-1),
            Self::Text => Bson::String("text".into()),
            Self::Geo2dSphere => Bson::String("2dsphere".into()),
            Self::Hashed => Bson::String("hashed".into()),
            Self::Geo2d => Bson::String("2d".into()),
        }
    }
}

fn direction(n: f64) -> KeyKind {
    if n < 0.0 { KeyKind::Desc } else { KeyKind::Asc }
}

fn invalid(message: String) -> CoreError {
    CoreError::InvalidDocument(message)
}

fn number(value: &Bson) -> Option<i64> {
    match value {
        Bson::Int32(n) => Some(i64::from(*n)),
        Bson::Int64(n) => Some(*n),
        Bson::Double(n) if n.is_finite() => Some(*n as i64),
        _ => None,
    }
}

fn flag(doc: &Document, field: &str) -> bool {
    doc.get_bool(field).unwrap_or(false)
}

fn sub_document(doc: &Document, field: &str) -> Option<Document> {
    doc.get_document(field).ok().cloned()
}

impl IndexSpec {
    /// Reads one entry of a `listIndexes` reply. A document without `key` or `name` is an error.
    pub fn from_document(doc: &Document) -> Result<IndexSpec> {
        let key_doc = doc
            .get_document("key")
            .map_err(|_| invalid("index has no key".into()))?;
        let name = doc
            .get_str("name")
            .map_err(|_| invalid("index has no name".into()))?
            .to_string();
        let keys = key_doc
            .iter()
            .map(|(field, value)| {
                Ok(IndexKey {
                    field: field.clone(),
                    kind: KeyKind::from_bson(field, value)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let options = IndexOptions {
            unique: flag(doc, "unique"),
            sparse: flag(doc, "sparse"),
            hidden: flag(doc, "hidden"),
            expire_after_seconds: doc.get("expireAfterSeconds").and_then(number),
            partial_filter: sub_document(doc, "partialFilterExpression"),
            collation: sub_document(doc, "collation"),
            wildcard_projection: sub_document(doc, "wildcardProjection"),
        };
        Ok(IndexSpec {
            name,
            keys,
            options,
            raw: doc.clone(),
        })
    }

    /// The `createIndexes` command for this spec. The UI preview and the store both call this.
    pub fn create_command(&self, collection: &str) -> Document {
        doc! { "createIndexes": collection, "indexes": [self.index_document()] }
    }

    /// Hex SHA-256 over what makes two indexes the same: keys in order and every option except
    /// `hidden`, and not the name.
    pub fn definition_hash(&self) -> String {
        let identity = self.definition(false);
        let bytes = bson::to_vec(&identity).expect("a document of plain values serializes");
        Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// The definition as relaxed extended JSON, for "copy as JSON". Fields of `raw` this type
    /// does not model are kept.
    pub fn to_json(&self) -> String {
        let mut document = self.index_document();
        for (field, value) in &self.raw {
            let known = MODELLED_FIELDS.contains(&field.as_str());
            if !known && field != "ns" && !document.contains_key(field) {
                document.insert(field.clone(), value.clone());
            }
        }
        doc_to_view_json(&document)
    }

    /// The index document as `createIndexes` and `listIndexes` write it: key, name, then the
    /// options that are set.
    fn index_document(&self) -> Document {
        let mut document = self.definition(true);
        // Name leads, as the server writes it.
        let mut named = doc! { "key": document.remove("key").unwrap_or_default() };
        named.insert("name", self.name.clone());
        named.extend(document);
        named
    }

    /// Keys and options; `hidden` only when asked for, since it does not change what is indexed.
    fn definition(&self, with_hidden: bool) -> Document {
        let options = &self.options;
        let mut key = Document::new();
        for index_key in &self.keys {
            key.insert(index_key.field.clone(), index_key.kind.to_bson());
        }
        let mut document = doc! { "key": key };
        if options.unique {
            document.insert("unique", true);
        }
        if options.sparse {
            document.insert("sparse", true);
        }
        if with_hidden && options.hidden {
            document.insert("hidden", true);
        }
        if let Some(seconds) = options.expire_after_seconds {
            document.insert("expireAfterSeconds", seconds);
        }
        for (field, value) in [
            ("partialFilterExpression", &options.partial_filter),
            ("collation", &options.collation),
            ("wildcardProjection", &options.wildcard_projection),
        ] {
            if let Some(value) = value {
                document.insert(field, value.clone());
            }
        }
        document
    }
}
