//! Index storage of the in-memory store: per-namespace specs (`_id_` always first) and
//! scripted `$indexStats` rows. The `DocumentStore` methods in `memory_store` delegate here.

use std::collections::BTreeMap;

use super::memory_store::InMemoryStore;
use crate::error::{CoreError, Result};
use crate::indexes::{IndexKey, IndexOptions, IndexSpec, KeyKind};
use crate::store::{CollectionIndex, IndexUsage, Namespace};

/// Every index reports this size; the fake stores no index data.
const FAKE_INDEX_SIZE: u64 = 4096;

pub(super) type NsKey = (String, String);
pub(super) type IndexMap = BTreeMap<NsKey, Vec<IndexSpec>>;
pub(super) type UsageMap = BTreeMap<NsKey, Vec<IndexUsage>>;

fn id_index() -> IndexSpec {
    IndexSpec {
        name: "_id_".into(),
        keys: vec![IndexKey {
            field: "_id".into(),
            kind: KeyKind::Asc,
        }],
        options: IndexOptions::default(),
        raw: bson::Document::new(),
    }
}

/// The indexes of a namespace: `_id_` plus whatever was created.
pub(super) fn list(map: &IndexMap, k: &NsKey) -> Vec<CollectionIndex> {
    std::iter::once(id_index())
        .chain(map.get(k).cloned().unwrap_or_default())
        .map(|spec| CollectionIndex {
            spec,
            size_bytes: Some(FAKE_INDEX_SIZE),
        })
        .collect()
}

pub(super) fn create(map: &mut IndexMap, k: NsKey, spec: &IndexSpec) -> Result<()> {
    let taken = spec.name == "_id_"
        || map
            .get(&k)
            .is_some_and(|v| v.iter().any(|i| i.name == spec.name));
    if taken {
        return Err(CoreError::Database(format!(
            "index {} already exists",
            spec.name
        )));
    }
    map.entry(k).or_default().push(spec.clone());
    Ok(())
}

pub(super) fn drop_named(map: &mut IndexMap, k: &NsKey, name: &str) -> Result<()> {
    if name == "_id_" {
        return Err(CoreError::Database("cannot drop the _id_ index".into()));
    }
    let specs = map.get_mut(k).ok_or(CoreError::NotFound)?;
    let at = specs
        .iter()
        .position(|i| i.name == name)
        .ok_or(CoreError::NotFound)?;
    specs.remove(at);
    Ok(())
}

/// Replaces the options of the index `name` with `change(options)`.
pub(super) fn update_options(
    map: &mut IndexMap,
    k: &NsKey,
    name: &str,
    change: impl FnOnce(IndexOptions) -> IndexOptions,
) -> Result<()> {
    let specs = map.get_mut(k).ok_or(CoreError::NotFound)?;
    let spec = specs
        .iter_mut()
        .find(|i| i.name == name)
        .ok_or(CoreError::NotFound)?;
    *spec = IndexSpec {
        options: change(spec.options.clone()),
        ..spec.clone()
    };
    Ok(())
}

impl InMemoryStore {
    /// Returns a store whose `index_stats` for `ns` answers `usage`.
    pub fn with_index_usage(self, ns: &Namespace, usage: Vec<IndexUsage>) -> Self {
        self.set_index_usage(ns, usage);
        self
    }

    /// Replaces the scripted `index_stats` rows of `ns`, e.g. between two samples.
    pub fn set_index_usage(&self, ns: &Namespace, usage: Vec<IndexUsage>) {
        // A poisoned lock only means a test thread panicked; the map is still usable.
        let mut map = self.usage.lock().unwrap_or_else(|e| e.into_inner());
        map.insert((ns.db.clone(), ns.collection.clone()), usage);
    }
}

#[cfg(test)]
mod tests {
    use super::super::memory_store::InMemoryStore;
    use crate::error::CoreError;
    use crate::indexes::KeyKind;
    use crate::store::{DocumentStore, IndexUsage, Namespace};
    use crate::test_support::mock_objects::*;

