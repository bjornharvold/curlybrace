//! Second review of the query history, part two: the backups notice, connection changes against
//! a newer or unreadable file, favourite size limits, and what a refused duplicate says.
use super::Op;
use super::second_tests::{app_with_connection, on_disk, star_op, text};
use super::tests::{browsing_app, file};
use crate::app::persistence::{PersistGuard, Recovery};
use crate::app::query_history_store::{self, HistorySaveError};
use crate::app::reducer::Input;
use crate::app::state::Banner;
use crate::app::tools_state::ToolsAction;
use crate::app::*;
use crate::views::connections::ConnectionAction;
use curlybrace_core::query_history::QueryHistory;
use curlybrace_core::runtime::EMBEDDED_SESSION;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// A "backup" `remove_file` cannot remove: a folder with something in it.
fn stubborn_backup(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let bad = dir.path().join("query_history.json.corrupt-1");
    std::fs::create_dir(&bad).unwrap();
    std::fs::write(bad.join("x"), "SECRET").unwrap();
    bad
}

#[test]
fn a_backup_notice_outlives_the_write_that_ends_an_earlier_failure() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    // A write failure is announced (failing), then dismissed.
    a.state_mut().query_history.guard = PersistGuard::blocked();
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(1), "one"))
        .unwrap();
    a.dispatch(Input::ForgetCancelled);
    a.dispatch(Input::DismissBanner);
    // Switching off with a backup that cannot be deleted: its notice takes the free slot.
    let bad = stubborn_backup(&dir);
    a.dispatch(Input::Tools(ToolsAction::SetRememberQueries(false)));
    assert_eq!(
        a.state().banner,
        Some(Banner::QueryHistoryPersist(
            HistorySaveError::BackupsNotDeleted
        ))
    );
    // The earlier failure ends: a write works while the deletion is still outstanding.
    a.state_mut().query_history.guard = PersistGuard::default();
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(2), "two"))
        .unwrap();
    a.dispatch(Input::ForgetCancelled);
    assert_eq!(
        on_disk(&dir).favourites(EMBEDDED_SESSION).len(),
        2,
        "the write worked"
    );
    assert!(bad.exists());
    assert_eq!(
        a.state().banner,
        Some(Banner::QueryHistoryPersist(
            HistorySaveError::BackupsNotDeleted
        )),
        "the notice about the backups is not the failure that ended"
    );
}

#[test]
fn the_backup_notice_goes_when_the_deletion_works() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    let bad = stubborn_backup(&dir);
    a.dispatch(Input::Tools(ToolsAction::SetRememberQueries(false)));
    assert!(a.state().banner.is_some());
    std::fs::remove_dir_all(&bad).unwrap();
    std::fs::write(&bad, "SECRET").unwrap();
    a.state_mut()
        .query_history_tick(Instant::now() + Duration::from_secs(31));
    assert!(!bad.exists());
    assert_eq!(a.state().banner, None, "its notice is taken down");
}

#[test]
fn switching_on_cancels_a_wanted_deletion_and_its_notice() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    let bad = stubborn_backup(&dir);
    a.dispatch(Input::Tools(ToolsAction::SetRememberQueries(false)));
    assert!(a.state().banner.is_some());
    a.dispatch(Input::Tools(ToolsAction::SetRememberQueries(true)));
    assert_eq!(a.state().banner, None);
    std::fs::remove_dir_all(&bad).unwrap();
    std::fs::write(&bad, "KEEP").unwrap();
    let wait = a
        .state_mut()
        .query_history_tick(Instant::now() + Duration::from_secs(31));
    assert_eq!(wait, None, "nothing wanted any more");
    assert!(bad.exists(), "and nothing deleted");
}

