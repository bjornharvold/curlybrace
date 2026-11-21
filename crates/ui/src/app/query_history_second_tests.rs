//! Second review of the query history: notices that must not use up the failure flags, retries
//! after a lasting failure, a paused history that must stay paused on disk, and switching on
//! after another window repaired the file.
use super::Op;
use super::tests::{browsing_app, dispatch_run, file};
use crate::app::persistence::{PersistGuard, Recovery};
use crate::app::query_history_store::{self, HistorySaveError};
use crate::app::reducer::Input;
use crate::app::state::Banner;
use crate::app::tools_state::ToolsAction;
use crate::app::*;
use crate::views::connections::ConnectionAction;
use curlybrace_core::query_history::{FindText, QueryHistory};
use curlybrace_core::runtime::EMBEDDED_SESSION;
use std::time::{Duration, Instant};
use uuid::Uuid;

fn text(filter: &str) -> FindText {
    FindText {
        filter: filter.into(),
        ..FindText::default()
    }
}

fn star_op(id: Uuid, name: &str) -> Op {
    Op::Star {
        id,
        connection: EMBEDDED_SESSION,
        db: "app".into(),
        collection: "people".into(),
        name: name.into(),
        text: Box::new(text("{}")),
        now: 1,
    }
}

fn on_disk(dir: &tempfile::TempDir) -> QueryHistory {
    query_history_store::load(&file(dir), &persistence::back_up).settings
}

#[test]
fn a_replay_notice_does_not_silence_the_next_real_failure() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    let (mut b, _kb) = browsing_app(&dir);
    b.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(1), "same"))
        .unwrap();
    b.dispatch(Input::DismissBanner);
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(2), "Same"))
        .unwrap();
    a.state_mut().banner = None;
    a.dispatch(Input::DismissBanner);
    assert_eq!(
        a.state().banner,
        Some(Banner::QueryHistoryPersist(HistorySaveError::ReplayDropped))
    );
    a.dispatch(Input::DismissBanner);
    assert_eq!(a.state().banner, None);
    a.state_mut().query_history.guard = PersistGuard::blocked();
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(3), "other"))
        .unwrap();
    a.dispatch(Input::DismissBanner);
    assert_eq!(
        a.state().banner,
        Some(Banner::QueryHistoryPersist(HistorySaveError::Blocked)),
        "the failure after the notice is announced"
    );
}

#[test]
fn a_failed_write_is_tried_again_after_a_while_with_nothing_changed() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    a.state_mut().query_history.guard = PersistGuard::blocked();
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(3), "fav"))
        .unwrap();
    a.dispatch(Input::DismissBanner);
    a.state_mut().query_history.guard = PersistGuard::default();
    let soon = a.state_mut().query_history_tick(Instant::now());
    assert!(soon.is_some(), "a retry is waiting");
    a.dispatch(Input::DismissBanner);
    assert_eq!(
        on_disk(&dir).favourites(EMBEDDED_SESSION).len(),
        0,
        "not yet"
    );
    let later = Instant::now() + Duration::from_secs(31);
    a.save_workspace_if_changed(&eframe::egui::Context::default(), later);
    assert_eq!(on_disk(&dir).favourites(EMBEDDED_SESSION).len(), 1);
    assert_eq!(
        a.state_mut().query_history_tick(later),
        None,
        "nothing waits now"
    );
}

#[test]
fn a_find_is_a_reason_to_retry_even_with_remembering_off() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    a.dispatch(Input::Tools(ToolsAction::SetRememberQueries(false)));
    a.state_mut().query_history.guard = PersistGuard::blocked();
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(3), "fav"))
        .unwrap();
    a.dispatch(Input::DismissBanner);
    a.state_mut().query_history.guard = PersistGuard::default();
    for i in 0..3 {
        dispatch_run(&mut a, &format!("{{\"n\":{i}}}"));
    }
    let written = on_disk(&dir);
    assert_eq!(written.favourites(EMBEDDED_SESSION).len(), 1, "retried");
    assert_eq!(written.history_len(), 0, "and still nothing recorded");
}

