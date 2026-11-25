//! `overwrite_header` on its own.

use std::fs;
use std::io::Write;

use super::export_header::overwrite_header;

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
