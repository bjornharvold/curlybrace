//! Ready-built domain objects for tests.

use bson::{Binary, Bson, Document, Regex, Timestamp, doc, oid::ObjectId, spec::BinarySubtype};

use crate::connection::{ConnectionConfig, SshAuth, SshConfig, TlsConfig};
use crate::indexes::{IndexKey, IndexOptions, IndexSpec, KeyKind};
use crate::store::Namespace;

pub fn mock_doc() -> Document {
    mock_doc_with_id(ObjectId::new())
}

pub fn mock_doc_with_id(id: impl Into<Bson>) -> Document {
    doc! { "_id": id.into(), "name": "alice", "n": 1_i32 }
}

/// Eight documents whose `_id`s span several BSON types (ObjectId, string, int, compound
/// document), listed out of key order.
pub fn mock_docs_with_mixed_ids() -> Vec<Document> {
    let oid = |last: u8| ObjectId::from_bytes([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, last]);
    vec![
        doc! { "_id": oid(2), "kind": "oid" },
        doc! { "_id": "beta", "kind": "string" },
        doc! { "_id": doc! { "a": 1_i32, "b": "y" }, "kind": "compound" },
        doc! { "_id": 7_i32, "kind": "int" },
        doc! { "_id": oid(1), "kind": "oid" },
        doc! { "_id": "alpha", "kind": "string" },
        doc! { "_id": doc! { "a": 1_i32, "b": "x" }, "kind": "compound" },
        doc! { "_id": 3_i32, "kind": "int" },
    ]
}

pub fn mock_namespace() -> Namespace {
    Namespace::new("testdb", "people")
}

pub fn mock_connection() -> ConnectionConfig {
    ConnectionConfig::new("local", "mongodb://localhost:27017")
}

/// The password every fixture and audit test stores in the keychain. Nothing that is saved,
/// exported, logged or drawn may ever contain it.
pub const MOCK_PASSWORD_MARKER: &str = "hunter2-marker-7f3a";

/// A connection filed under `folder`.
pub fn mock_connection_in_folder(name: &str, folder: &str) -> ConnectionConfig {
    ConnectionConfig {
        folder: Some(folder.into()),
        ..ConnectionConfig::new(name, "mongodb://localhost:27017")
    }
}

/// Three connections with fixed timestamps: one at the root, two in the "Production" folder, the
/// second of which uses TLS and an SSH tunnel.
pub fn mock_connections() -> Vec<ConnectionConfig> {
    let local = ConnectionConfig {
        created_at: Some(1_700_000_000),
        updated_at: Some(1_700_000_000),
        ..ConnectionConfig::new("Local dev", "mongodb://localhost:27017")
    };
    let replica = ConnectionConfig {
        username: Some("reporting".into()),
        auth_source: Some("admin".into()),
        created_at: Some(1_700_100_000),
        updated_at: Some(1_700_200_000),
        last_connected: Some(1_700_300_000),
        ..mock_connection_in_folder("Production replica", "Production")
    };
    let secure = ConnectionConfig {
        uri: "mongodb://db.internal:27017".into(),
        username: Some("admin".into()),
        tls: TlsConfig {
            enabled: true,
            ..TlsConfig::default()
        },
        ssh: Some(SshConfig {
            host: "bastion.example.com".into(),
            port: 22,
            username: "tunnel".into(),
            auth: SshAuth::Password,
        }),
        created_at: Some(1_700_150_000),
        updated_at: Some(1_700_150_000),
        ..mock_connection_in_folder("Production primary", "Production")
    };
    vec![local, replica, secure]
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

/// A compound index on `customerId` ascending then `createdAt` descending, with no options.
pub fn mock_index_spec() -> IndexSpec {
    mock_index_spec_named(
        "customerId_1_createdAt_-1",
        &[("customerId", KeyKind::Asc), ("createdAt", KeyKind::Desc)],
    )
}

/// An index called `name` over `keys`, with no options. Shape it with struct update syntax.
pub fn mock_index_spec_named(name: &str, keys: &[(&str, KeyKind)]) -> IndexSpec {
    IndexSpec {
        name: name.into(),
        keys: keys
            .iter()
            .map(|(field, kind)| IndexKey {
                field: (*field).into(),
                kind: kind.clone(),
            })
            .collect(),
        options: IndexOptions::default(),
        raw: Document::new(),
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
    fn mock_connections_cover_root_folder_and_tls_with_ssh() {
        let all = mock_connections();
        assert_eq!(all.len(), 3);
        assert_eq!(all.iter().filter(|c| c.folder.is_none()).count(), 1);
        let filed: Vec<_> = all
            .iter()
            .filter(|c| c.folder.as_deref() == Some("Production"))
            .collect();
        assert_eq!(filed.len(), 2);
        assert!(all.iter().any(|c| c.tls.enabled && c.ssh.is_some()));
        assert!(all.iter().all(|c| c.validate().is_ok()));
    }

    #[test]
    fn mock_connection_in_folder_sets_the_folder() {
        let c = mock_connection_in_folder("a", "F");
        assert_eq!(c.folder.as_deref(), Some("F"));
        assert!(c.validate().is_ok());
    }

    #[test]
    fn mock_doc_with_id_uses_given_id() {
        assert_eq!(mock_doc_with_id(5_i32).get_i32("_id"), Ok(5));
    }
}
