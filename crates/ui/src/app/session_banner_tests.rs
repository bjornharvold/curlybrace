//! Banners name the session they are about, and only that session's events clear them.
use crate::I18n;
use crate::app::banner::banner_prefix;
use crate::app::reducer::{Input, update};
use crate::app::state::{AppState, Banner, ConnStatus};
use curlybrace_core::connection::{ConnectionConfig, SshAuth, SshConfig};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Event;
use uuid::Uuid;

fn via(name: &str, host: &str) -> ConnectionConfig {
    let mut c = ConnectionConfig::new(name, "mongodb://db:27017");
    c.ssh = Some(SshConfig {
        host: host.into(),
        port: 2222,
        username: "me".into(),
        auth: SshAuth::Password,
    });
    c
}

fn app(names: &[&str]) -> (AppState, Vec<Uuid>) {
    let cfgs: Vec<_> = names
        .iter()
        .map(|n| via(n, &format!("bastion-{n}")))
        .collect();
    let ids = cfgs.iter().map(|c| c.id).collect();
    (AppState::new(cfgs), ids)
}

fn ev(state: AppState, event: Event) -> AppState {
    update(state, Input::Event(event)).0
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
    ev(s, connected(id, epoch))
}

fn find_failed(session: Uuid) -> Event {
    Event::Failed {
        session,
        context: "find",
        error: CoreError::NotFound,
    }
}

fn connect_failed(session: Uuid, epoch: u64, error: CoreError) -> Event {
    Event::ConnectFailed {
        session,
        epoch,
        error,
    }
}

fn prefix(s: &AppState) -> Option<String> {
    banner_prefix(s, &I18n::english())
}

fn is_find_failure(s: &AppState) -> bool {
    matches!(
        s.banner,
        Some(Banner::Failure {
            context: "find",
            ..
        })
    )
}

/// `a` active and connected, `b` connected in the background.
fn two_open() -> (AppState, Uuid, Uuid) {
    let (s, ids) = app(&["alpha", "beta"]);
    let s = opened(opened(s, ids[1]), ids[0]);
    (s, ids[0], ids[1])
}

/// `a` connecting (host key changed answer not yet delivered) is easier to build by hand.
fn host_key_changed(s: AppState, id: Uuid) -> AppState {
    let (s, _) = update(s, Input::Connect(id));
    let epoch = s.epoch_counter;
    ev(s, connect_failed(id, epoch, CoreError::SshHostKeyChanged))
}

// naming

#[test]
fn a_session_owned_failure_names_its_connection() {
    let (s, a, _) = two_open();
    let s = ev(s, find_failed(a));
    assert_eq!(prefix(&s).as_deref(), Some("In alpha:"));
}

#[test]
fn a_background_failure_is_named_with_the_background_session() {
    let (s, a, b) = two_open();
    let s = ev(s, find_failed(b));
    assert_eq!(s.active, Some(a));
    assert!(is_find_failure(&s));
    assert_eq!(prefix(&s).as_deref(), Some("In beta:"));
}

#[test]
fn a_data_failure_of_a_background_session_replaces_the_slot() {
    let (s, a, b) = two_open();
    let s = ev(s, find_failed(a));
    let s = ev(s, find_failed(b));
    assert_eq!(s.banner_session, Some(b));
    assert_eq!(prefix(&s).as_deref(), Some("In beta:"));
}

#[test]
fn an_ownerless_banner_has_no_prefix() {
    let (mut s, a, _) = two_open();
    s.banner = Some(Banner::NotConnected);
    assert_eq!(prefix(&s), None);
    // A stale owner never names an unrelated banner.
    s.banner_session = Some(a);
    assert_eq!(prefix(&s), None);
    s.banner = Some(Banner::Failure {
        context: "compare_open",
        error: CoreError::NotFound,
    });
    assert_eq!(prefix(&s), None);
}

#[test]
fn a_deleted_saved_connection_gives_no_prefix() {
    let (s, a, _) = two_open();
    let mut s = ev(s, find_failed(a));
    s.conn.connections.retain(|c| c.id != a);
    assert_eq!(prefix(&s), None);
}

// (a) connect

#[test]
fn a_banner_owned_by_another_session_survives_connect() {
    let (s, ids) = app(&["alpha", "beta", "gamma"]);
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    let s = ev(opened(opened(s, a), b), find_failed(a));
    let (s, _) = update(s, Input::Connect(c));
    assert!(s.banner.is_some());
    assert_eq!(s.banner_session, Some(a));
}

#[test]
fn connect_of_the_owner_clears_its_banner_and_the_forget_offer() {
    let (s, ids) = app(&["alpha", "beta"]);
    let s = host_key_changed(opened(s, ids[1]), ids[0]);
    assert!(s.forgettable_host().is_some());
    let (s, _) = update(s, Input::Connect(ids[0]));
    assert_eq!(s.banner, None);
    assert_eq!(s.banner_session, None);
    assert!(s.forget_target.is_none() && s.forgettable_host().is_none());
}

#[test]
fn another_connect_keeps_a_forget_offer() {
    let (s, ids) = app(&["alpha", "beta"]);
    let s = host_key_changed(s, ids[0]);
    let (s, _) = update(s, Input::Connect(ids[1]));
    assert!(s.forgettable_host().is_some());
}