#[test]
fn a_window_that_follows_a_file_saying_on_drops_its_backup_notice_and_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    let (mut b, _kb) = browsing_app(&dir);
    let bad = stubborn_backup(&dir);
    a.dispatch(Input::Tools(ToolsAction::SetRememberQueries(false)));
    assert!(a.state().banner.is_some(), "the notice is up");
    // The other window switches recording on again.
    b.dispatch(Input::Tools(ToolsAction::SetRememberQueries(true)));
    assert!(on_disk(&dir).remembering());
    // The first window follows that file with its next change.
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(1), "one"))
        .unwrap();
    a.dispatch(Input::ForgetCancelled);
    assert_eq!(a.state().banner, None, "the notice would be false");
    std::fs::remove_dir_all(&bad).unwrap();
    std::fs::write(&bad, "KEEP").unwrap();
    let wait = a
        .state_mut()
        .query_history_tick(Instant::now() + Duration::from_secs(31));
    assert_eq!(wait, None);
    assert!(bad.exists(), "and nothing is deleted any more");
}

#[test]
fn switching_on_is_made_again_on_top_of_another_windows_file_even_with_changes_waiting() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(file(&dir), "{ nope").unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    let (mut b, _kb) = browsing_app(&dir);
    // The other window (paused too) writes the file with the switch off.
    b.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(1), "theirs"))
        .unwrap();
    b.dispatch(Input::ForgetCancelled);
    assert!(!on_disk(&dir).remembering());
    // This window has a change waiting (a connection removed), then the person switches on.
    a.state_mut()
        .query_history_apply_queued(Op::ForgetConnection(Uuid::from_u128(9)));
    a.dispatch(Input::Tools(ToolsAction::SetRememberQueries(true)));
    assert!(a.state().query_history.recording());
    assert!(on_disk(&dir).remembering(), "the choice is in the file");
}

#[test]
fn a_connection_change_with_a_newer_version_file_is_not_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let newer = "{\"version\": 99, \"history\": [], \"favourites\": []}";
    std::fs::write(file(&dir), newer).unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let (mut a, _ka) = app_with_connection(&dir, &cfg);
    a.state_mut().banner = None;
    a.dispatch_connection(ConnectionAction::Duplicate(cfg.id));
    assert_eq!(a.state().banner, None, "no red banner for a duplicate");
    a.dispatch_connection(ConnectionAction::Delete(cfg.id));
    assert_eq!(a.state().banner, None, "nor for a delete");
    assert_eq!(std::fs::read_to_string(file(&dir)).unwrap(), newer);
}

#[test]
fn a_newer_file_replaced_by_a_readable_one_is_scrubbed_on_a_connection_delete() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        file(&dir),
        "{\"version\": 99, \"history\": [], \"favourites\": []}",
    )
    .unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let id = cfg.id;
    let (mut a, _ka) = app_with_connection(&dir, &cfg);
    // The newer file goes away and another window writes a readable one with X's queries.
    let theirs = QueryHistory::default()
        .recorded(id, "app", "people", text("{\"ssn\":\"SECRET456\"}"), 1)
        .unwrap();
    query_history_store::save(&file(&dir), &theirs).unwrap();
    a.dispatch_connection(ConnectionAction::Delete(id));
    let raw = std::fs::read_to_string(file(&dir)).unwrap();
    assert!(!raw.contains("SECRET456"), "left on disk: {raw}");
}

fn big_favourites_store(ids: &[(Uuid, usize)]) -> QueryHistory {
    ids.iter().fold(QueryHistory::default(), |s, (id, count)| {
        (0..*count).fold(s, |s, i| {
            s.starred(
                *id,
                "app",
                "people",
                &format!("f{i}"),
                text(&format!("{i}{}", "x".repeat(250 * 1024))),
                1,
            )
            .unwrap()
        })
    })
}

