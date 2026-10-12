//! Connect/Disconnect outcomes arrive in any order; epochs and `Superseded` decide.
//! The UI mirrors the runtime's request counter: every Connect/Disconnect it sends is epoch
//! `latest_requested_epoch` (1, 2, 3, ...), and an outcome older than that is ignored.
use super::*;

fn app_with_connection() -> (AppState, uuid::Uuid) {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    (AppState::new(vec![cfg]), id)
}

fn superseded() -> Event {
    Event::Failed {
        context: "connect",
        error: CoreError::Superseded,
    }
}

fn connected(epoch: u64) -> Event {
    Event::Connected { epoch }
}

fn disconnected(epoch: u64) -> Event {
    Event::Disconnected { epoch }
}

fn connect_failed(epoch: u64) -> Event {
    Event::ConnectFailed {
        epoch,
        error: CoreError::Database("refused".into()),
    }
}

/// Feeds `events` in order; returns the final state and every command they produced.
fn feed(state: AppState, events: Vec<Event>) -> (AppState, Vec<Command>) {
    events
        .into_iter()
        .fold((state, Vec::new()), |(s, mut all), ev| {
            let (s, cmds) = event(s, ev);
            all.extend(cmds);
            (s, all)
        })
}

fn list_databases_sent(cmds: &[Command]) -> usize {
    cmds.iter()
        .filter(|c| matches!(c, Command::ListDatabases))
        .count()
}

/// Starts from a fresh state and dispatches `steps` (true = Connect, false = Disconnect).
fn dispatched(steps: &[bool]) -> AppState {
    let (mut s, id) = app_with_connection();
    for connect in steps {
        let input = if *connect {
            Input::Connect(id)
        } else {
            Input::Disconnect
        };
        s = run(s, input).0;
    }
    s
}

fn refused_banner() -> Option<Banner> {
    Some(Banner::Failure {
        context: "connect",
        error: CoreError::Database("refused".into()),
    })
}

fn permutations(items: Vec<Event>) -> Vec<Vec<Event>> {
    if items.len() <= 1 {
        return vec![items];
    }
    let mut out = Vec::new();
    for i in 0..items.len() {
        let mut rest = items.clone();
        let head = rest.remove(i);
        for mut tail in permutations(rest) {
            tail.insert(0, head.clone());
            out.push(tail);
        }
    }
    out
}

#[test]
fn every_connect_and_disconnect_dispatch_bumps_the_mirrored_epoch() {
    assert_eq!(
        dispatched(&[true, true, false, true]).latest_requested_epoch,
        4
    );
    // An unknown id sends no command and consumes no epoch.
    let (s, _) = run(AppState::new(vec![]), Input::Connect(uuid::Uuid::new_v4()));
    assert_eq!(s.latest_requested_epoch, 0);
}

#[test]
fn a_fast_second_connect_wins_in_either_arrival_order() {
    for events in [
        vec![connected(2), superseded()],
        vec![superseded(), connected(2)],
    ] {
        let (s, cmds) = feed(dispatched(&[true, true]), events);
        assert_eq!(s.status, ConnStatus::Connected);
        assert_eq!(list_databases_sent(&cmds), 1);
    }
}

#[test]
fn a_real_failure_of_the_second_connect_is_shown_in_either_order() {
    for events in [
        vec![connect_failed(2), superseded()],
        vec![superseded(), connect_failed(2)],
    ] {
        let (s, cmds) = feed(dispatched(&[true, true]), events);
        assert_eq!(s.status, ConnStatus::Disconnected);
        assert!(cmds.is_empty());
        assert_eq!(s.banner, refused_banner());
    }
}

#[test]
fn a_stale_real_failure_of_a_replaced_connect_cannot_clobber_the_newer_one() {
    // Ordering 1: A's failure was emitted before B was dispatched, processed after B's Connected.
    let (s, cmds) = feed(
        dispatched(&[true, true]),
        vec![connected(2), connect_failed(1)],
    );
    assert_eq!(s.status, ConnStatus::Connected);
    assert_eq!(s.banner, None);
    assert_eq!(list_databases_sent(&cmds), 1);
}

#[test]
fn a_stale_connected_after_a_newer_connect_failed_is_ignored() {
    // Ordering 2: A installed, the user clicked B, B failed fast, then A's Connected arrives.
    let (s, cmds) = feed(
        dispatched(&[true, true]),
        vec![connect_failed(2), connected(1)],
    );
    assert_eq!(s.status, ConnStatus::Disconnected);
    assert_eq!(s.banner, refused_banner());
    assert_eq!(list_databases_sent(&cmds), 0);
}

