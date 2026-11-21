//! The query history file. Every file lives in a tempdir: nothing here may touch the real config
//! directory.
use super::*;
use crate::app::persistence::back_up;
use curlybrace_core::query_history::{FindText, MAX_ENTRY_BYTES};
use uuid::Uuid;

fn text(filter: &str) -> FindText {
    FindText {
        filter: filter.into(),
        ..FindText::default()
    }
}

fn sample() -> QueryHistory {
    QueryHistory::default()
        .recorded(Uuid::from_u128(1), "shop", "orders", text("{\"a\":1}"), 5)
        .unwrap()
        .starred(Uuid::from_u128(1), "shop", "orders", "mine", text("{}"), 6)
        .unwrap()
}

fn path_in(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("query_history.json")
}

fn backups(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("query_history.json.corrupt-")
        })
        .count()
}

#[test]
fn the_store_round_trips_through_a_private_file_in_a_new_folder() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub").join("query_history.json");
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
        assert_eq!(
            mode & 0o777,
            0o600,
            "private: queries can hold personal values"
        );
    }
}

#[test]
fn a_missing_file_is_an_empty_store_that_remembers() {
    let dir = tempfile::tempdir().unwrap();
    let started = load(&path_in(&dir), &back_up);
    assert_eq!(started.settings, QueryHistory::default());
    assert!(started.settings.remembering());
    assert_eq!(started.recovery, None);
}

#[test]
fn a_corrupt_file_is_backed_up_and_an_empty_store_loads() {
    let dir = tempfile::tempdir().unwrap();
    let path = path_in(&dir);
    std::fs::write(&path, "{ not json").unwrap();
    let started = load(&path, &back_up);
    assert_eq!(started.settings, QueryHistory::default());
    assert_eq!(
        started.recovery,
        Some(Recovery::QueryHistory { backed_up: true })
    );
    assert!(started.guard.check().is_ok());
    assert_eq!(backups(dir.path()), 1);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
}

#[test]
fn a_failed_backup_blocks_saving_so_the_original_survives() {
    let dir = tempfile::tempdir().unwrap();
    let path = path_in(&dir);
    std::fs::write(&path, "[]").unwrap();
    let failing = |_: &Path, _: &str| Err(std::io::ErrorKind::PermissionDenied.into());
    let started = load(&path, &failing);
    assert_eq!(
        started.recovery,
        Some(Recovery::QueryHistory { backed_up: false })
    );
    assert_eq!(started.guard.check(), Err(PersistError::Blocked));
    assert_eq!(
        saved(&path, &started.guard, DiskStamp::default(), &sample()),
        Err(HistorySaveError::Blocked)
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "[]");
}