#[test]
fn a_duplicate_whose_favourites_cannot_all_be_kept_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let (mut a, _ka) = app_with_connection(&dir, &cfg);
    a.state_mut().query_history.store =
        big_favourites_store(&[(cfg.id, 3), (Uuid::from_u128(77), 15)]);
    a.state_mut().banner = None;
    a.dispatch_connection(ConnectionAction::Duplicate(cfg.id));
    assert_eq!(
        a.state().banner,
        Some(Banner::QueryHistoryPersist(HistorySaveError::CopyRefused))
    );
    let copy = a
        .state()
        .conn
        .connections
        .iter()
        .find(|c| c.id != cfg.id)
        .expect("the copy")
        .id;
    assert!(a.state().query_history.store.favourites(copy).is_empty());
}

#[test]
fn a_write_failure_after_the_pause_ended_is_announced() {
    // A damaged file at launch, and saving blocked (as when its backup could not be made): the
    // person switches on, which ends the pause and its notice, and the write then fails.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(file(&dir), "{ nope").unwrap();
    let (a, _ka, _ca) = crate::app::pipelines_wiring_tests::app_in(&dir, Vec::new());
    let mut a = a.with_query_history_path(Ok(file(&dir)));
    a.state_mut().query_history.guard = PersistGuard::blocked();
    assert!(matches!(
        a.state().banner,
        Some(Banner::Recovery(Recovery::QueryHistory { .. }))
    ));
    a.dispatch(Input::Tools(ToolsAction::SetRememberQueries(true)));
    assert!(a.state().query_history.recording(), "the pause ended");
    assert_eq!(
        a.state().banner,
        Some(Banner::QueryHistoryPersist(HistorySaveError::Blocked)),
        "the write failure is shown, not the old notice"
    );
}

#[test]
fn a_duplicate_refused_on_the_replay_is_a_dropped_change_not_a_silent_loss() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let other = Uuid::from_u128(77);
    let (mut a, _ka) = app_with_connection(&dir, &cfg);
    let (mut b, _kb) = app_with_connection(&dir, &cfg);
    // The other window wrote the file with the favourites of both connections.
    for (connection, count) in [(cfg.id, 3), (other, 15)] {
        for i in 0..count {
            b.state_mut()
                .query_history_apply(Op::Star {
                    id: Uuid::new_v4(),
                    connection,
                    db: "app".into(),
                    collection: "people".into(),
                    name: format!("f{i}"),
                    text: Box::new(text(&format!("{i}{}", "x".repeat(250 * 1024)))),
                    now: 1,
                })
                .unwrap();
        }
    }
    b.dispatch(Input::ForgetCancelled);
    assert_eq!(on_disk(&dir).favourites(cfg.id).len(), 3);
    // This window knows only the first connection's three: the copy fits here.
    a.state_mut().query_history.store = big_favourites_store(&[(cfg.id, 3)]);
    a.state_mut().banner = None;
    a.dispatch_connection(ConnectionAction::Duplicate(cfg.id));
    assert_eq!(
        a.state().banner,
        Some(Banner::QueryHistoryPersist(HistorySaveError::ReplayDropped)),
        "refused on the file it was replayed on: said, not lost"
    );
    let copy = a
        .state()
        .conn
        .connections
        .iter()
        .find(|c| c.id != cfg.id)
        .expect("the copy")
        .id;
    assert!(on_disk(&dir).favourites(copy).is_empty());
}

#[test]
fn the_refused_duplicate_notice_survives_the_write_that_ends_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let (mut a, _ka) = app_with_connection(&dir, &cfg);
    // A failure is announced (failing), then dismissed and its cause goes.
    a.state_mut().query_history.guard = PersistGuard::blocked();
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(1), "one"))
        .unwrap();
    a.dispatch(Input::ForgetCancelled);
    a.dispatch(Input::DismissBanner);
    a.state_mut().query_history.guard = PersistGuard::default();
    // A refused duplicate puts its notice up; the next write works.
    a.state_mut().query_history.store =
        big_favourites_store(&[(cfg.id, 3), (Uuid::from_u128(77), 15)]);
    a.dispatch_connection(ConnectionAction::Duplicate(cfg.id));
    assert_eq!(
        a.state().banner,
        Some(Banner::QueryHistoryPersist(HistorySaveError::CopyRefused))
    );
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(2), "two"))
        .unwrap();
    a.dispatch(Input::ForgetCancelled);
    assert_eq!(
        a.state().banner,
        Some(Banner::QueryHistoryPersist(HistorySaveError::CopyRefused)),
        "the write that ended the failure left the notice alone"
    );
}

