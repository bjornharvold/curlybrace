//! The file helpers on their own: path validation, the guard and the header replacement.

use std::fs;
use std::io::Write;

use super::export_file::{FileGuard, overwrite_header, validate_path};

#[test]
fn validate_path_accepts_a_new_file_and_an_existing_regular_file() {
    let dir = tempfile::tempdir().unwrap();
    assert!(validate_path(&dir.path().join("new.js")).is_ok());
    let existing = dir.path().join("old.js");
    fs::write(&existing, "x").unwrap();
    assert!(validate_path(&existing).is_ok());
}

#[test]
fn validate_path_refuses_relative_rootless_directory_and_missing_parent() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("f");
    fs::write(&file, "x").unwrap();
    for bad in [
        "x.js".into(),
        "".into(),
        "/".into(),
        dir.path().to_path_buf(),
        dir.path().join("nope").join("x.js"),
        file.join("child.js"),
    ] {
        assert!(validate_path(&bad).is_err(), "{bad:?}");
    }
}

#[test]
fn the_guard_removes_the_file_unless_committed() {
    let dir = tempfile::tempdir().unwrap();
    let dropped = dir.path().join("dropped");
    let (file, guard) = FileGuard::create(&dropped).unwrap();
    drop(file);
    drop(guard);
    assert!(!dropped.exists());
    let kept = dir.path().join("kept");
    let (file, guard) = FileGuard::create(&kept).unwrap();
    drop(file);
    guard.commit();
    assert!(kept.exists());
    let removed = dir.path().join("removed");
    let (file, guard) = FileGuard::create(&removed).unwrap();
    drop(file);
    assert!(guard.remove());
    assert!(!removed.exists());
}

#[test]
fn a_path_that_cannot_be_opened_is_never_removed_by_a_guard() {
    let dir = tempfile::tempdir().unwrap();
    let existing = dir.path().join("dir");
    fs::create_dir(&existing).unwrap();
    assert!(FileGuard::create(&existing).is_err());
    assert!(existing.exists(), "an existing directory was not touched");
}

fn file_with(content: &[u8]) -> (tempfile::TempDir, fs::File) {
    let dir = tempfile::tempdir().unwrap();
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(dir.path().join("f"))
        .unwrap();
    file.write_all(content).unwrap();
    (dir, file)
}

fn contents(dir: &tempfile::TempDir) -> Vec<u8> {
    fs::read(dir.path().join("f")).unwrap()
}

#[test]
fn overwrite_header_replaces_only_the_head_in_place() {
    let (dir, mut file) = file_with(b"AAAA body");
    overwrite_header(&mut file, 4, b"BBBB").unwrap();
    assert_eq!(contents(&dir), b"BBBB body");
}

#[test]
fn overwrite_header_refuses_another_length_without_touching_the_file() {
    for header in [&b"BBBBB"[..], b"BBB"] {
        let (dir, mut file) = file_with(b"AAAA body");
        assert!(overwrite_header(&mut file, 4, header).is_err());
        assert_eq!(contents(&dir), b"AAAA body");
    }
}

#[test]
fn a_trailing_separator_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let mut with_slash = dir.path().join("x.js").into_os_string();
    with_slash.push("/");
    assert!(validate_path(std::path::Path::new(&with_slash)).is_err());
}

#[cfg(unix)]
#[test]
fn create_never_follows_a_symlink_or_empties_a_hard_linked_file() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.txt");
    fs::write(&real, "KEEP").unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(FileGuard::create(&link).is_err());
    let hard = dir.path().join("hard");
    fs::hard_link(&real, &hard).unwrap();
    assert!(FileGuard::create(&hard).is_err());
    assert!(FileGuard::create(&real).is_err(), "it now has two links");
    assert_eq!(fs::read_to_string(&real).unwrap(), "KEEP");
    assert!(hard.exists() && link.exists(), "refusals remove nothing");
}

#[cfg(unix)]
#[test]
fn create_does_not_block_on_a_fifo() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("fifo");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    assert!(FileGuard::create(&fifo).is_err());
    assert!(fifo.exists());
}
