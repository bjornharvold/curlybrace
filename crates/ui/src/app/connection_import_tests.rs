//! Import in one pass: same names, ids-aside same entries and order as naming each entry against
//! the growing list, and fast enough for the largest accepted file.
use super::connection_io::{MAX_IMPORT_ENTRIES, export_bytes, prepare_import};
use curlybrace_core::connection::{ConnectionConfig, MAX_NAME_LEN, unique_name};

fn cfg(name: &str) -> ConnectionConfig {
    ConnectionConfig::new(name, "mongodb://h")
}

/// Naming each entry against the whole list so far: the quadratic way import used to work.
fn named_one_by_one(existing: &[ConnectionConfig], incoming: &[ConnectionConfig]) -> Vec<String> {
    let mut taken = existing.to_vec();
    let mut names = Vec::new();
    for c in incoming {
        let name = unique_name(&c.name, &taken);
        taken.push(cfg(&name));
        names.push(name);
    }
    names
}

#[test]
fn a_mixed_file_gets_the_names_naming_one_by_one_gives() {
    let long = "L".repeat(MAX_NAME_LEN);
    let spaced = format!("{} bc", "s".repeat(MAX_NAME_LEN - 3));
    let existing = vec![cfg("a"), cfg("a 3"), cfg("zeta")];
    let mut incoming = vec![
        cfg("a"),
        cfg("a"),
        cfg("b"),
        cfg("a"),
        cfg("a 2"),
        cfg(&long),
        cfg(&long),
        cfg(&spaced),
        cfg(&spaced),
        cfg("zeta"),
        cfg("b"),
    ];
    // Entries the strict import skips sit between the others.
    incoming.insert(3, cfg("bad\nname"));
    incoming.insert(6, ConnectionConfig::new("no-scheme", "localhost:27017"));
    let bytes = serde_json::to_vec(&incoming).unwrap();

    let imported = prepare_import(&bytes, &existing, 9).unwrap();

    let kept: Vec<ConnectionConfig> = incoming
        .iter()
        .filter(|c| c.validate().is_ok())
        .cloned()
        .collect();
    assert_eq!(imported.skipped, 2);
    let names: Vec<_> = imported
        .connections
        .iter()
        .map(|c| c.name.clone())
        .collect();
    assert_eq!(names, named_one_by_one(&existing, &kept));
    assert!(names.iter().all(|n| n.chars().count() <= MAX_NAME_LEN));
    assert!(names.iter().all(|n| !n.contains("  ")));
}

#[test]
fn the_largest_accepted_import_is_named_uniquely_and_in_order() {
    let existing = vec![cfg("host 2")];
    let incoming: Vec<ConnectionConfig> = (0..MAX_IMPORT_ENTRIES)
        .map(|i| match i % 4 {
            0 => cfg("host"),
            1 => cfg(&format!("db {}", i % 50)),
            2 => cfg(&"x".repeat(MAX_NAME_LEN)),
            _ => cfg(&format!("unique {i}")),
        })
        .collect();
    let bytes = export_bytes(&incoming).unwrap();

    let imported = prepare_import(&bytes, &existing, 9).unwrap();

    assert_eq!(imported.connections.len(), MAX_IMPORT_ENTRIES);
    assert_eq!(imported.skipped, 0);
    let names: Vec<_> = imported
        .connections
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    let distinct: std::collections::HashSet<_> = names.iter().copied().collect();
    assert_eq!(distinct.len(), MAX_IMPORT_ENTRIES, "every name is unique");
    assert!(!distinct.contains("host 2"), "the existing name is avoided");
    assert_eq!(&names[..3], ["host", "db 1", &"x".repeat(MAX_NAME_LEN)]);
    assert_eq!(names[4], "host 3");
    assert!(imported.connections.iter().all(|c| c.validate().is_ok()));
}