#[test]
fn closing_the_app_tries_a_waiting_change_once_more() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    a.state_mut().query_history.guard = PersistGuard::blocked();
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(3), "fav"))
        .unwrap();
    a.dispatch(Input::DismissBanner);
    a.state_mut().query_history.guard = PersistGuard::default();
    a.save_workspace_now();
    assert_eq!(on_disk(&dir).favourites(EMBEDDED_SESSION).len(), 1);
}

#[test]
fn a_favourite_saved_while_paused_is_written_with_the_switch_off() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(file(&dir), "{ nope").unwrap();
    {
        let (mut a, _ka) = browsing_app(&dir);
        assert!(!a.state().query_history.recording());
        a.state_mut()
            .query_history_apply(star_op(Uuid::from_u128(3), "fav"))
            .unwrap();
        a.dispatch(Input::DismissBanner);
    }
    let written = std::fs::read_to_string(file(&dir)).unwrap();
    assert!(written.contains("\"remember\": false"), "{written}");
    let (b, _kb) = browsing_app(&dir);
    assert!(!b.state().query_history.recording(), "still no recording");
    assert_eq!(
        b.state()
            .query_history
            .store
            .favourites(EMBEDDED_SESSION)
            .len(),
        1
    );
}

#[test]
fn a_repeated_favourite_id_is_not_written_into_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    let (mut b, _kb) = browsing_app(&dir);
    let id = Uuid::from_u128(77);
    b.state_mut()
        .query_history_apply(star_op(id, "renamed"))
        .unwrap();
    b.dispatch(Input::DismissBanner);
    a.state_mut()
        .query_history_apply(star_op(id, "original"))
        .unwrap();
    a.dispatch(Input::DismissBanner);
    let started = query_history_store::load(&file(&dir), &persistence::back_up);
    assert!(started.recovery.is_none(), "{:?}", started.recovery);
}

#[test]
fn switching_on_after_another_window_repaired_the_file_keeps_what_it_holds() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(file(&dir), "{ nope").unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    assert!(!a.state().query_history.recording());
    // Another window starts fresh and records.
    let other = QueryHistory::default()
        .recorded(EMBEDDED_SESSION, "app", "people", text("{\"theirs\":1}"), 5)
        .unwrap();
    query_history_store::save(&file(&dir), &other).unwrap();
    a.dispatch(Input::Tools(ToolsAction::SetRememberQueries(true)));
    dispatch_run(&mut a, "{\"mine\":1}");
    let written = on_disk(&dir);
    assert_eq!(written.history_len(), 2, "theirs is kept, mine is added");
    assert!(written.remembering());
}

/// Two windows start on a damaged file (both paused); the second switches on and records two
/// finds; then the first, still paused, has a favourite change to write.
fn two_paused_windows_one_switched_on(
    dir: &tempfile::TempDir,
) -> (CurlybraceApp, super::tests::Keep) {
    std::fs::write(file(dir), "{ nope").unwrap();
    let (a, ka) = browsing_app(dir);
    let (mut b, _kb) = browsing_app(dir);
    b.dispatch(Input::Tools(ToolsAction::SetRememberQueries(true)));
    dispatch_run(&mut b, "{\"b\":1}");
    dispatch_run(&mut b, "{\"b\":2}");
    assert_eq!(on_disk(dir).history_len(), 2);
    (a, ka)
}

fn assert_b_survived(dir: &tempfile::TempDir) {
    let written = on_disk(dir);
    assert_eq!(
        written.history_len(),
        2,
        "the other window's history is kept"
    );
    assert!(written.remembering(), "and so is its switch");
    assert_eq!(
        written.favourites(EMBEDDED_SESSION).len(),
        1,
        "plus the favourite"
    );
}

#[test]
fn a_paused_window_never_erases_or_switches_off_what_another_window_wrote() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = two_paused_windows_one_switched_on(&dir);
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(3), "fav"))
        .unwrap();
    a.dispatch(Input::DismissBanner);
    assert_b_survived(&dir);
    assert!(
        a.state().query_history.recording(),
        "the switch is known now"
    );
}

