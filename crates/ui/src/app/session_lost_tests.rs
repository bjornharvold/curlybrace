//! A session whose server becomes unreachable after connecting: its root fails with a Reconnect.
use crate::I18n;
use crate::app::reducer::{Input, Out, update};
use crate::app::state::{AppState, Banner, ConnStatus, HostEndpoint};
use crate::viewmodel::{DocAction, EditorAction, SidebarAction};
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
    for context in ["list_databases", "list_collections", "find"] {
        let (s, ids) = app(&["a"]);
        let a = ids[0];
        let (s, _) = ev(opened(s, a), lost(a, context));
        assert_eq!(s.status, ConnStatus::Failed, "{context}");
        assert_eq!(s.failure, Some(CoreError::Unreachable), "{context}");
        assert!(is_banner(&s, context, a), "{context}");
    }
}

#[test]
fn a_lost_delete_or_clone_fails_the_root_and_says_the_change_may_have_applied() {
    for (context, tunnel, expected) in [
        ("delete", false, CoreError::UnreachableWrite),
        ("clone", false, CoreError::UnreachableWrite),
        ("delete", true, CoreError::UnreachableWriteSsh),
        ("clone", true, CoreError::UnreachableWriteSsh),
    ] {
        let (s, ids) = app(&["a"]);
        let a = ids[0];
        let s = opened(s, a);
        let s = if tunnel { behind_a_tunnel(s) } else { s };
        let (s, _) = ev(s, lost(a, context));
        assert_eq!(s.status, ConnStatus::Failed, "{context}");
        assert_eq!(s.failure, Some(expected.clone()), "{context}");
        assert!(
            matches!(&s.banner, Some(Banner::Failure { context: c, error }) if *c == context && *error == expected),
            "{context}"
        );
    }
}

/// `a` browsing with the new-document editor open and a save of `{}` in flight.
fn saving(state: AppState, id: Uuid) -> AppState {
    let s = browsing(opened(state, id), id);
    let (s, _) = update(s, Input::Doc(DocAction::Insert));
    assert!(s.editor.is_some());
    let (s, cmds) = update(
        s,
        Input::Editor(EditorAction::Save {
            original_id: None,
            text: "{}".into(),
        }),
    );
    assert!(matches!(cmds.as_slice(), [Command::Insert { .. }]));
    assert!(s.pending.is_some());
    s
}

#[test]
fn a_lost_read_keeps_the_open_editor_text_and_the_root_connected() {
    for context in ["list_databases", "list_collections", "find"] {
        let (s, ids) = app(&["a"]);
        let a = ids[0];
        let s = saving(s, a);
        let (s, _) = ev(s, lost(a, context));
        assert_eq!(s.status, ConnStatus::Connected, "{context}");
        assert_eq!(s.editor.as_ref().map(|e| e.text.as_str()), Some("{}"));
        assert!(is_banner(&s, context, a), "{context}");
    }
}

#[test]
fn the_root_fails_on_the_next_lost_command_after_the_editor_closed() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let s = saving(s, a);
    let (s, _) = ev(s, lost(a, "find"));
    let (s, _) = update(s, Input::Editor(EditorAction::Cancel));
    assert!(s.editor.is_none());
    let (s, _) = ev(s, lost(a, "find"));
    assert_eq!(s.status, ConnStatus::Failed);
}

#[test]
fn a_lost_save_keeps_the_editor_text_and_does_not_fail_the_root() {
    for context in ["insert", "replace"] {
        let (s, ids) = app(&["a"]);
        let a = ids[0];
        let s = saving(s, a);
        let (s, _) = ev(s, lost(a, context));
        assert_eq!(s.status, ConnStatus::Connected, "{context}");
        assert!(s.pending.is_none(), "{context}");
        let editor = s.editor.expect("the text the user typed stays");
        assert_eq!(editor.error, Some(CoreError::UnreachableWrite), "{context}");
        assert_eq!(editor.text, "{}");
    }
}

#[test]
fn a_lost_insert_without_a_matching_editor_fails_the_root() {
    // A clone answers `insert` but no editor holds what was saved.
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let s = browsing(opened(s, a), a);
    assert!(s.editor.is_none());
    let (s, _) = ev(s, lost(a, "insert"));
    assert_eq!(s.status, ConnStatus::Failed);
    assert_eq!(s.failure, Some(CoreError::UnreachableWrite));
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

fn behind_a_tunnel(state: AppState) -> AppState {
    AppState {
        ssh_endpoint: Some(HostEndpoint {
            host: "bastion".into(),
            port: 22,
        }),
        ..state
    }
}

#[test]
fn a_lost_server_behind_a_tunnel_names_the_tunnel_on_the_root_and_the_banner() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let (s, _) = ev(behind_a_tunnel(opened(s, a)), lost(a, "find"));
    assert_eq!(s.failure, Some(CoreError::UnreachableSsh));
    assert!(matches!(
        &s.banner,
        Some(Banner::Failure {
            error: CoreError::UnreachableSsh,
            ..
        })
    ));
}

#[test]
fn a_lost_save_behind_a_tunnel_names_the_tunnel_in_the_editor() {
    let (s, ids) = app(&["a"]);
    let a = ids[0];
    let (s, _) = ev(behind_a_tunnel(saving(s, a)), lost(a, "insert"));
    let editor = s.editor.expect("the text stays");
    assert_eq!(editor.error, Some(CoreError::UnreachableWriteSsh));
}

#[test]
fn the_two_unreachable_texts_differ_only_in_naming_the_tunnel() {
    let i18n = I18n::english();
    let plain = i18n.error(&CoreError::Unreachable);
    let tunnel = i18n.error(&CoreError::UnreachableSsh);
    assert_ne!(plain, tunnel);
    assert!(tunnel.contains("SSH tunnel") && !plain.contains("SSH"));
}
