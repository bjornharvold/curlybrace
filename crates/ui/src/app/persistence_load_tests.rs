//! What the load path repairs: only entries that fail `validate()`, and nothing it can leave be.
use super::persistence::{Recovery, load_connections, startup};
use curlybrace_core::connection::ConnectionConfig;
use std::path::{Path, PathBuf};

fn write(dir: &tempfile::TempDir, list: &[ConnectionConfig]) -> PathBuf {
    let path = dir.path().join("connections.json");
    std::fs::write(&path, serde_json::to_string(list).unwrap()).unwrap();
    path
}

fn ok_backup(path: &Path, _tag: &str) -> std::io::Result<PathBuf> {
    Ok(path.to_path_buf())
}

#[test]
fn validate_accepts_a_name_with_edge_spaces() {
    // The rule the load path relies on: such a name is valid, so it is simply left alone.
    assert!(
        ConnectionConfig::new(" copy ", "mongodb://h")
            .validate()
            .is_ok()
    );
}

#[test]
fn an_entry_that_already_validates_is_loaded_untouched_and_never_reported() {
    let dir = tempfile::tempdir().unwrap();
    let list = vec![
        ConnectionConfig::new("copy ", "mongodb://h"),
        ConnectionConfig::new(" lead", "mongodb://h"),
    ];
    let path = write(&dir, &list);
    let loaded = load_connections(&path).unwrap();
    assert_eq!(loaded.connections, list);
    assert_eq!((loaded.skipped, loaded.repaired), (0, 0));
    assert_eq!(startup(&path, &ok_backup).recovery, None);
}

#[test]
fn only_the_invalid_entries_of_a_mixed_file_are_adjusted() {
    let dir = tempfile::tempdir().unwrap();
    let mut folder = ConnectionConfig::new("f", "mongodb://h");
    folder.folder = Some(" work ".into());
    let list = vec![
        ConnectionConfig::new("keep ", "mongodb://h"),
        ConnectionConfig::new("a\nb", "mongodb://h"),
        folder,
    ];
    let loaded = load_connections(&write(&dir, &list)).unwrap();
    assert_eq!(loaded.connections[0], list[0]);
    assert_eq!(loaded.connections[1].name, "a b");
    assert_eq!(loaded.connections[2].folder.as_deref(), Some("work"));
    assert_eq!((loaded.skipped, loaded.repaired), (0, 2));
}

#[test]
fn a_uri_with_userinfo_and_no_host_text_validates_so_the_load_keeps_it_unchanged() {
    // `validate()` only needs a non-empty authority ("u@"), and the strict host check is for
    // imports. So the load path neither skips nor adjusts it.
    let dir = tempfile::tempdir().unwrap();
    let list = vec![ConnectionConfig::new("nohost", "mongodb://u@")];
    assert!(list[0].validate().is_ok());
    let path = write(&dir, &list);
    let loaded = load_connections(&path).unwrap();
    assert_eq!(loaded.connections, list);
    assert_eq!((loaded.skipped, loaded.repaired), (0, 0));
    assert_eq!(startup(&path, &ok_backup).recovery, None);
}

#[test]
fn skipped_and_adjusted_entries_make_one_combined_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let list = vec![
        ConnectionConfig::new("a\nb", "mongodb://h"),
        ConnectionConfig::new("bad", "http://nope"),
    ];
    let s = startup(&write(&dir, &list), &ok_backup);
    assert_eq!(
        s.recovery,
        Some(Recovery::SkippedAndRepaired {
            skipped: 1,
            repaired: 1,
            backed_up: true
        })
    );
}

fn with_folder(name: &str, folder: &str) -> ConnectionConfig {
    let mut c = ConnectionConfig::new(name, "mongodb://h");
    c.folder = Some(folder.into());
    c
}

#[test]
fn an_invalid_folder_is_repaired_without_touching_a_valid_name() {
    let dir = tempfile::tempdir().unwrap();
    let list = vec![with_folder("prod ", " work ")];
    assert!(
        ConnectionConfig::new("prod ", "mongodb://h")
            .validate()
            .is_ok()
    );
    let path = write(&dir, &list);
    let loaded = load_connections(&path).unwrap();
    assert_eq!(loaded.connections[0].name, "prod ");
    assert_eq!(loaded.connections[0].folder.as_deref(), Some("work"));
    assert_eq!((loaded.skipped, loaded.repaired), (0, 1));
    assert_eq!(
        startup(&path, &ok_backup).recovery,
        Some(Recovery::Repaired {
            count: 1,
            backed_up: true
        })
    );
}

#[test]
fn an_invalid_name_is_repaired_without_touching_a_valid_folder() {
    // A folder passes its own check; the name is the one that fails.
    let dir = tempfile::tempdir().unwrap();
    let list = vec![with_folder("a\nb", "work")];
    let loaded = load_connections(&write(&dir, &list)).unwrap();
    assert_eq!(loaded.connections[0].name, "a b");
    assert_eq!(loaded.connections[0].folder.as_deref(), Some("work"));
    assert_eq!(loaded.repaired, 1);
}

#[test]
fn an_entry_with_both_labels_invalid_gets_both_repaired() {
    let dir = tempfile::tempdir().unwrap();
    let loaded = load_connections(&write(&dir, &[with_folder("a\nb", " work ")])).unwrap();
    assert_eq!(loaded.connections[0].name, "a b");
    assert_eq!(loaded.connections[0].folder.as_deref(), Some("work"));
    assert_eq!(loaded.repaired, 1);
}

#[test]
fn an_entry_with_a_malformed_production_flag_loads_as_production_instead_of_being_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let plain = ConnectionConfig::new("plain", "mongodb://h");
    let mut damaged =
        serde_json::to_value(ConnectionConfig::new("damaged", "mongodb://h2")).unwrap();
    damaged["production"] = serde_json::json!("yes");
    let list = serde_json::json!([serde_json::to_value(&plain).unwrap(), damaged]);
    let path = dir.path().join("connections.json");
    std::fs::write(&path, list.to_string()).unwrap();
    let loaded = load_connections(&path).unwrap();
    assert_eq!(loaded.skipped, 0, "nothing is dropped");
    assert_eq!(loaded.connections.len(), 2);
    assert!(!loaded.connections[0].production);
    assert!(loaded.connections[1].production, "fail safe: production");
}
