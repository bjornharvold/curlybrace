use super::*;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn write(dir: &Path, name: &str, mode: u32) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, "#!/bin/sh\necho 1.0.0\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
    path
}

fn path_var(dirs: &[&Path]) -> OsString {
    std::env::join_paths(dirs).unwrap()
}

#[test]
fn a_chosen_executable_must_be_an_absolute_path() {
    let choice = ToolChoice::Executable(PathBuf::from("bin/mongosh"));
    assert_eq!(
        resolve(ToolKind::Mongosh, &choice, None),
        Err(ToolError::NotAbsolute)
    );
}

#[test]
fn a_missing_file_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let choice = ToolChoice::Executable(dir.path().join("nope"));
    assert_eq!(
        resolve(ToolKind::Mongosh, &choice, None),
        Err(ToolError::Missing)
    );
}

#[test]
fn a_directory_is_not_a_tool() {
    let dir = tempfile::tempdir().unwrap();
    let choice = ToolChoice::Executable(dir.path().to_path_buf());
    assert_eq!(
        resolve(ToolKind::Mongosh, &choice, None),
        Err(ToolError::NotAFile)
    );
}

#[test]
fn a_file_without_the_execute_bit_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let choice = ToolChoice::Executable(write(dir.path(), "mongosh", 0o644));
    assert_eq!(
        resolve(ToolKind::Mongosh, &choice, None),
        Err(ToolError::NotExecutable)
    );
}

#[test]
fn an_executable_file_resolves_to_itself() {
    let dir = tempfile::tempdir().unwrap();
    let exe = write(dir.path(), "any-name", 0o755);
    let choice = ToolChoice::Executable(exe.clone());
    assert_eq!(resolve(ToolKind::Mongosh, &choice, None), Ok(exe));
}

#[test]
fn the_path_search_takes_the_first_executable_match() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    write(first.path(), "mongodump", 0o644);
    let wanted = write(second.path(), "mongodump", 0o755);
    let var = path_var(&[first.path(), second.path()]);
    assert_eq!(
        resolve(ToolKind::Mongodump, &ToolChoice::Path, Some(&var)),
        Ok(wanted)
    );
}

#[test]
fn the_path_search_ignores_relative_entries() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "mongosh", 0o755);
    let var = OsString::from(".");
    assert_eq!(
        resolve(ToolKind::Mongosh, &ToolChoice::Path, Some(&var)),
        Err(ToolError::NotOnPath)
    );
}

#[test]
fn nothing_on_the_path_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let var = path_var(&[dir.path()]);
    assert_eq!(
        resolve(ToolKind::Mongorestore, &ToolChoice::Path, Some(&var)),
        Err(ToolError::NotOnPath)
    );
    assert_eq!(
        resolve(ToolKind::Mongorestore, &ToolChoice::Path, None),
        Err(ToolError::NotOnPath)
    );
}

#[test]
fn every_error_has_a_translation_key() {
    for error in [
        ToolError::NotAbsolute,
        ToolError::Missing,
        ToolError::NotAFile,
        ToolError::NotExecutable,
        ToolError::NotOnPath,
        ToolError::CouldNotRun,
        ToolError::TimedOut,
        ToolError::NoVersion,
    ] {
        assert!(error.i18n_key().starts_with("tools.error_"), "{error:?}");
    }
}
