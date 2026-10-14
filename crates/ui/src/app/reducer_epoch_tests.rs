//! Connect/Disconnect outcomes arrive in any order; epochs and `Superseded` decide.
//! The UI mirrors the runtime's request counter: every Connect/Disconnect it sends is epoch
//! `latest_requested_epoch` (1, 2, 3, ...), and an outcome older than that is ignored.
use super::*;
use curlybrace_core::runtime::EMBEDDED_SESSION;

fn app_with_connection() -> (AppState, uuid::Uuid) {
    // The connection IS the session the events below address, so they are routed to its root.
    let cfg = ConnectionConfig {
        id: EMBEDDED_SESSION,
        ..ConnectionConfig::new("a", "mongodb://localhost")
    };
    let id = cfg.id;
    (AppState::new(vec![cfg]), id)
}

fn superseded() -> Event {
    Event::Failed {
        session: EMBEDDED_SESSION,
        context: "connect",
        error: CoreError::Superseded,
    }
}

fn connected(epoch: u64) -> Event {
    Event::Connected {
        session: EMBEDDED_SESSION,
        epoch,
        topology: None,
    }
}

fn disconnected(epoch: u64) -> Event {
    Event::Disconnected {
        session: EMBEDDED_SESSION,
        epoch,
    }
}

fn connect_failed(epoch: u64) -> Event {
    Event::ConnectFailed {
        session: EMBEDDED_SESSION,
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
        .filter(|c| matches!(c, Command::ListDatabases { .. }))
        .count()
}

/// Starts from a fresh state and dispatches `steps` (true = Connect, false = Disconnect).
/// A Connect on a root that is still open only focuses it, so a repeated Connect models a retry:
/// the previous attempt is marked failed first (its outcome event is fed by the test itself).
fn dispatched(steps: &[bool]) -> AppState {
    let (mut s, id) = app_with_connection();
    for connect in steps {
        let input = if *connect {
            if s.is_open(id) {
                s.status = ConnStatus::Failed;
            }
            Input::Connect(id)
        } else {
            Input::Disconnect(id)
        };
        s = run(s, input).0;
    }
    // Every step must really have been dispatched (a Connect with an unknown id would not be).
    assert_eq!(s.epoch_counter as usize, steps.len());
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
    let s = dispatched(&[true, true, false, true]);
    assert_eq!((s.epoch_counter, s.latest_requested_epoch), (4, 4));
    // An unknown id sends no command and consumes no epoch.
    let (s, _) = run(AppState::new(vec![]), Input::Connect(uuid::Uuid::new_v4()));
    assert_eq!((s.epoch_counter, s.latest_requested_epoch), (0, 0));
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
        assert_eq!(s.status, ConnStatus::Failed);
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
    assert_eq!(s.status, ConnStatus::Failed);
    assert_eq!(s.banner, refused_banner());
    assert_eq!(list_databases_sent(&cmds), 0);
}

#[test]
fn a_consistent_real_failure_then_a_connected_ends_connected() {
    // A fails for real (epoch 1) before B (epoch 2) is requested; B then connects.
    let s = dispatched(&[true]);
    let (s, _) = event(s, connect_failed(1));
    let (s, _) = run(s, Input::Connect(uuid_of(&dispatched(&[true]))));
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
        assert_eq!(s.status, ConnStatus::Failed, "{label}");
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

// ---- session stamping: the single-session UI addresses the connection it opened ----

fn session_of(command: &Command) -> Option<uuid::Uuid> {
    command.session()
}

#[test]
fn connect_data_and_disconnect_commands_all_carry_the_opened_connections_id() {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    assert_ne!(id, EMBEDDED_SESSION);
    let (s, cmds) = update(AppState::new(vec![cfg]), Input::Connect(id));
    assert_eq!(cmds.iter().map(session_of).collect::<Vec<_>>(), [Some(id)]);
    let (s, cmds) = event(
        s,
        Event::Connected {
            session: id,
            epoch: 1,
            topology: None,
        },
    );
    assert_eq!(
        cmds.iter().map(session_of).collect::<Vec<_>>(),
        [Some(id)],
        "the first fetch after connecting addresses the opened connection"
    );
    let (_, cmds) = update(s, Input::Disconnect(id));
    assert_eq!(
        cmds.iter().map(session_of).collect::<Vec<_>>(),
        [Some(id)],
        "disconnect names the session it closes"
    );
}

#[test]
fn without_an_opened_connection_commands_address_the_embedded_session() {
    assert_eq!(AppState::new(vec![]).session_id(), EMBEDDED_SESSION);
    // There is no root to close: nothing is sent and no epoch is consumed.
    let (s, cmds) = update(AppState::new(vec![]), Input::Disconnect(EMBEDDED_SESSION));
    assert!(cmds.is_empty());
    assert_eq!(s.epoch_counter, 0);
}

// ---- switching connections keeps the previous session open ----

fn two_connections() -> (AppState, uuid::Uuid, uuid::Uuid) {
    let (a, b) = (
        ConnectionConfig::new("a", "mongodb://localhost"),
        ConnectionConfig::new("b", "mongodb://localhost"),
    );
    let ids = (a.id, b.id);
    (AppState::new(vec![a, b]), ids.0, ids.1)
}

#[test]
fn connecting_to_another_connection_keeps_the_open_one_and_sends_only_a_connect() {
    // Replaces the single-session behaviour (Disconnect of the old session, then Connect, two
    // epochs): the old session is parked, still open, and only one epoch is consumed.
    let (s, a, b) = two_connections();
    let (s, _) = update(s, Input::Connect(a));
    let before = s.epoch_counter;
    let (s, cmds) = update(s, Input::Connect(b));
    assert!(matches!(cmds.as_slice(), [Command::Connect(cfg)] if cfg.id == b));
    assert_eq!(s.epoch_counter, before + 1);
    assert_eq!(s.active, Some(b));
    assert_eq!(s.open, vec![a, b]);
}

#[test]
fn reconnecting_a_failed_connection_sends_only_a_connect() {
    let (s, a, _) = two_connections();
    let (s, _) = update(s, Input::Connect(a));
    let (s, _) = event(s, connect_failed_for(a, 1));
    let before = s.epoch_counter;
    let (s, cmds) = update(s, Input::Connect(a));
    assert!(matches!(cmds.as_slice(), [Command::Connect(cfg)] if cfg.id == a));
    assert_eq!(s.epoch_counter, before + 1);
}

fn connect_failed_for(session: uuid::Uuid, epoch: u64) -> Event {
    Event::ConnectFailed {
        session,
        epoch,
        error: CoreError::Database("refused".into()),
    }
}

#[test]
fn the_first_connect_sends_only_a_connect() {
    let (s, a, _) = two_connections();
    let (s, cmds) = update(s, Input::Connect(a));
    assert!(matches!(cmds.as_slice(), [Command::Connect(_)]));
    assert_eq!(s.latest_requested_epoch, 1);
}
