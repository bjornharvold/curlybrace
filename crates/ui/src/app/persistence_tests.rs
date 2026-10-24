use super::*;
use curlybrace_core::connection::{SshAuth, SshConfig};

fn cfg(name: &str) -> ConnectionConfig {
    ConnectionConfig::new(name, "mongodb://localhost:27017")
}

fn path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("sub").join(FILE_NAME)
}

#[test]
fn round_trips_including_ssh_and_unicode() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = cfg("Prod ☃ Überdatenbank");
    c.ssh = Some(SshConfig {
        host: "bastion".into(),
        port: 2222,
        username: "me".into(),
        auth: SshAuth::Password,
    });
    let list = vec![cfg("a"), c];
    save_connections(&path(&dir), &list).unwrap();
    let loaded = load_connections(&path(&dir)).unwrap();
    assert_eq!(
        loaded,
        Loaded {
            connections: list,
            skipped: 0,
            repaired: 0
        }
    );
}

#[test]
fn missing_file_is_an_empty_list() {
    let dir = tempfile::tempdir().unwrap();
    let loaded = load_connections(&path(&dir)).unwrap();
    assert_eq!((loaded.connections.len(), loaded.skipped), (0, 0));
}

#[test]
fn corrupt_file_is_an_error_and_is_left_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(FILE_NAME);
    std::fs::write(&p, "{ not an array").unwrap();
    assert_eq!(load_connections(&p), Err(PersistError::Corrupt));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "{ not an array");
}

#[test]
fn invalid_and_undecodable_entries_are_skipped_and_counted() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(FILE_NAME);
    let good = serde_json::to_value(cfg("ok")).unwrap();
    let blank_name = serde_json::to_value(cfg("  ")).unwrap();
    let bad_uri = serde_json::to_value(ConnectionConfig::new("x", "http://nope")).unwrap();
    let garbage = serde_json::json!({"id": 5});
    let all = serde_json::json!([good, blank_name, bad_uri, garbage]);
    std::fs::write(&p, all.to_string()).unwrap();
    let loaded = load_connections(&p).unwrap();
    // The blank name is repaired (to its host), not skipped.
    assert_eq!(loaded.connections.len(), 2);
    assert_eq!(loaded.connections[0].name, "ok");
    assert_eq!(loaded.connections[1].name, "localhost:27017");
    assert_eq!((loaded.skipped, loaded.repaired), (2, 1));
}

fn file_of(uris: &[&str]) -> Vec<u8> {
    let entries: Vec<_> = uris
        .iter()
        .map(|u| serde_json::to_value(ConnectionConfig::new("n", u)).unwrap())
        .collect();
    serde_json::Value::Array(entries).to_string().into_bytes()
}

#[test]
fn import_rejects_a_password_leaked_into_the_host_but_load_keeps_it() {
    let bytes = file_of(&[
        "mongodb://u:pa/ss@h",
        "mongodb://h",
        "mongodb://h1:27017,h2:27018",
        "mongodb://[::1]:27017",
        "mongodb+srv://cluster.example.com",
        "mongodb://%2Ftmp%2Fmongodb-27017.sock",
    ]);
    let imported = parse_entries(read_entries(&bytes).unwrap(), false).unwrap();
    assert_eq!((imported.connections.len(), imported.skipped), (5, 1));

    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(FILE_NAME);
    std::fs::write(&p, &bytes).unwrap();
    let loaded = load_connections(&p).unwrap();
    assert_eq!((loaded.connections.len(), loaded.skipped), (6, 0));
}

/// A password that spilled into the path, query or a second `@` of the userinfo passes the
/// host list check on its own; import must still skip it, load must still keep it.
#[test]
fn import_rejects_an_at_sign_in_the_tail_or_userinfo_but_load_keeps_it() {
    let kept = [
        "mongodb://admin:2024/S3CR3T@db",
        "mongodb://u:12345?S3CR3T@h",
    ];
    let imported = parse_entries(read_entries(&file_of(&kept)).unwrap(), false).unwrap();
    assert_eq!((imported.connections.len(), imported.skipped), (0, 2));
    // A second `@` in the userinfo is rejected on import too (load may drop it already).
    // `u:pa@S3CR3T@h` is skipped by `validate()` (':' in userinfo), so `u@x@h` is what
    // actually exercises the userinfo rule.
    let userinfo = parse_entries(
        read_entries(&file_of(&["mongodb://u@x@h", "mongodb://h/db@x"])).unwrap(),
        false,
    )
    .unwrap();
    assert_eq!((userinfo.connections.len(), userinfo.skipped), (0, 2));

    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(FILE_NAME);
    std::fs::write(&p, file_of(&kept)).unwrap();
    let loaded = load_connections(&p).unwrap();
    assert_eq!((loaded.connections.len(), loaded.skipped), (2, 0));
}