    fn usage(name: &str, host: &str, ops: i64) -> IndexUsage {
        IndexUsage {
            name: name.into(),
            host: host.into(),
            ops,
            since_ms: 1_000,
            spec: mock_index_spec_named(name, &[("a", KeyKind::Asc)]),
        }
    }

    async fn names(s: &InMemoryStore, ns: &Namespace) -> Vec<String> {
        let listed = s.list_indexes(ns).await.unwrap();
        listed.into_iter().map(|i| i.spec.name).collect()
    }

    #[tokio::test]
    async fn memory_store_creates_lists_hides_and_drops_an_index() {
        let (s, ns) = (InMemoryStore::new(), mock_namespace());
        assert_eq!(names(&s, &ns).await, vec!["_id_"]);

        let spec = mock_index_spec();
        s.create_index(&ns, &spec).await.unwrap();
        assert_eq!(names(&s, &ns).await, vec!["_id_", &spec.name]);

        s.set_index_hidden(&ns, &spec.name, true).await.unwrap();
        s.set_index_ttl(&ns, &spec.name, 60).await.unwrap();
        let listed = s.list_indexes(&ns).await.unwrap();
        assert!(listed[1].spec.options.hidden);
        assert_eq!(listed[1].spec.options.expire_after_seconds, Some(60));
        assert!(listed[1].size_bytes.is_some_and(|s| s > 0));
        s.set_index_hidden(&ns, &spec.name, false).await.unwrap();
        assert!(!s.list_indexes(&ns).await.unwrap()[1].spec.options.hidden);

        s.drop_index(&ns, &spec.name).await.unwrap();
        assert_eq!(names(&s, &ns).await, vec!["_id_"]);
    }

    #[tokio::test]
    async fn memory_store_rejects_a_duplicate_index_name() {
        let (s, ns) = (InMemoryStore::new(), mock_namespace());
        let spec = mock_index_spec();
        s.create_index(&ns, &spec).await.unwrap();
        assert!(matches!(
            s.create_index(&ns, &spec).await,
            Err(CoreError::Database(_))
        ));
        let id = mock_index_spec_named("_id_", &[("_id", KeyKind::Asc)]);
        assert!(s.create_index(&ns, &id).await.is_err());
        assert_eq!(names(&s, &ns).await.len(), 2);
    }

    #[tokio::test]
    async fn memory_store_refuses_to_change_a_missing_index_or_drop_id() {
        let (s, ns) = (InMemoryStore::new(), mock_namespace());
        assert_eq!(s.drop_index(&ns, "nope").await, Err(CoreError::NotFound));
        assert_eq!(
            s.set_index_hidden(&ns, "nope", true).await,
            Err(CoreError::NotFound)
        );
        assert_eq!(
            s.set_index_ttl(&ns, "nope", 5).await,
            Err(CoreError::NotFound)
        );
        assert!(matches!(
            s.drop_index(&ns, "_id_").await,
            Err(CoreError::Database(_))
        ));
        assert_eq!(names(&s, &ns).await, vec!["_id_"]);
    }

    #[tokio::test]
    async fn memory_store_scripts_index_stats_per_namespace() {
        let ns = mock_namespace();
        let other = Namespace::new("other", "c");
        let s = InMemoryStore::new().with_index_usage(&ns, vec![usage("a_1", "h1", 3)]);
        assert_eq!(s.index_stats(&ns).await.unwrap()[0].ops, 3);
        assert!(s.index_stats(&other).await.unwrap().is_empty());

        s.set_index_usage(&ns, vec![usage("a_1", "h1", 9), usage("a_1", "h2", 4)]);
        let rows = s.index_stats(&ns).await.unwrap();
        assert_eq!(rows.iter().map(|r| r.ops).collect::<Vec<_>>(), vec![9, 4]);
    }
}
