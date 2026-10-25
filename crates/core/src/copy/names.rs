//! Name rules for copy targets: which databases and collections are never copied, how a
//! free "copy" name is picked, and MongoDB's naming limits. Pure; no I/O.

use crate::store::Namespace;

const EXCLUDED_DATABASES: [&str; 3] = ["admin", "local", "config"];
const SYSTEM_PREFIX: &str = "system.";
/// Characters MongoDB forbids in a database name (NUL is checked separately).
const FORBIDDEN_DB_CHARS: &str = "/\\. \"$*<>:|?";
const MAX_DATABASE_BYTES: usize = 63;
const MAX_NAMESPACE_BYTES: usize = 255;

/// Why a name cannot be used as a copy target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameProblem {
    Empty,
    TooLong,
    ForbiddenChar,
    SystemPrefix,
}

/// Server-owned databases are never copied.
pub fn is_excluded_database(name: &str) -> bool {
    EXCLUDED_DATABASES.contains(&name)
}

/// `system.*` collections are server-owned and never copied.
pub fn is_excluded_collection(name: &str) -> bool {
    name.starts_with(SYSTEM_PREFIX)
}

/// `wanted_copy`, or `wanted_copy2`, `wanted_copy3`... for the first name not in `taken`.
pub fn unique_copy_name(wanted: &str, taken: &[String]) -> String {
    let base = format!("{wanted}_copy");
    std::iter::once(base.clone())
        .chain((2..).map(|n| format!("{base}{n}")))
        .find(|candidate| !taken.contains(candidate))
        .expect("an unbounded counter always yields a free name")
}

pub fn validate_database_name(name: &str) -> Result<(), NameProblem> {
    if name.is_empty() {
        return Err(NameProblem::Empty);
    }
    if name.len() > MAX_DATABASE_BYTES {
        return Err(NameProblem::TooLong);
    }
    if name
        .chars()
        .any(|c| c == '\0' || FORBIDDEN_DB_CHARS.contains(c))
    {
        return Err(NameProblem::ForbiddenChar);
    }
    Ok(())
}

/// Checks the collection part only; dots, spaces and unicode are valid. The `db.collection`
/// length limit needs the database too, see [`validate_namespace`].
pub fn validate_collection_name(name: &str) -> Result<(), NameProblem> {
    if name.is_empty() {
        return Err(NameProblem::Empty);
    }
    if name.contains(['$', '\0']) {
        return Err(NameProblem::ForbiddenChar);
    }
    if is_excluded_collection(name) {
        return Err(NameProblem::SystemPrefix);
    }
    Ok(())
}

/// Both names are valid and `db.collection` fits MongoDB's 255-byte namespace limit.
pub fn validate_namespace(ns: &Namespace) -> Result<(), NameProblem> {
    validate_database_name(&ns.db)?;
    validate_collection_name(&ns.collection)?;
    if ns.db.len() + 1 + ns.collection.len() > MAX_NAMESPACE_BYTES {
        return Err(NameProblem::TooLong);
    }
    Ok(())
}

pub fn is_same_namespace(a: &Namespace, b: &Namespace) -> bool {
    a == b
}

#[cfg(test)]
#[path = "names_tests.rs"]
mod tests;
