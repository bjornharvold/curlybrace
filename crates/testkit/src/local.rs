//! Tier 1: the already-running local server and throwaway database names.
use crate::container::MongoServer;
use crate::gate::local_uri;

/// The server tier-1 tests use. Never starts anything.
pub fn local_server() -> MongoServer {
    MongoServer::External(local_uri())
}

/// A unique throwaway database name `cbit_<12 hex>` (plus `_<suffix>` when given). Tier-1 tests
/// write only to databases named this way.
pub fn unique_db_name(suffix: &str) -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    let base = format!("cbit_{}", &id[..12]);
    if suffix.is_empty() {
        base
    } else {
        format!("{base}_{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_prefixed() {
        let (a, b) = (unique_db_name(""), unique_db_name("x"));
        assert!(a.starts_with("cbit_") && b.starts_with("cbit_") && b.ends_with("_x"));
        assert_ne!(a, unique_db_name(""));
    }
}
