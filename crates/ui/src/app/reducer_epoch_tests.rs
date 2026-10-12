//! Connect/Disconnect outcomes arrive in any order; epochs and `Superseded` decide.
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

/// Connect A (epoch 1), then Connect B (epoch 2).
fn connect_a_then_b() -> AppState {
    let (s, id) = app_with_connection();
    let (s, _) = run(s, Input::Connect(id));
    run(s, Input::Connect(id)).0
}

#[test]
fn a_fast_second_connect_wins_when_its_connected_arrives_before_the_superseded_one() {
    let (s, cmds) = feed(connect_a_then_b(), vec![connected(2), superseded()]);
    assert_eq!(s.status, ConnStatus::Connected);
    assert_eq!(list_databases_sent(&cmds), 1);
}

#[test]
fn a_fast_second_connect_wins_when_the_superseded_one_arrives_first() {
    let (s, cmds) = feed(connect_a_then_b(), vec![superseded(), connected(2)]);
    assert_eq!(s.status, ConnStatus::Connected);
    assert_eq!(list_databases_sent(&cmds), 1);
}

#[test]
fn a_real_failure_of_the_second_connect_is_shown_in_either_order() {
    let failed = || Event::Failed {
        context: "connect",
        error: CoreError::Database("refused".into()),
    };
    for events in [vec![failed(), superseded()], vec![superseded(), failed()]] {
        let (s, cmds) = feed(connect_a_then_b(), events);
        assert_eq!(s.status, ConnStatus::Disconnected);
        assert!(cmds.is_empty());
        assert_eq!(
            s.banner,
            Some(Banner::Failure {
                context: "connect",
                error: CoreError::Database("refused".into())
            })
        );
    }
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
fn connect_disconnect_connect_ends_connected_in_every_arrival_order() {
    let (s, id) = app_with_connection();
    let outcomes = vec![superseded(), disconnected(2), connected(3)];
    let orders = permutations(outcomes);
    assert_eq!(orders.len(), 6);
    for events in orders {
        let label = format!("{events:?}");
        let (state, _) = run(s_clone(&s), Input::Connect(id));
        let (state, _) = run(state, Input::Disconnect);
        let (state, _) = run(state, Input::Connect(id));
        let (state, cmds) = feed(state, events);
        assert_eq!(state.status, ConnStatus::Connected, "{label}");
        assert_eq!(list_databases_sent(&cmds), 1, "{label}");
        assert_eq!(state.banner, None, "{label}");
    }
}

/// `AppState` is not `Clone`; rebuild an equivalent starting state.
fn s_clone(s: &AppState) -> AppState {
    AppState::new(s.conn.connections.clone())
}

#[test]
fn connect_then_disconnect_ends_disconnected_in_either_order() {
    for events in [
        vec![superseded(), disconnected(2)],
        vec![disconnected(2), superseded()],
    ] {
        let (s, id) = app_with_connection();
        let (s, _) = run(s, Input::Connect(id));
        let (s, _) = run(s, Input::Disconnect);
        let (s, cmds) = feed(s, events);
        assert_eq!(s.status, ConnStatus::Disconnected);
        assert!(cmds.is_empty());
    }
}

#[test]
fn a_stale_disconnected_after_a_newer_connected_is_ignored() {
    let (s, _) = run(AppState::new(vec![]), Input::Disconnect);
    let (s, _) = feed(s, vec![connected(5)]);
    assert_eq!(s.status, ConnStatus::Connected);
    let (s, cmds) = event(s, disconnected(2));
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(cmds.is_empty() && s.highest_epoch_seen == 5);
}

#[test]
fn superseded_failures_never_touch_the_banner_or_status() {
    let (s, cmds) = event(browsing(), superseded());
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(cmds.is_empty() && s.banner.is_none());
}
