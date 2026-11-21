//! Second review of the query history: notices that must not use up the failure flags, retries
//! after a lasting failure, a paused history that must stay paused on disk, and switching on
//! after another window repaired the file.
use super::Op;
use super::tests::{browsing_app, dispatch_run, file};
use crate::app::persistence::PersistGuard;
use crate::app::query_history_store::{self, HistorySaveError};
use crate::app::reducer::Input;
use crate::app::state::Banner;
use crate::app::tools_state::ToolsAction;
use crate::app::*;
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

#[test]
fn switching_on_while_the_file_is_still_damaged_starts_fresh() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(file(&dir), "{ nope").unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    a.dispatch(Input::Tools(ToolsAction::SetRememberQueries(true)));
    dispatch_run(&mut a, "{\"mine\":1}");
    assert_eq!(on_disk(&dir).history_len(), 1);
}
