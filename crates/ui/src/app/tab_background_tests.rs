//! The answers of a session that is NOT the one in front find their tab too (#159): the write in
//! flight of a background session lives in that session's parked view, not in the foreground.
use crate::app::reducer::{Input, update};
use crate::app::state::{AppState, ConnStatus, Pending};
use crate::app::tabs::TabId;
use crate::app::tabs::test_support::{act, ev, orders, parked, two_tabs, with_second_session};
use crate::viewmodel::aggregate_run::RunStatus;
use crate::viewmodel::index_write::IndexWrite;
use crate::viewmodel::{DocAction, EditorAction};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use uuid::Uuid;

/// Session A (two tabs) with a write in flight from its parked tab two, then session B connected
/// in front of it: A is in the background. `prepare` shapes tab two's state before B connects.
fn background_writer(prepare: impl FnOnce(&mut AppState, TabId)) -> (AppState, Uuid, TabId, Uuid) {
    let t = two_tabs();
    let mut s = t.s;
    prepare(&mut s, t.second);
    s.pending = Some(Pending {
        ns: orders(),
        editor: None,
        tab: Some(t.second),
    });
    let (s, b, _) = with_second_session(s);
    assert_eq!(s.active, Some(b), "A is in the background");
    assert!(s.parked[&t.a].pending.is_some(), "A's write is in A's view");
    assert!(s.pending.is_none(), "B has none");
    (s, t.a, t.second, b)
}

fn failed(session: Uuid, context: &'static str, error: CoreError) -> Event {
    Event::Failed {
        session,
        context,
        error,
    }
}

#[test]
fn a_refused_save_of_a_background_sessions_parked_tab_shows_its_error_there() {
    let t = two_tabs();
    let (s, _) = act(t.s, t.second, Input::Doc(DocAction::Insert));
    let text = parked(&s, t.second)
        .editor
        .as_ref()
        .expect("an editor")
        .text
        .clone();
    let (s, commands) = act(
        s,
        t.second,
        Input::Editor(EditorAction::Save {
            original_id: None,
            text,
        }),
    );
    assert!(matches!(commands.as_slice(), [Command::Insert { .. }]));
    let (s, b, _) = with_second_session(s);
    let before_b = s.selected_ns().cloned();
    let s = ev(
        s,
        failed(t.a, "insert", CoreError::Database("refused".into())),
    );
    let error = parked(&s, t.second)
        .editor
        .as_ref()
        .and_then(|e| e.error.clone());
    assert_eq!(error, Some(CoreError::Database("refused".into())));
    assert!(s.parked[&t.a].pending.is_none(), "A's write is freed");
    assert_eq!(s.active, Some(b), "B stays in front");
    assert_eq!(s.selected_ns().cloned(), before_b, "B's tab is untouched");
}

#[test]
fn a_failed_index_write_of_a_background_sessions_parked_tab_settles_there() {
    let (s, a, second, b) = background_writer(|s, tab| {
        if let Some(view) = s.parked_tabs.get_mut(&tab) {
            view.indexes.writing = Some(IndexWrite::Create {
                name: "a_1".into(),
                replaces: None,
            });
        }
    });
    let s = ev(
        s,
        failed(a, "create_index", CoreError::Database("dup".into())),
    );
    assert!(parked(&s, second).indexes.writing.is_none());
    assert!(s.parked[&a].pending.is_none());
    assert_eq!(s.active, Some(b));
    assert_eq!(s.status_of(a), Some(ConnStatus::Connected));
}

#[test]
fn a_failed_run_of_a_background_sessions_parked_tab_does_not_stay_running() {
    let (s, a, second, _) = background_writer(|s, tab| {
        if let Some(view) = s.parked_tabs.get_mut(&tab) {
            view.aggregate.run.status = RunStatus::Running;
        }
    });
    let s = ev(
        s,
        failed(a, "aggregate_run", CoreError::Database("x".into())),
    );
    assert_ne!(parked(&s, second).aggregate.run.status, RunStatus::Running);
}

#[test]
fn a_background_sessions_failure_leaves_the_session_in_front_alone() {
    let (s, a, _, b) = background_writer(|_, _| {});
    let in_front_page = s.page.as_ref().map(|p| p.docs.len());
    let s = ev(
        s,
        failed(a, "insert", CoreError::Database("refused".into())),
    );
    assert_eq!(s.active, Some(b));
    assert_eq!(s.page.as_ref().map(|p| p.docs.len()), in_front_page);
    let _ = update(s, Input::DismissBanner);
}