#[test]
fn the_exit_flush_of_a_paused_window_does_not_either() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = two_paused_windows_one_switched_on(&dir);
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(3), "fav"))
        .unwrap();
    a.save_workspace_now();
    assert_b_survived(&dir);
}

#[test]
fn a_file_from_a_newer_version_is_not_retried_on_a_timer() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        file(&dir),
        "{\"version\": 99, \"history\": [], \"favourites\": []}",
    )
    .unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(3), "fav"))
        .unwrap();
    a.dispatch(Input::DismissBanner);
    let later = Instant::now() + Duration::from_secs(3600);
    assert_eq!(a.state_mut().query_history_tick(later), None);
}

#[test]
fn a_change_that_already_landed_is_made_again_without_a_notice() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    let (mut b, _kb) = browsing_app(&dir);
    let id = Uuid::from_u128(5);
    b.state_mut()
        .query_history_apply(star_op(id, "same"))
        .unwrap();
    b.dispatch(Input::DismissBanner);
    // The first window made the very same change (as if its write landed but said it failed).
    a.state_mut()
        .query_history_apply(star_op(id, "same"))
        .unwrap();
    a.state_mut().banner = None;
    a.dispatch(Input::DismissBanner);
    assert_eq!(a.state().banner, None, "no notice");
    assert_eq!(on_disk(&dir).favourites(EMBEDDED_SESSION).len(), 1);
}

fn app_with_connection(
    dir: &tempfile::TempDir,
    cfg: &ConnectionConfig,
) -> (CurlybraceApp, super::tests::Keep) {
    let (app, events, commands) =
        crate::app::pipelines_wiring_tests::app_in(dir, vec![cfg.clone()]);
    let app = app.with_query_history_path(Ok(file(dir)));
    (app, (events, commands))
}

fn record_op(connection: Uuid, filter: &str) -> Op {
    Op::Record {
        connection,
        db: "app".into(),
        collection: "people".into(),
        text: Box::new(text(filter)),
        now: 1,
    }
}

fn star_for(connection: Uuid, name: &str) -> Op {
    Op::Star {
        id: Uuid::new_v4(),
        connection,
        db: "app".into(),
        collection: "people".into(),
        name: name.into(),
        text: Box::new(text("{}")),
        now: 1,
    }
}

#[test]
fn a_connection_deleted_in_a_stale_window_is_forgotten_in_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let id = cfg.id;
    let (mut a, _ka) = app_with_connection(&dir, &cfg);
    let (mut b, _kb) = app_with_connection(&dir, &cfg);
    b.state_mut()
        .query_history_apply(record_op(id, "{\"q\":1}"))
        .unwrap();
    b.state_mut()
        .query_history_apply(star_for(id, "fav"))
        .unwrap();
    b.dispatch(Input::ForgetCancelled);
    assert_eq!(on_disk(&dir).history_len(), 1, "the other window recorded");
    a.dispatch_connection(ConnectionAction::Delete(id));
    let written = on_disk(&dir);
    assert_eq!(
        written.history_len(),
        0,
        "the deleted connection's queries are gone"
    );
    assert!(written.favourites(id).is_empty());
}

#[test]
fn a_connection_duplicated_in_a_stale_window_gets_the_favourites_in_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let id = cfg.id;
    let (mut a, _ka) = app_with_connection(&dir, &cfg);
    let (mut b, _kb) = app_with_connection(&dir, &cfg);
    b.state_mut()
        .query_history_apply(star_for(id, "fav"))
        .unwrap();
    b.dispatch(Input::ForgetCancelled);
    a.dispatch_connection(ConnectionAction::Duplicate(id));
    let copy = a
        .state()
        .conn
        .connections
        .iter()
        .find(|c| c.id != id)
        .expect("the copy")
        .id;
    assert_eq!(on_disk(&dir).favourites(copy).len(), 1);
}

