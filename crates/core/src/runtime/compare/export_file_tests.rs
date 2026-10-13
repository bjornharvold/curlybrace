//! The file helpers on their own: path validation, the guard and the header replacement.

use std::fs;
use std::io::Write;

use super::export_file::{FileGuard, replace_header, validate_path};

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
fn replace_header_shifts_the_body_across_chunks_and_truncates() {
    let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let mut content = b"HEADER-LONGER".to_vec();
    content.extend(&body);
    let (dir, mut file) = file_with(&content);
    let len = replace_header(&mut file, 13, b"HDR").unwrap();
    let mut expected = b"HDR".to_vec();
    expected.extend(&body);
    assert_eq!(len, expected.len() as u64);
    assert_eq!(contents(&dir), expected);
}

#[test]
fn replace_header_with_an_equal_length_header_only_overwrites_the_head() {
    let (dir, mut file) = file_with(b"AAAA body");
    assert_eq!(replace_header(&mut file, 4, b"BBBB").unwrap(), 9);
    assert_eq!(contents(&dir), b"BBBB body");
}

#[test]
fn replace_header_refuses_a_longer_header_without_touching_the_file() {
    let (dir, mut file) = file_with(b"AAAA body");
    assert!(replace_header(&mut file, 4, b"BBBBB").is_err());
    assert_eq!(contents(&dir), b"AAAA body");
}
