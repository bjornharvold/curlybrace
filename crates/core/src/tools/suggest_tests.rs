use super::*;
use std::os::unix::fs::PermissionsExt;

fn put(dir: &Path, name: &str, mode: u32) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    path
}

#[test]
fn only_existing_runnable_files_are_suggested() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let c = tempfile::tempdir().unwrap();
    put(a.path(), "mongosh", 0o644);
    let wanted = put(b.path(), "mongosh", 0o755);
    let dirs = [a.path(), b.path(), c.path()];
    assert_eq!(suggestions_in(ToolKind::Mongosh, &dirs), vec![wanted]);
}

#[test]
fn suggestions_keep_the_order_of_the_folders() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let first = put(a.path(), "mongodump", 0o755);
    let second = put(b.path(), "mongodump", 0o755);
    assert_eq!(
        suggestions_in(ToolKind::Mongodump, &[a.path(), b.path()]),
        vec![first, second]
    );
}

#[test]
fn nothing_is_suggested_for_another_tool() {
    let a = tempfile::tempdir().unwrap();
    put(a.path(), "mongosh", 0o755);
    assert!(suggestions_in(ToolKind::Mongorestore, &[a.path()]).is_empty());
}

#[test]
fn suggesting_never_runs_the_file() {
    let a = tempfile::tempdir().unwrap();
    let marker = a.path().join("ran");
    let path = a.path().join("mongosh");
    std::fs::write(&path, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(suggestions_in(ToolKind::Mongosh, &[a.path()]), vec![path]);
    assert!(!marker.exists());
}

#[cfg(target_os = "macos")]
#[test]
fn the_common_folders_are_the_homebrew_and_usr_local_ones() {
    assert!(COMMON_DIRS.contains(&"/opt/homebrew/bin"));
    assert!(COMMON_DIRS.contains(&"/usr/local/bin"));
}

#[test]
fn the_search_path_gains_the_common_folders_it_lacks() {
    let base = std::ffi::OsString::from("/usr/bin:/opt/homebrew/bin");
    let joined = search_path(Some(&base), &["/opt/homebrew/bin", "/usr/local/bin"]);
    let parts: Vec<_> = std::env::split_paths(&joined).collect();
    assert_eq!(
        parts,
        vec![
            PathBuf::from("/usr/bin"),
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ],
        "the existing order is kept and nothing is repeated"
    );
}

#[test]
fn without_a_path_the_common_folders_are_the_search_path() {
    let joined = search_path(None, &["/a", "/b"]);
    let parts: Vec<_> = std::env::split_paths(&joined).collect();
    assert_eq!(parts, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
}

#[test]
fn the_default_search_path_includes_the_common_folders() {
    let joined = default_search_path(None);
    for dir in COMMON_DIRS {
        assert!(std::env::split_paths(&joined).any(|p| p == Path::new(dir)));
    }
}
