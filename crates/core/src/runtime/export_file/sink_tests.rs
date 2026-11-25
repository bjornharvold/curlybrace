//! The buffered sink on its own.

use std::fs;
use std::io::Write;

use super::test_hooks::fail_writes_after;
use super::{ExportSink, buffered, into_file};

#[test]
fn buffered_writes_reach_the_file_after_into_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out");
    let file = fs::File::create(&path).unwrap();
    let mut out = buffered(ExportSink::new(file, &path));
    out.write_all(b"hello").unwrap();
    into_file(out).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"hello");
}

#[test]
fn a_write_failure_injected_for_the_path_surfaces_from_into_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out");
    let file = fs::File::create(&path).unwrap();
    fail_writes_after(&path, 0);
    let mut out = buffered(ExportSink::new(file, &path));
    out.write_all(b"x").unwrap();
    assert!(into_file(out).is_err());
}
