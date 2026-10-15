//! A session whose server becomes unreachable after connecting: its root fails with a Reconnect.
use crate::I18n;
use crate::app::reducer::{Input, Out, update};
use crate::app::state::{AppState, Banner, ConnStatus};
use crate::viewmodel::{EditorAction, SidebarAction};
use bson::doc;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Namespace, Page, Query};
use uuid::Uuid;

fn app(names: &[&str]) -> (AppState, Vec<Uuid>) {
    let cfgs: Vec<_> = names
        .iter()
        .map(|n| ConnectionConfig::new(n, "mongodb://localhost"))
        .collect();
    let ids = cfgs.iter().map(|c| c.id).collect();
    (AppState::new(cfgs), ids)
}

fn ev(state: AppState, event: Event) -> Out {
    update(state, Input::Event(event))
}

fn connected(session: Uuid, epoch: u64) -> Event {
    Event::Connected {
        session,
        epoch,
        topology: None,
    }
}

fn opened(state: AppState, id: Uuid) -> AppState {
    let (s, _) = update(state, Input::Connect(id));
    let epoch = s.epoch_counter;
    ev(s, connected(id, epoch)).0
}

fn users() -> Namespace {
    Namespace::new("app", "users")
}

/// `id` open and active with a page of users on screen.
fn browsing(state: AppState, id: Uuid) -> AppState {
    let s = update(state, Input::Sidebar(SidebarAction::Open(users()))).0;
    ev(
        s,
        Event::Results {
            session: id,
            ns: users(),
            query: Query::default(),
            page: Page {
                docs: vec![doc! { "who": "ada" }],
                skip: 0,
                has_more: false,
            },
        },
    )
    .0
}

fn lost(session: Uuid, context: &'static str) -> Event {
    Event::Failed {
        session,
        context,
        error: CoreError::Unreachable,
    }
}

fn is_banner(s: &AppState, context: &'static str, owner: Uuid) -> bool {
    matches!(&s.banner, Some(Banner::Failure { context: c, error: CoreError::Unreachable }) if *c == context)
        && s.banner_owner() == Some(owner)
}

#[test]
fn an_unreachable_find_marks_the_focused_root_failed_and_clears_its_view() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let s = browsing(opened(s, a), a);
    assert!(s.page.is_some());
    let (s, cmds) = ev(s, lost(a, "find"));
    assert!(cmds.is_empty());
    assert_eq!((s.active, s.status), (Some(a), ConnStatus::Failed));
    assert_eq!(s.failure, Some(CoreError::Unreachable));
    assert!(s.page.is_none() && s.selected_ns().is_none() && !s.loading_page);
    assert!(is_banner(&s, "find", a));
    assert_eq!(s.forget_target, None);
    let roots = s.roots(&I18n::english());
    assert_eq!(roots[0].status, ConnStatus::Failed);
    assert_eq!(roots[0].failure, Some(CoreError::Unreachable));
}

#[test]
fn every_data_command_that_loses_the_server_fails_the_root() {
    for context in [
        "list_databases",
        "list_collections",
        "find",
        "insert",
        "replace",
        "delete",
        "clone",
    ] {
        let (s, ids) = app(&["a"]);
        let a = ids[0];
        let (s, _) = ev(opened(s, a), lost(a, context));
        assert_eq!(s.status, ConnStatus::Failed, "{context}");
        assert_eq!(s.failure, Some(CoreError::Unreachable), "{context}");
        assert!(is_banner(&s, context, a), "{context}");
    }
}

#[test]
fn a_lost_session_drops_its_write_in_flight_with_the_view() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let s = browsing(opened(s, a), a);
    let (s, cmds) = update(
        s,
        Input::Editor(EditorAction::Save {
            original_id: None,
            text: "{}".into(),
        }),
    );
    assert!(matches!(cmds.as_slice(), [Command::Insert { .. }]));
    assert!(s.pending.is_some());
    let (s, _) = ev(s, lost(a, "insert"));
    assert!(s.pending.is_none() && s.editor.is_none());
    assert_eq!(s.status, ConnStatus::Failed);
}