#[test]
fn import_accepts_an_at_sign_in_a_key_value_query() {
    let uris = [
        "mongodb://h/?appName=bob@laptop",
        "mongodb://h/db?tlsCAFile=/certs@2/ca.pem",
    ];
    let imported = parse_entries(read_entries(&file_of(&uris)).unwrap(), false).unwrap();
    assert_eq!((imported.connections.len(), imported.skipped), (2, 0));
}

/// `mongodb://u@` has userinfo but no host text: import rejects it ("missing host"), the
/// saved-list load keeps accepting it so a saved connection is never dropped by the
/// strict host check.
#[test]
fn a_uri_with_userinfo_and_no_host_is_rejected_on_import_but_loaded() {
    let bytes = file_of(&["mongodb://u@"]);
    let imported = parse_entries(read_entries(&bytes).unwrap(), false).unwrap();
    assert_eq!((imported.connections.len(), imported.skipped), (0, 1));
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(FILE_NAME);
    std::fs::write(&p, &bytes).unwrap();
    let loaded = load_connections(&p).unwrap();
    assert_eq!((loaded.connections.len(), loaded.skipped), (1, 0));
}

/// A file as v0.2.0 could write it: any non-blank name, any folder text.
fn old_format_file() -> (String, Vec<ConnectionConfig>) {
    let mut emoji = cfg("fam\u{1F468}\u{200D}\u{1F469}");
    emoji.folder = Some("work".into());
    let long = cfg(&"n".repeat(250));
    let control_only = cfg("\u{7}");
    let mut folder_nl = cfg("plain");
    folder_nl.folder = Some("a\nb".into());
    let bad_uri = ConnectionConfig::new("x", "http://nope");
    let list = vec![emoji, long, control_only, folder_nl, bad_uri];
    (serde_json::to_string(&list).unwrap(), list)
}

#[test]
fn saved_labels_from_an_older_release_are_repaired_not_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(FILE_NAME);
    let (json, old) = old_format_file();
    std::fs::write(&p, json).unwrap();
    let loaded = load_connections(&p).unwrap();
    assert_eq!(
        (loaded.connections.len(), loaded.repaired, loaded.skipped),
        (4, 4, 1)
    );
    for (new, before) in loaded.connections.iter().zip(&old) {
        assert_eq!(new.id, before.id, "the keychain id is kept");
        assert!(new.validate().is_ok(), "{:?}", new.name);
    }
    assert_eq!(loaded.connections[0].name, "fam\u{1F468}\u{1F469}");
    assert_eq!(loaded.connections[1].name.chars().count(), 200);
    assert_eq!(loaded.connections[2].name, "localhost:27017");
    assert_eq!(loaded.connections[3].folder.as_deref(), Some("a b"));
}

#[test]
fn a_repaired_only_file_is_backed_up_and_reported_as_adjusted() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(FILE_NAME);
    let (_, old) = old_format_file();
    let only_fixable: Vec<_> = old.iter().take(4).collect();
    let original = serde_json::to_string(&only_fixable).unwrap();
    std::fs::write(&p, &original).unwrap();
    let s = startup(&p, &ok_backup);
    assert_eq!(s.connections.len(), 4);
    assert_eq!(
        s.recovery,
        Some(Recovery::Repaired {
            count: 4,
            backed_up: true
        })
    );
    assert!(s.guard.check().is_ok());
    let backups = siblings(&dir, "connections.json.repaired-");
    assert_eq!(std::fs::read_to_string(&backups[0]).unwrap(), original);
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        original,
        "not rewritten"
    );
    let blocked = startup(&p, &failing_backup);
    assert_eq!(blocked.guard.check(), Err(PersistError::Blocked));
    assert_eq!(
        blocked.recovery,
        Some(Recovery::Repaired {
            count: 4,
            backed_up: false
        })
    );
}

#[test]
fn skipped_and_adjusted_entries_are_both_in_the_recovery_report() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(FILE_NAME);
    std::fs::write(&p, old_format_file().0).unwrap();
    let s = startup(&p, &ok_backup);
    assert_eq!(s.connections.len(), 4);
    assert_eq!(
        s.recovery,
        Some(Recovery::SkippedAndRepaired {
            skipped: 1,
            repaired: 4,
            backed_up: true
        })
    );
}

#[test]
fn import_parsing_stays_strict_and_does_not_repair() {
    let (json, _) = old_format_file();
    let parsed = parse_entries(read_entries(json.as_bytes()).unwrap(), false).unwrap();
    assert_eq!(
        (parsed.connections.len(), parsed.skipped, parsed.repaired),
        (0, 5, 0)
    );
}

#[test]
fn write_leaves_no_temp_file_and_sets_private_mode() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    save_connections(&p, &[cfg("a")]).unwrap();
    save_connections(&p, &[cfg("b")]).unwrap();
    let names: Vec<_> = std::fs::read_dir(p.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec![FILE_NAME.to_string()]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&p).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn file_never_contains_a_password_key() {
    let dir = tempfile::tempdir().unwrap();
    let p = path(&dir);
    save_connections(&p, &[cfg("a")]).unwrap();
    let text = std::fs::read_to_string(&p).unwrap().to_lowercase();
    assert!(!text.contains("password") && !text.contains("passphrase"));
}