#[test]
fn an_ownerless_banner_is_cleared_by_any_connect() {
    let (mut s, ids) = app(&["alpha"]);
    s.banner = Some(Banner::NotConnected);
    let (s, _) = update(s, Input::Connect(ids[0]));
    assert_eq!(s.banner, None);
}

// (b) Connected

#[test]
fn a_background_connected_keeps_the_active_sessions_data_failure() {
    let (s, ids) = app(&["alpha", "beta"]);
    let (a, b) = (ids[0], ids[1]);
    let (s, _) = update(s, Input::Connect(b));
    let b_epoch = s.epoch_counter;
    let s = ev(opened(s, a), find_failed(a));
    let s = ev(s, connected(b, b_epoch));
    assert_eq!(s.status_of(b), Some(ConnStatus::Connected));
    assert!(is_find_failure(&s));
    assert_eq!(s.banner_session, Some(a));
}

#[test]
fn the_active_sessions_own_connected_clears_its_data_failure() {
    let (s, ids) = app(&["alpha"]);
    let a = ids[0];
    let s = ev(opened(s, a), find_failed(a));
    assert!(s.banner.is_some());
    let s = ev(s, connected(a, 1));
    assert_eq!(s.banner, None);
    assert_eq!(s.banner_session, None);
}

#[test]
fn a_background_connected_clears_its_own_data_failure() {
    let (s, a, b) = two_open();
    let s = ev(s, find_failed(b));
    assert_eq!(s.banner_session, Some(b));
    let s = ev(s, connected(b, 1));
    assert_eq!(s.active, Some(a));
    assert_eq!(s.banner, None);
}

// (c) background connect failure

#[test]
fn a_background_connect_failure_keeps_another_sessions_banner() {
    let (s, ids) = app(&["alpha", "beta"]);
    let (a, b) = (ids[0], ids[1]);
    let (s, _) = update(s, Input::Connect(b));
    let b_epoch = s.epoch_counter;
    let s = ev(opened(s, a), find_failed(a));
    let s = ev(s, connect_failed(b, b_epoch, CoreError::AuthFailed));
    assert_eq!(s.banner_session, Some(a));
    assert!(is_find_failure(&s));
    assert_eq!(s.status_of(b), Some(ConnStatus::Failed));
    let root = s
        .roots(&I18n::english())
        .into_iter()
        .find(|r| r.id == b)
        .unwrap();
    assert_eq!(root.failure, Some(CoreError::AuthFailed));
}

#[test]
fn a_background_connect_failure_without_a_banner_raises_a_named_one() {
    let (s, ids) = app(&["alpha", "beta"]);
    let (a, b) = (ids[0], ids[1]);
    let (s, _) = update(s, Input::Connect(b));
    let b_epoch = s.epoch_counter;
    let s = ev(
        opened(s, a),
        connect_failed(b, b_epoch, CoreError::AuthFailed),
    );
    assert!(matches!(
        s.banner,
        Some(Banner::Failure {
            context: "connect",
            ..
        })
    ));
    assert_eq!(prefix(&s).as_deref(), Some("In beta:"));
}

#[test]
fn a_connect_failure_replaces_an_ownerless_banner() {
    let (s, ids) = app(&["alpha"]);
    let (mut s, _) = update(s, Input::Connect(ids[0]));
    s.banner = Some(Banner::NotConnected);
    let epoch = s.epoch_counter;
    let s = ev(s, connect_failed(ids[0], epoch, CoreError::AuthFailed));
    assert_eq!(prefix(&s).as_deref(), Some("In alpha:"));
}

// (d) / (e)

#[test]
fn disconnecting_the_owner_clears_its_failure() {
    let (s, a, _) = two_open();
    let s = ev(s, find_failed(a));
    let (s, _) = update(s, Input::Disconnect(a));
    assert_eq!((s.banner, s.banner_session), (None, None));
}

#[test]
fn dismiss_resets_the_owner_and_the_forget_target() {
    let (s, ids) = app(&["alpha"]);
    let s = host_key_changed(s, ids[0]);
    assert!(s.banner_session.is_some() && s.forget_target.is_some());
    let (s, _) = update(s, Input::DismissBanner);
    assert_eq!((s.banner_session, s.forget_target), (None, None));
}

#[test]
fn forget_finished_resets_the_owner_and_the_forget_target() {
    let (s, ids) = app(&["alpha"]);
    let s = host_key_changed(s, ids[0]);
    let (s, _) = update(s, Input::ForgetFinished(Ok(())));
    assert_eq!(s.banner, Some(Banner::HostForgotten));
    assert_eq!((s.banner_session, s.forget_target), (None, None));
}

#[test]
fn a_disconnect_after_a_dismiss_leaves_a_later_ownerless_failure() {
    let (s, a, _) = two_open();
    let s = ev(s, find_failed(a));
    let (mut s, _) = update(s, Input::DismissBanner);
    s.banner = Some(Banner::Failure {
        context: "compare_open",
        error: CoreError::NotFound,
    });
    let (s, _) = update(s, Input::Disconnect(a));
    assert!(s.banner.is_some());
}