#[test]
fn the_pause_notice_goes_when_another_windows_file_ends_the_pause() {
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
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(3), "fav"))
        .unwrap();
    a.dispatch(Input::ForgetCancelled);
    assert!(a.state().query_history.recording());
    assert_eq!(a.state().banner, None, "the stale notice is gone");
}

#[test]
fn a_pause_notice_waiting_behind_another_banner_goes_when_switching_on() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("pipelines.json"), "{ nope").unwrap();
    std::fs::write(file(&dir), "{ nope").unwrap();
    let (a, _ka, _ca) = crate::app::pipelines_wiring_tests::app_in(&dir, Vec::new());
    let mut a = a.with_query_history_path(Ok(file(&dir)));
    assert!(matches!(
        a.state().banner,
        Some(Banner::Recovery(Recovery::Pipelines { .. }))
    ));
    a.dispatch(Input::Tools(ToolsAction::SetRememberQueries(true)));
    a.dispatch(Input::DismissBanner);
    assert_eq!(a.state().banner, None, "the waiting notice was dropped");
}

fn big_favourite(i: u128) -> Op {
    Op::Star {
        id: Uuid::from_u128(1000 + i),
        connection: EMBEDDED_SESSION,
        db: "app".into(),
        collection: "people".into(),
        name: format!("big {i}"),
        text: Box::new(text(&"x".repeat(240 * 1024))),
        now: 1,
    }
}

#[test]
fn a_write_failure_after_the_pause_ended_is_announced_not_blocked_by_the_old_notice() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(file(&dir), "{ nope").unwrap();
    let (a, _ka, _ca) = crate::app::pipelines_wiring_tests::app_in(&dir, Vec::new());
    let mut a = a.with_query_history_path(Ok(file(&dir)));
    let (mut b, _kb) = browsing_app(&dir);
    b.dispatch(Input::Tools(ToolsAction::SetRememberQueries(true)));
    // Each window's favourites fit alone; together they do not fit in the file.
    for i in 0..11 {
        b.state_mut().query_history_apply(big_favourite(i)).unwrap();
    }
    b.dispatch(Input::ForgetCancelled);
    for i in 11..22 {
        a.state_mut().query_history_apply(big_favourite(i)).unwrap();
    }
    a.dispatch(Input::ForgetCancelled);
    assert!(a.state().query_history.recording(), "the pause ended");
    // Favourites that would not fit together are refused on the replay and said so: that
    // notice, not the old "nothing is recorded" one, holds the banner slot.
    assert_eq!(
        a.state().banner,
        Some(Banner::QueryHistoryPersist(HistorySaveError::ReplayDropped)),
        "the refusal is shown, not the stale notice"
    );
}

#[test]
fn a_newer_version_banner_goes_when_a_later_write_works() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        file(&dir),
        "{\"version\": 99, \"history\": [], \"favourites\": []}",
    )
    .unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(1), "one"))
        .unwrap();
    a.dispatch(Input::ForgetCancelled);
    assert_eq!(
        a.state().banner,
        Some(Banner::QueryHistoryPersist(HistorySaveError::NewerVersion))
    );
    // The file is replaced by a valid one; the next change is written.
    std::fs::write(
        file(&dir),
        "{\"version\": 1, \"history\": [], \"favourites\": []}\n",
    )
    .unwrap();
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(2), "two"))
        .unwrap();
    a.dispatch(Input::ForgetCancelled);
    assert_eq!(a.state().banner, None, "the stale failure is gone");
    assert_eq!(on_disk(&dir).favourites(EMBEDDED_SESSION).len(), 2);
}

#[test]
fn a_connection_change_that_changes_nothing_creates_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let (mut a, _ka) = app_with_connection(&dir, &cfg);
    a.dispatch_connection(ConnectionAction::Duplicate(cfg.id));
    assert!(
        !file(&dir).exists(),
        "a duplicate of one with no favourites"
    );
    a.dispatch_connection(ConnectionAction::Delete(cfg.id));
    assert!(
        !file(&dir).exists(),
        "a delete of one that never had a query"
    );
}