#[test]
fn failed_write_removes_the_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(FILE_NAME);
    std::fs::create_dir(&p).unwrap(); // renaming a file onto a directory fails
    assert!(matches!(
        save_connections(&p, &[cfg("a")]),
        Err(PersistError::Write(_))
    ));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

fn ok_backup(path: &Path, tag: &str) -> std::io::Result<PathBuf> {
    back_up(path, tag)
}

fn failing_backup(_: &Path, _: &str) -> std::io::Result<PathBuf> {
    Err(std::io::ErrorKind::PermissionDenied.into())
}

fn siblings(dir: &tempfile::TempDir, prefix: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(prefix))
        .collect();
    found.sort();
    found
}

#[test]
fn corrupt_with_a_working_backup_allows_saves_and_keeps_the_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(FILE_NAME);
    std::fs::write(&p, "{ corrupt").unwrap();
    let s = startup(&p, &ok_backup);
    assert_eq!(s.recovery, Some(Recovery::Corrupt { backed_up: true }));
    assert!(s.guard.check().is_ok() && s.connections.is_empty());
    let backups = siblings(&dir, "connections.json.corrupt-");
    assert_eq!(backups.len(), 1);
    assert_eq!(std::fs::read_to_string(&backups[0]).unwrap(), "{ corrupt");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "{ corrupt");
}

#[test]
fn a_failed_backup_blocks_saving() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(FILE_NAME);
    std::fs::write(&p, "{ corrupt").unwrap();
    let s = startup(&p, &failing_backup);
    assert_eq!(s.recovery, Some(Recovery::Corrupt { backed_up: false }));
    assert_eq!(s.guard.check(), Err(PersistError::Blocked));
}

#[test]
fn skipped_entries_are_backed_up_before_any_save_and_the_good_ones_load() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(FILE_NAME);
    let good = serde_json::to_value(cfg("ok")).unwrap();
    let original = serde_json::json!([good, {"id": 1, "from": "the future"}]).to_string();
    std::fs::write(&p, &original).unwrap();
    let s = startup(&p, &ok_backup);
    assert_eq!(s.connections.len(), 1);
    assert_eq!(
        s.recovery,
        Some(Recovery::Skipped {
            count: 1,
            backed_up: true
        })
    );
    let backups = siblings(&dir, "connections.json.skipped-");
    assert_eq!(std::fs::read_to_string(&backups[0]).unwrap(), original);
    let blocked = startup(&p, &failing_backup);
    assert_eq!(blocked.guard.check(), Err(PersistError::Blocked));
}

#[test]
fn repeated_corrupt_startups_never_overwrite_earlier_backups() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(FILE_NAME);
    for version in ["first", "second", "third"] {
        std::fs::write(&p, version).unwrap();
        assert!(startup(&p, &ok_backup).guard.check().is_ok());
    }
    let backups = siblings(&dir, "connections.json.corrupt-");
    let mut contents: Vec<String> = backups
        .iter()
        .map(|b| std::fs::read_to_string(b).unwrap())
        .collect();
    contents.sort();
    assert_eq!(contents, ["first", "second", "third"]);
}

#[test]
fn an_unreadable_file_that_cannot_be_copied_blocks_saving() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(FILE_NAME);
    std::fs::create_dir(&p).unwrap(); // reading a directory fails
    let s = startup(&p, &ok_backup);
    assert_eq!(s.recovery, Some(Recovery::Unreadable { backed_up: false }));
    assert_eq!(s.guard.check(), Err(PersistError::Blocked));
}

#[test]
fn healthy_and_missing_files_need_no_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(FILE_NAME);
    assert_eq!(startup(&p, &failing_backup).recovery, None);
    save_connections(&p, &[cfg("a")]).unwrap();
    let s = startup(&p, &failing_backup);
    assert_eq!((s.recovery, s.connections.len()), (None, 1));
    assert!(s.guard.check().is_ok());
}

#[test]
fn backups_are_private() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join(FILE_NAME);
    std::fs::write(&p, "x").unwrap();
    let b = back_up(&p, "corrupt").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(b).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn a_path_without_a_parent_is_an_invalid_path_not_a_missing_config_dir() {
    assert_eq!(
        atomic_write(Path::new("/"), b"x"),
        Err(PersistError::InvalidPath)
    );
}

#[test]
fn error_keys_are_distinct() {
    let mut keys = vec![
        PersistError::ConfigDirUnknown.i18n_key(),
        PersistError::Read(std::io::ErrorKind::Other).i18n_key(),
        PersistError::Corrupt.i18n_key(),
        PersistError::Write(std::io::ErrorKind::Other).i18n_key(),
        PersistError::InvalidPath.i18n_key(),
        PersistError::Blocked.i18n_key(),
    ];
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), 6);
}
