//! Tests for copy name rules, exclusions and unique copy names.

use super::*;

fn taken(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

#[test]
fn unique_copy_name_adds_copy_suffix_then_counts() {
    assert_eq!(unique_copy_name("orders", &[]), "orders_copy");
    assert_eq!(
        unique_copy_name("orders", &taken(&["orders_copy"])),
        "orders_copy2"
    );
    assert_eq!(
        unique_copy_name("orders", &taken(&["orders_copy", "orders_copy2"])),
        "orders_copy3"
    );
    assert_eq!(
        unique_copy_name("orders", &taken(&["orders", "orders_copy3"])),
        "orders_copy"
    );
}

#[test]
fn unique_copy_name_keeps_unicode_and_dots() {
    assert_eq!(unique_copy_name("my.données 1", &[]), "my.données 1_copy");
}

#[test]
fn database_name_rejects_slash_dot_space_dollar_and_null() {
    let bad_names = [
        "a/b", "a\\b", "a.b", "a b", "a\"b", "a$b", "a*b", "a<b", "a>b", "a:b", "a|b", "a?b",
        "a\0b",
    ];
    for bad in bad_names {
        assert_eq!(
            validate_database_name(bad),
            Err(NameProblem::ForbiddenChar),
            "{bad:?}"
        );
    }
    assert_eq!(validate_database_name(""), Err(NameProblem::Empty));
    assert_eq!(validate_database_name("shop_été-2"), Ok(()));
}

#[test]
fn collection_name_rejects_dollar_null_empty_and_system_prefix() {
    assert_eq!(validate_collection_name(""), Err(NameProblem::Empty));
    assert_eq!(
        validate_collection_name("a$b"),
        Err(NameProblem::ForbiddenChar)
    );
    assert_eq!(
        validate_collection_name("a\0b"),
        Err(NameProblem::ForbiddenChar)
    );
    assert_eq!(
        validate_collection_name("system.users"),
        Err(NameProblem::SystemPrefix)
    );
    assert_eq!(validate_collection_name("systems"), Ok(()));
}

#[test]
fn collection_name_accepts_dots_spaces_unicode() {
    assert_eq!(validate_collection_name("my.coll ection ünï"), Ok(()));
}

#[test]
fn database_name_limit_is_63_bytes_collection_namespace_255() {
    assert_eq!(validate_database_name(&"a".repeat(63)), Ok(()));
    assert_eq!(
        validate_database_name(&"a".repeat(64)),
        Err(NameProblem::TooLong)
    );
    // 32 two-byte chars are 64 bytes: the limit counts bytes, not chars.
    assert_eq!(
        validate_database_name(&"é".repeat(32)),
        Err(NameProblem::TooLong)
    );
    // "db." is 3 bytes, so 252 collection bytes fit and 253 do not.
    let fits = Namespace::new("db", "c".repeat(252));
    let too_long = Namespace::new("db", "c".repeat(253));
    assert_eq!(validate_namespace(&fits), Ok(()));
    assert_eq!(validate_namespace(&too_long), Err(NameProblem::TooLong));
}

#[test]
fn excluded_names_cover_admin_local_config_and_system_collections() {
    for db in ["admin", "local", "config"] {
        assert!(is_excluded_database(db));
    }
    assert!(!is_excluded_database("shop"));
    assert!(!is_excluded_database("Admin"));
    assert!(is_excluded_collection("system.views"));
    assert!(!is_excluded_collection("systems"));
    assert!(!is_excluded_collection("orders"));
}

#[test]
fn same_namespace_is_exact_match() {
    let a = Namespace::new("db", "c");
    assert!(is_same_namespace(&a, &Namespace::new("db", "c")));
    assert!(!is_same_namespace(&a, &Namespace::new("db", "C")));
    assert!(!is_same_namespace(&a, &Namespace::new("db2", "c")));
}
