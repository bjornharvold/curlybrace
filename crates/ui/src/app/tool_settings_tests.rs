//! The tool paths file. Every file lives in a tempdir: nothing here may touch the real config
//! directory.
use super::persistence::{PersistError, Recovery, back_up};
use super::tool_settings::{ToolsSaveError, default_tools_path, load, save, saved};
use curlybrace_core::tools::{ToolChoice, ToolKind, ToolSettings};
use std::path::Path;
use uuid::Uuid;

fn sample() -> ToolSettings {
    ToolSettings::default()
        .with_global(
            ToolKind::Mongosh,
            ToolChoice::Executable("/opt/mongosh".into()),
        )
        .with_override(Uuid::new_v4(), ToolKind::Mongodump, ToolChoice::Path)
}

#[test]
fn settings_round_trip_through_a_private_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub").join("tools.json");
    let before = sample();
    save(&path, &before).unwrap();
    let started = load(&path, &back_up);
    assert_eq!(started.settings, before);
    assert_eq!(started.recovery, None);
    assert!(started.guard.check().is_ok());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "private, like connections.json");
    }
}

#[test]
fn a_missing_file_is_the_defaults_with_nothing_to_report() {
    let dir = tempfile::tempdir().unwrap();
    let started = load(&dir.path().join("none.json"), &back_up);
    assert_eq!(started.settings, ToolSettings::default());
    assert_eq!(started.recovery, None);
    assert!(started.guard.check().is_ok());
}

#[test]
fn a_corrupt_file_is_backed_up_and_the_defaults_apply() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tools.json");
    std::fs::write(&path, "{ not json").unwrap();
    let started = load(&path, &back_up);
    assert_eq!(started.settings, ToolSettings::default());
    assert_eq!(started.recovery, Some(Recovery::Tools { backed_up: true }));
    assert!(started.guard.check().is_ok());
    let backups = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().contains("tools.json.corrupt-"))
        .count();
    assert_eq!(backups, 1);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
}

#[test]
fn a_failed_backup_blocks_saving_so_the_original_survives() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tools.json");
    std::fs::write(&path, "[]").unwrap();
    let failing = |_: &Path, _: &str| Err(std::io::ErrorKind::PermissionDenied.into());
    let started = load(&path, &failing);
    assert_eq!(started.recovery, Some(Recovery::Tools { backed_up: false }));
    assert_eq!(started.guard.check(), Err(PersistError::Blocked));
    assert_eq!(
        saved(&path, &started.guard, &sample()),
        Err(ToolsSaveError::Blocked)
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "[]");
}

#[test]
fn a_file_of_the_wrong_shape_is_unusable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tools.json");
    std::fs::write(&path, r#"{"global": {"mongosh": 5}}"#).unwrap();
    assert!(load(&path, &back_up).recovery.is_some());
}

#[test]
fn the_tool_paths_sit_beside_the_connection_list() {
    if let Ok(path) = default_tools_path() {
        assert_eq!(path.file_name().unwrap(), "tools.json");
        assert_eq!(path.parent().unwrap().file_name().unwrap(), "curlybrace");
    }
}

#[test]
fn the_recovery_notice_has_its_own_message_keys() {
    assert_eq!(
        Recovery::Tools { backed_up: true }.i18n_key(),
        "persist.tools_backed_up"
    );
    assert_eq!(
        Recovery::Tools { backed_up: false }.i18n_key(),
        "persist.tools_backup_failed"
    );
}