#[test]
fn the_backups_notice_names_both_kinds_of_backup_file() {
    let text = I18n::english().t("persist.query_history_backups_not_deleted");
    assert!(text.contains("query_history.json.corrupt-"), "{text}");
    assert!(text.contains("query_history.json.unreadable-"), "{text}");
}

#[test]
fn a_dismissed_backup_notice_is_not_offered_again() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    let _bad = stubborn_backup(&dir);
    a.dispatch(Input::Tools(ToolsAction::SetRememberQueries(false)));
    assert!(a.state().banner.is_some());
    a.dispatch(Input::DismissBanner);
    for n in 1..=3 {
        a.state_mut()
            .query_history_tick(Instant::now() + Duration::from_secs(31 * n));
        assert_eq!(a.state().banner, None, "tick {n}");
    }
}

#[test]
fn switching_on_leaves_an_unrelated_banner_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    let _bad = stubborn_backup(&dir);
    a.dispatch(Input::Tools(ToolsAction::SetRememberQueries(false)));
    a.dispatch(Input::DismissBanner);
    a.state_mut().banner = Some(Banner::NotConnected);
    a.dispatch(Input::Tools(ToolsAction::SetRememberQueries(true)));
    assert_eq!(a.state().banner, Some(Banner::NotConnected));
}

#[test]
fn a_backup_notice_that_found_the_slot_taken_is_offered_again_later() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    let _bad = stubborn_backup(&dir);
    a.state_mut().banner = Some(Banner::NotConnected);
    a.dispatch(Input::Tools(ToolsAction::SetRememberQueries(false)));
    assert_eq!(
        a.state().banner,
        Some(Banner::NotConnected),
        "the slot was taken"
    );
    a.dispatch(Input::DismissBanner);
    a.state_mut()
        .query_history_tick(Instant::now() + Duration::from_secs(31));
    assert_eq!(
        a.state().banner,
        Some(Banner::QueryHistoryPersist(
            HistorySaveError::BackupsNotDeleted
        ))
    );
}

#[test]
fn the_deletion_of_backups_is_tried_again_until_it_works() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    let bad = stubborn_backup(&dir);
    a.dispatch(Input::Tools(ToolsAction::SetRememberQueries(false)));
    assert!(bad.exists());
    let soon = a.state_mut().query_history_tick(Instant::now());
    assert!(soon.is_some(), "a retry is waiting");
    assert!(bad.exists(), "not yet");
    // The cause goes (the folder is replaced by a plain file, which can be deleted).
    std::fs::remove_dir_all(&bad).unwrap();
    std::fs::write(&bad, "SECRET").unwrap();
    a.state_mut()
        .query_history_tick(Instant::now() + Duration::from_secs(31));
    assert!(!bad.exists(), "deleted on the retry");
    assert_eq!(
        a.state_mut()
            .query_history_tick(Instant::now() + Duration::from_secs(31)),
        None
    );
}

fn newer_body(padding: usize) -> String {
    format!(
        "{{\"version\": 99, \"history\": [], \"favourites\": [], \"pad\": \"{}\"}}",
        "x".repeat(padding)
    )
}

#[test]
fn a_newer_file_of_any_size_is_left_alone_by_connection_changes() {
    let dir = tempfile::tempdir().unwrap();
    // Bigger than a file of ours may be: `load` and `saved` still call it newer.
    let body = newer_body(6 * 1024 * 1024);
    std::fs::write(file(&dir), &body).unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let (mut a, _ka) = app_with_connection(&dir, &cfg);
    a.state_mut().banner = None;
    a.dispatch_connection(ConnectionAction::Duplicate(cfg.id));
    assert_eq!(a.state().banner, None, "no banner for a duplicate");
    a.dispatch_connection(ConnectionAction::Delete(cfg.id));
    assert_eq!(a.state().banner, None, "nor for a delete");
    assert_eq!(
        std::fs::read_to_string(file(&dir)).unwrap(),
        body,
        "untouched"
    );
}

