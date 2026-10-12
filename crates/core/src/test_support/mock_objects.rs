//! Ready-built domain objects for tests.

use bson::{Binary, Bson, Document, Regex, Timestamp, doc, oid::ObjectId, spec::BinarySubtype};

use crate::connection::ConnectionConfig;
use crate::store::Namespace;

pub fn mock_doc() -> Document {
    mock_doc_with_id(ObjectId::new())
}

pub fn mock_doc_with_id(id: impl Into<Bson>) -> Document {
    doc! { "_id": id.into(), "name": "alice", "n": 1_i32 }
}

pub fn mock_namespace() -> Namespace {
    Namespace::new("testdb", "people")
}

pub fn mock_connection() -> ConnectionConfig {
    ConnectionConfig::new("local", "mongodb://localhost:27017")
}

/// A document holding one value of every BSON type the viewer must render.
pub fn exotic_doc() -> Document {
    doc! {
        "_id": ObjectId::new(),
        "int32": 7_i32,
        "int64": 1_i64 << 40,
        "double": 1.5_f64,
        "nan": f64::NAN,
        "decimal": Bson::Decimal128("1.5".parse().expect("valid decimal128")),
        "binary": Binary { subtype: BinarySubtype::Generic, bytes: vec![0, 1, 2, 255] },
        "date": bson::DateTime::from_millis(1_700_000_000_000),
        "minKey": Bson::MinKey,
        "maxKey": Bson::MaxKey,
        "nested": { "a": { "b": 1_i32 } },
        "array": [1_i32, "two", { "three": 3_i32 }],
        "regex": Regex { pattern: "^a.*z$".into(), options: "i".into() },
        "oid": ObjectId::new(),
        "timestamp": Timestamp { time: 1_700_000_000, increment: 3 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exotic_doc_covers_every_required_type() {
        let d = exotic_doc();
        let kinds: Vec<_> = d.values().map(Bson::element_type).collect();
        use bson::spec::ElementType::*;
        for want in [
            Int32,
            Int64,
            Double,
            Decimal128,
            Binary,
            DateTime,
            MinKey,
            MaxKey,
            EmbeddedDocument,
            Array,
            RegularExpression,
            ObjectId,
            Timestamp,
        ] {
            assert!(kinds.contains(&want), "missing {want:?}");
        }
        assert!(d.get_f64("nan").unwrap().is_nan());
    }

    #[test]
    fn mock_doc_with_id_uses_given_id() {
        assert_eq!(mock_doc_with_id(5_i32).get_i32("_id"), Ok(5));
    }
}