#[test]
fn a_consistent_real_failure_then_a_connected_ends_connected() {
    // A fails for real (epoch 1) before B (epoch 2) is requested; B then connects.
    let (s, _) = run(
        dispatched(&[true]),
        Input::Connect(uuid_of(&dispatched(&[true]))),
    );
    let (s, _) = event(s, connect_failed(1));
    let (s, cmds) = event(s, connected(2));
    assert_eq!(s.status, ConnStatus::Connected);
    assert_eq!(list_databases_sent(&cmds), 1);
}

fn uuid_of(s: &AppState) -> uuid::Uuid {
    s.conn.connections[0].id
}

#[test]
fn a_stale_connected_after_a_disconnect_was_dispatched_is_ignored() {
    let (s, cmds) = feed(dispatched(&[true, false]), vec![connected(1)]);
    assert_eq!(s.status, ConnStatus::Disconnected);
    assert_eq!(list_databases_sent(&cmds), 0);
}

#[test]
fn a_stale_connect_failure_after_a_newer_connect_is_ignored_without_a_banner() {
    let (s, _) = feed(dispatched(&[true, true]), vec![connect_failed(1)]);
    assert_eq!(s.status, ConnStatus::Connecting);
    assert_eq!(s.banner, None);
}

#[test]
fn all_arrival_orders_of_two_connects_end_in_the_second_ones_outcome() {
    let ok = permutations(vec![superseded(), connected(2), connect_failed(1)]);
    assert_eq!(ok.len(), 6);
    for events in ok {
        let label = format!("{events:?}");
        let (s, cmds) = feed(dispatched(&[true, true]), events);
        assert_eq!(s.status, ConnStatus::Connected, "{label}");
        assert_eq!(list_databases_sent(&cmds), 1, "{label}");
        assert_eq!(s.banner, None, "{label}");
    }
    let failed = permutations(vec![superseded(), connect_failed(2), connected(1)]);
    for events in failed {
        let label = format!("{events:?}");
        let (s, cmds) = feed(dispatched(&[true, true]), events);
        assert_eq!(s.status, ConnStatus::Disconnected, "{label}");
        assert_eq!(list_databases_sent(&cmds), 0, "{label}");
        assert_eq!(s.banner, refused_banner(), "{label}");
    }
}

#[test]
fn connect_disconnect_connect_ends_connected_in_every_arrival_order() {
    let orders = permutations(vec![superseded(), disconnected(2), connected(3)]);
    assert_eq!(orders.len(), 6);
    for events in orders {
        let label = format!("{events:?}");
        let (state, cmds) = feed(dispatched(&[true, false, true]), events);
        assert_eq!(state.status, ConnStatus::Connected, "{label}");
        assert_eq!(list_databases_sent(&cmds), 1, "{label}");
        assert_eq!(state.banner, None, "{label}");
    }
}

#[test]
fn connect_then_disconnect_ends_disconnected_in_either_order() {
    for events in [
        vec![superseded(), disconnected(2)],
        vec![disconnected(2), superseded()],
    ] {
        let (s, cmds) = feed(dispatched(&[true, false]), events);
        assert_eq!(s.status, ConnStatus::Disconnected);
        assert!(cmds.is_empty());
    }
}

#[test]
fn a_stale_disconnected_after_a_newer_connected_is_ignored() {
    let (s, _) = feed(dispatched(&[true, false, true]), vec![connected(3)]);
    let (s, cmds) = event(s, disconnected(2));
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(cmds.is_empty() && s.latest_requested_epoch == 3);
}

#[test]
fn superseded_failures_never_touch_the_banner_or_status() {
    let (s, cmds) = event(browsing(), superseded());
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(cmds.is_empty() && s.banner.is_none());
}

#[test]
fn connecting_clears_leftover_data_failure_banners_but_keeps_other_ones() {
    let mut s = dispatched(&[true]);
    s.banner = Some(Banner::Failure {
        context: "find",
        error: CoreError::NotFound,
    });
    let (s, _) = event(s, connected(1));
    assert_eq!(s.banner, None);
    let mut s = dispatched(&[true]);
    s.banner = Some(Banner::HostForgotten);
    let (s, _) = event(s, connected(1));
    assert_eq!(s.banner, Some(Banner::HostForgotten));
}

#[test]
fn session_clearing_does_not_reset_the_mirrored_epoch() {
    let s = dispatched(&[true, true]);
    assert_eq!(s.session_cleared().latest_requested_epoch, 2);
}