#[test]
fn a_newer_file_is_left_alone_and_saving_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let path = path_in(&dir);
    let text = r#"{"version": 99, "history": [], "favourites": [], "extra": true}"#;
    std::fs::write(&path, text).unwrap();
    let started = load(&path, &back_up);
    assert_eq!(started.recovery, Some(Recovery::QueryHistoryNewer));
    assert_eq!(started.settings, QueryHistory::default());
    assert_eq!(backups(dir.path()), 0, "not ours to copy or replace");
    assert_eq!(
        saved(&path, &started.guard, started.seen, &sample()),
        Err(HistorySaveError::NewerVersion)
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn a_file_that_breaks_a_limit_is_backed_up_and_not_trimmed() {
    let dir = tempfile::tempdir().unwrap();
    let path = path_in(&dir);
    let entry = r#"{"text":{"filter":"a"},"ran_unix":1}"#;
    let entries = vec![entry; 101].join(",");
    let body = format!(
        r#"{{"version":1,"history":[{{"connection":"{}","db":"d","collection":"c","entries":[{entries}]}}],"favourites":[]}}"#,
        Uuid::from_u128(1)
    );
    std::fs::write(&path, &body).unwrap();
    let started = load(&path, &back_up);
    assert_eq!(
        started.recovery,
        Some(Recovery::QueryHistory { backed_up: true })
    );
    assert_eq!(started.settings.history_len(), 0);
    assert_eq!(backups(dir.path()), 1);
}

#[test]
fn a_file_over_five_megabytes_is_treated_as_corrupt_and_just_under_loads() {
    let dir = tempfile::tempdir().unwrap();
    let path = path_in(&dir);
    let body = r#"{"version": 1}"#;
    std::fs::write(&path, format!("{body}{}", " ".repeat(MAX_FILE_BYTES))).unwrap();
    assert_eq!(
        load(&path, &back_up).recovery,
        Some(Recovery::QueryHistory { backed_up: true })
    );
    std::fs::write(
        &path,
        format!("{body}{}", " ".repeat(MAX_FILE_BYTES - body.len())),
    )
    .unwrap();
    assert_eq!(load(&path, &back_up).recovery, None);
}

#[test]
fn a_change_by_another_window_is_noticed_before_overwriting() {
    let dir = tempfile::tempdir().unwrap();
    let path = path_in(&dir);
    let guard = PersistGuard::default();
    let first = saved(&path, &guard, DiskStamp::default(), &sample()).unwrap();
    // Another window writes.
    save(&path, &QueryHistory::default().with_remembering(false)).unwrap();
    assert_eq!(
        saved(&path, &guard, first.stamp, &sample()),
        Err(HistorySaveError::ChangedOnDisk)
    );
    match reloaded(&path) {
        Reload::Usable(store, stamp) => {
            assert!(!store.remembering());
            assert!(saved(&path, &guard, stamp, &sample()).is_ok());
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_file_that_cannot_be_read_reloads_as_unusable() {
    let dir = tempfile::tempdir().unwrap();
    let path = path_in(&dir);
    std::fs::write(&path, "nonsense").unwrap();
    assert!(matches!(
        reloaded(&path),
        Reload::Unusable { tag: "corrupt", .. }
    ));
    assert!(matches!(
        reloaded(&dir.path().join("none.json")),
        Reload::Usable(..)
    ));
}

/// A store whose history alone would not fit in the file: each entry is near the entry limit.
fn bulky() -> QueryHistory {
    let mut store = sample();
    for ns in 0..30 {
        for i in 0..4 {
            let body = format!("{}{ns}-{i}", "x".repeat(MAX_ENTRY_BYTES - 16));
            store = store
                .recorded(
                    Uuid::from_u128(1),
                    "shop",
                    &format!("c{ns}"),
                    text(&body),
                    i,
                )
                .unwrap();
        }
    }
    store
}

#[test]
fn a_save_never_fails_for_size_it_drops_the_oldest_history_first() {
    let dir = tempfile::tempdir().unwrap();
    let path = path_in(&dir);
    let big = bulky();
    assert!(serde_json::to_vec_pretty(&big).unwrap().len() > MAX_FILE_BYTES);
    let written = saved(&path, &PersistGuard::default(), DiskStamp::default(), &big).unwrap();
    assert!(std::fs::metadata(&path).unwrap().len() as usize <= MAX_FILE_BYTES);
    assert!(written.store.history_len() < big.history_len());
    assert_eq!(written.store.favourites(Uuid::from_u128(1)).len(), 1);
    // The next launch reads it back whole.
    let started = load(&path, &back_up);
    assert_eq!(started.recovery, None);
    assert_eq!(started.settings, written.store);
}

#[test]
fn favourites_that_cannot_fit_are_refused_not_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = QueryHistory::default();
    for i in 0..30 {
        let body = "x".repeat(MAX_ENTRY_BYTES - 16);
        store = store
            .starred(
                Uuid::from_u128(1),
                "d",
                "c",
                &format!("n{i}"),
                text(&body),
                1,
            )
            .unwrap();
    }
    assert_eq!(
        saved(
            &path_in(&dir),
            &PersistGuard::default(),
            DiskStamp::default(),
            &store
        ),
        Err(HistorySaveError::TooLarge)
    );
    assert!(!path_in(&dir).exists());
}

#[test]
fn backup_copies_and_temp_leftovers_are_removed_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let path = path_in(&dir);
    for name in [
        "query_history.json.corrupt-1",
        "query_history.json.unreadable-2",
        ".query_history.json.tmp-3",
    ] {
        std::fs::write(dir.path().join(name), "x").unwrap();
    }
    for keep in [
        "connections.json",
        "query_history.json",
        "workspace.json.corrupt-1",
    ] {
        std::fs::write(dir.path().join(keep), "x").unwrap();
    }
    remove_backups(&path).unwrap();
    let mut left: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(
        left,
        [
            "connections.json",
            "query_history.json",
            "workspace.json.corrupt-1"
        ]
    );
}

#[test]
fn the_file_sits_beside_the_connection_list() {
    if let Ok(path) = default_query_history_path() {
        assert_eq!(path.file_name().unwrap(), "query_history.json");
        assert_eq!(path.parent().unwrap().file_name().unwrap(), "curlybrace");
    }
}

#[test]
fn every_refusal_and_recovery_has_a_message_key() {
    for error in [
        HistorySaveError::Blocked,
        HistorySaveError::TooLarge,
        HistorySaveError::NewerVersion,
        HistorySaveError::ChangedOnDisk,
        HistorySaveError::Persist(PersistError::ConfigDirUnknown),
        HistorySaveError::Persist(PersistError::Write(std::io::ErrorKind::Other)),
    ] {
        assert!(error.i18n_key().starts_with("persist.query_history_"));
    }
    assert_eq!(
        Recovery::QueryHistory { backed_up: true }.i18n_key(),
        "persist.query_history_backed_up"
    );
    assert_eq!(
        Recovery::QueryHistory { backed_up: false }.i18n_key(),
        "persist.query_history_backup_failed"
    );
    assert_eq!(
        Recovery::QueryHistoryNewer.i18n_key(),
        "persist.query_history_newer"
    );
}