#[test]
fn a_background_session_lost_keeps_the_banner_of_another_session() {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let s = opened(opened(s, a), b);
    // b is active; a failure of b raises its banner.
    let (s, _) = ev(
        s,
        Event::Failed {
            session: b,
            context: "find",
            error: CoreError::NotFound,
        },
    );
    assert_eq!(s.banner_owner(), Some(b));
    let (s, _) = ev(s, lost(a, "find"));
    assert_eq!(s.banner_owner(), Some(b), "b's banner stays");
    assert!(matches!(
        s.banner,
        Some(Banner::Failure {
            error: CoreError::NotFound,
            ..
        })
    ));
    assert_eq!((s.active, s.status), (Some(b), ConnStatus::Connected));
    assert_eq!(s.parked[&a].status, ConnStatus::Failed);
    assert_eq!(s.parked[&a].failure, Some(CoreError::Unreachable));
    let roots = s.roots(&I18n::english());
    assert_eq!(roots[0].status, ConnStatus::Failed);
    assert_eq!(roots[1].status, ConnStatus::Connected);
}

#[test]
fn a_background_session_lost_with_no_banner_up_names_itself() {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let s = opened(opened(s, a), b);
    assert!(s.banner.is_none());
    let (s, _) = ev(s, lost(a, "list_collections"));
    assert!(is_banner(&s, "list_collections", a));
    assert_eq!(s.active, Some(b), "the focus does not move");
    assert_eq!(s.parked[&a].status, ConnStatus::Failed);
}

#[test]
fn an_unreachable_answer_of_a_closed_session_is_dropped() {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let s = opened(opened(s, a), b);
    let (s, _) = update(s, Input::Disconnect(a));
    let (s, cmds) = ev(s, lost(a, "find"));
    assert!(cmds.is_empty());
    assert_eq!(s.open, vec![b]);
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(s.banner.is_none());
}

#[test]
fn an_unreachable_answer_while_connecting_or_failed_is_ignored() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let (s, _) = update(s, Input::Connect(a));
    let (s, _) = ev(s, lost(a, "find"));
    assert_eq!(
        (s.status, s.failure.clone()),
        (ConnStatus::Connecting, None)
    );
    assert!(s.banner.is_none());
    // Failed: a second lost command of the same session changes nothing.
    let (s2, ids2) = app(&["z"]);
    let z = ids2[0];
    let (s2, _) = ev(opened(s2, z), lost(z, "find"));
    let (s2, _) = update(s2, Input::DismissBanner);
    let (s2, _) = ev(s2, lost(z, "list_collections"));
    assert_eq!(s2.status, ConnStatus::Failed);
    assert!(s2.banner.is_none(), "an already failed root ignores it");
}

#[test]
fn other_data_failures_still_only_raise_the_banner() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let s = browsing(opened(s, a), a);
    let (s, _) = ev(
        s,
        Event::Failed {
            session: a,
            context: "find",
            error: CoreError::Database("timeout".into()),
        },
    );
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(s.failure.is_none() && s.page.is_some());
    assert!(s.banner.is_some());
}

#[test]
fn reconnect_after_a_lost_server_sends_connect_with_the_next_epoch_and_applies_its_events() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let s = opened(s, a);
    assert_eq!(s.epoch_counter, 1);
    let (s, _) = ev(s, lost(a, "find"));
    let (s, cmds) = update(s, Input::Connect(a));
    assert!(matches!(cmds.as_slice(), [Command::Connect(cfg)] if cfg.id == a));
    assert_eq!((s.epoch_counter, s.latest_requested_epoch), (2, 2));
    assert_eq!(
        (s.status, s.failure.clone()),
        (ConnStatus::Connecting, None)
    );
    assert!(s.banner.is_none());
    let (s, cmds) = ev(s, connected(a, 2));
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(matches!(cmds.as_slice(), [Command::ListDatabases { session }] if *session == a));
}

#[test]
fn closing_a_lost_root_sends_a_disconnect_and_clears_its_banner() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let (s, _) = ev(opened(s, a), lost(a, "find"));
    let (s, cmds) = update(s, Input::Disconnect(a));
    assert!(matches!(cmds.as_slice(), [Command::Disconnect { session }] if *session == a));
    assert!(s.open.is_empty() && s.banner.is_none());
}

#[test]
fn a_lost_session_is_not_counted_as_connected_for_the_forget_offer() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let (s, _) = ev(opened(s, a), lost(a, "find"));
    assert_eq!(s.forget_target, None);
}