#[test]
fn a_paused_window_leaves_the_damaged_file_alone_on_an_unrelated_delete() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(file(&dir), "{ nope").unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let (mut a, _ka) = app_with_connection(&dir, &cfg);
    a.dispatch_connection(ConnectionAction::Delete(cfg.id));
    assert_eq!(std::fs::read_to_string(file(&dir)).unwrap(), "{ nope");
    // So the next launch says again that the file was damaged.
    let (b, _kb) = app_with_connection(&dir, &cfg);
    assert!(matches!(
        b.state().banner,
        Some(Banner::Recovery(Recovery::QueryHistory { .. }))
    ));
}

thread_local! {
    static READS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// A file signature that is different at every look.
fn changing_signature(_: &std::path::Path) -> super::Sig {
    READS.with(|reads| {
        let n = reads.get();
        reads.set(n + 1);
        Some((n, None))
    })
}

#[test]
fn a_failure_is_remembered_against_the_file_as_it_was_before_the_attempt() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    a.state_mut().query_history.signature_reader = Some(changing_signature);
    a.state_mut().query_history.guard = PersistGuard::blocked();
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(1), "one"))
        .unwrap();
    a.dispatch(Input::ForgetCancelled);
    let stall = a.state().query_history.stalled.clone().expect("it failed");
    assert_eq!(
        stall.file,
        Some((0, None)),
        "the first look, taken before the attempt, not a later one"
    );
}

#[test]
fn switching_on_ends_the_pause_notice_too() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(file(&dir), "{ nope").unwrap();
    let (a, _ka, _ca) = crate::app::pipelines_wiring_tests::app_in(&dir, Vec::new());
    let mut a = a.with_query_history_path(Ok(file(&dir)));
    a.dispatch(Input::Tools(ToolsAction::SetRememberQueries(true)));
    assert_eq!(a.state().banner, None);
}

#[test]
fn switching_on_while_the_file_is_still_damaged_starts_fresh() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(file(&dir), "{ nope").unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    a.dispatch(Input::Tools(ToolsAction::SetRememberQueries(true)));
    dispatch_run(&mut a, "{\"mine\":1}");
    assert_eq!(on_disk(&dir).history_len(), 1);
}

#[test]
fn deleting_the_only_connection_with_queries_scrubs_them_from_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let id = cfg.id;
    let (mut a, _ka) = app_with_connection(&dir, &cfg);
    a.state_mut()
        .query_history_apply(record_op(id, "{\"ssn\":\"SECRET123\"}"))
        .unwrap();
    a.state_mut()
        .query_history_apply(star_for(id, "fav"))
        .unwrap();
    a.dispatch(Input::ForgetCancelled);
    assert!(
        std::fs::read_to_string(file(&dir))
            .unwrap()
            .contains("SECRET123")
    );
    a.dispatch_connection(ConnectionAction::Delete(id));
    assert!(a.state().connection(id).is_none());
    let raw = std::fs::read_to_string(file(&dir)).unwrap();
    assert!(!raw.contains("SECRET123"), "left on disk: {raw}");
}

#[test]
fn history_hidden_in_a_file_with_the_switch_off_is_scrubbed_when_its_connection_is_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let id = cfg.id;
    let body = format!(
        "{{\"version\":1,\"remember\":false,\"favourites\":[],\"history\":[{{\"connection\":\"{id}\",\"db\":\"app\",\"collection\":\"people\",\"entries\":[{{\"text\":{{\"filter\":\"HIDDEN\"}},\"ran_unix\":1}}]}}]}}"
    );
    std::fs::write(file(&dir), body).unwrap();
    let (mut a, _ka) = app_with_connection(&dir, &cfg);
    assert!(a.state().banner.is_none());
    a.dispatch_connection(ConnectionAction::Delete(id));
    let raw = std::fs::read_to_string(file(&dir)).unwrap();
    assert!(!raw.contains("HIDDEN"), "left on disk: {raw}");
}

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