#[test]
fn a_favourite_change_never_asks_whether_the_file_is_newer() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(file(&dir), newer_body(0)).unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let (mut a, _ka) = app_with_connection(&dir, &cfg);
    let before = query_history_store::NEWER_CHECKS.with(|n| n.get());
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(1), "one"))
        .unwrap();
    a.dispatch(Input::ForgetCancelled);
    assert_eq!(query_history_store::NEWER_CHECKS.with(|n| n.get()), before);
}

#[test]
fn a_connection_change_asks_once_while_the_file_is_newer_and_never_after_a_readable_one() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(file(&dir), newer_body(0)).unwrap();
    let (c1, c2, c3) = (
        ConnectionConfig::new("one", "mongodb://localhost:27017"),
        ConnectionConfig::new("two", "mongodb://localhost:27018"),
        ConnectionConfig::new("three", "mongodb://localhost:27019"),
    );
    let (app, events, commands) =
        crate::app::pipelines_wiring_tests::app_in(&dir, vec![c1.clone(), c2.clone(), c3.clone()]);
    let mut a = app.with_query_history_path(Ok(file(&dir)));
    let _keep = (events, commands);
    let checks = || query_history_store::NEWER_CHECKS.with(|n| n.get());
    let start = checks();
    // An empty store and a connection-only change: the file is looked at once, still newer.
    a.dispatch_connection(ConnectionAction::Delete(c1.id));
    assert_eq!(checks(), start + 1, "asked once");
    // A readable file replaces it: asked again (no longer newer), and following it ends the flag.
    query_history_store::save(&file(&dir), &QueryHistory::default()).unwrap();
    a.dispatch_connection(ConnectionAction::Delete(c2.id));
    assert_eq!(checks(), start + 2);
    assert!(!a.state().query_history.newer_file);
    // Never asked again.
    a.dispatch_connection(ConnectionAction::Duplicate(c3.id));
    a.dispatch_connection(ConnectionAction::Delete(c3.id));
    assert_eq!(
        checks(),
        start + 2,
        "not asked after a readable file was followed"
    );
}

#[cfg(unix)]
#[test]
fn a_write_failure_after_following_another_windows_file_is_announced_with_the_pause_ended() {
    use std::os::unix::fs::PermissionsExt;
    /// Gives the folder its write permission back, whatever happens.
    struct Restore(std::path::PathBuf);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(file(&dir), "{ nope").unwrap();
    let (a, _ka, _ca) = crate::app::pipelines_wiring_tests::app_in(&dir, Vec::new());
    let mut a = a.with_query_history_path(Ok(file(&dir)));
    assert!(matches!(
        a.state().banner,
        Some(Banner::Recovery(Recovery::QueryHistory { .. }))
    ));
    let (mut b, _kb) = browsing_app(&dir);
    b.dispatch(Input::Tools(ToolsAction::SetRememberQueries(true)));
    let _restore = Restore(dir.path().to_path_buf());
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::write(dir.path().join("probe"), "x").is_ok() {
        // Running as a user that permissions do not bind (root): the failure after the follow
        // cannot be made here, and the test says so instead of passing for nothing.
        eprintln!("SKIPPED: this user can write to a read-only folder");
        return;
    }
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(1), "one"))
        .unwrap();
    a.dispatch(Input::ForgetCancelled);
    assert!(
        a.state().query_history.recording(),
        "following the file ended the pause"
    );
    assert!(
        matches!(
            a.state().banner,
            Some(Banner::QueryHistoryPersist(HistorySaveError::Persist(_)))
        ),
        "the write failure is shown, not the old notice: {:?}",
        a.state().banner
    );
}
