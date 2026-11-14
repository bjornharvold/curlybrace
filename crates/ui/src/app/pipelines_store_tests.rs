//! The saved pipelines file. Every file lives in a tempdir: nothing here may touch the real
//! config directory.
use super::persistence::{PersistError, PersistGuard, Recovery, back_up};
use super::pipelines_store::{
    DiskStamp, MAX_FILE_BYTES, PipelinesSaveError, default_pipelines_path, load, save, saved,
};
use curlybrace_core::saved_pipeline::{SavedPipelines, SavedStage};
use std::path::Path;
use uuid::Uuid;

fn sample() -> SavedPipelines {
    SavedPipelines::default()
        .saved(
            Uuid::new_v4(),
            "totals",
            Some("shop.orders".into()),
            vec![SavedStage {
                text: r#"{"$match": {}}"#.into(),
                enabled: true,
            }],
            7,
            false,
        )
        .unwrap()
}

#[test]
fn pipelines_round_trip_through_a_private_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub").join("pipelines.json");
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
        assert_eq!(mode & 0o777, 0o600, "private: stages can hold user data");
    }
}

#[test]
fn a_missing_file_is_an_empty_store_with_nothing_to_report() {
    let dir = tempfile::tempdir().unwrap();
    let started = load(&dir.path().join("none.json"), &back_up);
    assert_eq!(started.settings, SavedPipelines::default());
    assert_eq!(started.recovery, None);
    assert!(started.guard.check().is_ok());
}

#[test]
fn a_corrupt_file_is_backed_up_and_an_empty_store_loads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pipelines.json");
    std::fs::write(&path, "{ not json").unwrap();
    let started = load(&path, &back_up);
    assert_eq!(started.settings, SavedPipelines::default());
    assert_eq!(
        started.recovery,
        Some(Recovery::Pipelines { backed_up: true })
    );
    assert!(started.guard.check().is_ok());
    let backups: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().contains("pipelines.json.corrupt-"))
        .collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&backups[0]).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "the backup holds the same data");
    }
}

#[test]
fn a_failed_backup_blocks_saving_so_the_original_survives() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pipelines.json");
    std::fs::write(&path, "[]").unwrap();
    let failing = |_: &Path, _: &str| Err(std::io::ErrorKind::PermissionDenied.into());
    let started = load(&path, &failing);
    assert_eq!(
        started.recovery,
        Some(Recovery::Pipelines { backed_up: false })
    );
    assert_eq!(started.guard.check(), Err(PersistError::Blocked));
    assert_eq!(
        saved(&path, &started.guard, DiskStamp::default(), &sample()),
        Err(PipelinesSaveError::Blocked)
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "[]");
}

#[test]
fn a_file_of_the_wrong_shape_is_unusable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pipelines.json");
    for text in [
        r#"{"version": 1, "connections": {"not-a-uuid": []}}"#,
        r#"{"version": 1, "connections": []}"#,
    ] {
        std::fs::write(&path, text).unwrap();
        assert!(load(&path, &back_up).recovery.is_some(), "{text}");
        // The original stays where it was, and a copy was made.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
}

#[test]
fn a_file_over_five_megabytes_is_treated_as_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pipelines.json");
    // Valid JSON, but too big to trust.
    let padding = " ".repeat(MAX_FILE_BYTES);
    std::fs::write(
        &path,
        format!(r#"{{"version": 1, "connections": {{}}}}{padding}"#),
    )
    .unwrap();
    let started = load(&path, &back_up);
    assert_eq!(
        started.recovery,
        Some(Recovery::Pipelines { backed_up: true })
    );
    assert_eq!(started.settings, SavedPipelines::default());
}

#[test]
fn a_file_just_under_the_limit_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pipelines.json");
    let body = r#"{"version": 1, "connections": {}}"#;
    let padding = " ".repeat(MAX_FILE_BYTES - body.len());
    std::fs::write(&path, format!("{body}{padding}")).unwrap();
    assert_eq!(load(&path, &back_up).recovery, None);
}

#[test]
fn the_pipelines_sit_beside_the_connection_list() {
    if let Ok(path) = default_pipelines_path() {
        assert_eq!(path.file_name().unwrap(), "pipelines.json");
        assert_eq!(path.parent().unwrap().file_name().unwrap(), "curlybrace");
    }
}

#[test]
fn the_recovery_notice_has_its_own_message_keys() {
    assert_eq!(
        Recovery::Pipelines { backed_up: true }.i18n_key(),
        "aggregate.saved.recovered_backed_up"
    );
    assert_eq!(
        Recovery::Pipelines { backed_up: false }.i18n_key(),
        "aggregate.saved.recovered_backup_failed"
    );
}

#[test]
fn a_save_failure_says_which_message_applies() {
    assert_eq!(
        PipelinesSaveError::Persist(PersistError::ConfigDirUnknown).i18n_key(),
        "aggregate.saved.config_dir_unknown"
    );
    assert_eq!(
        PipelinesSaveError::Persist(PersistError::Write(std::io::ErrorKind::Other)).i18n_key(),
        "aggregate.saved.write_failed"
    );
    assert_eq!(
        PipelinesSaveError::Blocked.i18n_key(),
        "aggregate.saved.blocked"
    );
}

/// `count` pipelines of one connection, each with `bytes` of stage text.
fn heavy(count: usize, bytes: usize) -> SavedPipelines {
    let conn = Uuid::new_v4();
    (0..count).fold(SavedPipelines::default(), |store, i| {
        let stage = SavedStage {
            text: "x".repeat(bytes),
            enabled: true,
        };
        store
            .saved(conn, &format!("p{i}"), None, vec![stage], 1, false)
            .unwrap()
    })
}

#[test]
fn a_write_past_the_file_cap_is_refused_and_the_file_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pipelines.json");
    save(&path, &sample()).unwrap();
    let before = std::fs::read(&path).unwrap();
    // Six pipelines under the 1 MB each limit, together over the 5 MB the next launch accepts.
    let result = saved(
        &path,
        &PersistGuard::default(),
        DiskStamp::default(),
        &heavy(6, 900_000),
    );
    assert_eq!(result, Err(PipelinesSaveError::TooLarge));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        PipelinesSaveError::TooLarge.i18n_key(),
        "aggregate.saved.error.file_too_large"
    );
}

#[test]
fn a_file_the_app_wrote_under_the_cap_loads_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pipelines.json");
    let store = heavy(4, 900_000);
    saved(
        &path,
        &PersistGuard::default(),
        DiskStamp::default(),
        &store,
    )
    .unwrap();
    let started = load(&path, &back_up);
    assert_eq!(started.recovery, None);
    assert_eq!(started.settings, store);
}

const NEWER: &str = r#"{"version": 2, "connections": {}, "future": true}"#;

fn backups_of(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("pipelines.json.")
        })
        .count()
}

#[test]
fn a_file_of_a_newer_version_is_neither_backed_up_nor_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pipelines.json");
    std::fs::write(&path, NEWER).unwrap();
    let started = load(&path, &back_up);
    assert_eq!(started.settings, SavedPipelines::default());
    assert_eq!(started.recovery, Some(Recovery::PipelinesNewer));
    assert_eq!(started.guard.check(), Err(PersistError::Blocked));
    assert_eq!(backups_of(dir.path()), 0, "nothing was copied aside");
    let result = saved(&path, &started.guard, DiskStamp::default(), &sample());
    assert_eq!(result, Err(PipelinesSaveError::NewerVersion));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), NEWER);
    assert_eq!(
        Recovery::PipelinesNewer.i18n_key(),
        PipelinesSaveError::NewerVersion.i18n_key()
    );
}
