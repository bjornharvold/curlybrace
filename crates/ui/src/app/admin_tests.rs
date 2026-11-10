//! The events of the administration commands: a change refreshes the sidebar lists, and the
//! statistics answers are consumed.

use super::state::{AppState, ConnStatus};
use super::{Input, update};
use curlybrace_core::admin::{CollectionStats, DatabaseStats};
use curlybrace_core::runtime::{Command, EMBEDDED_SESSION, Event};
use curlybrace_core::store::Namespace;

/// A connected session with `app` expanded and loaded, and `other` not loaded.
fn browsing() -> AppState {
    let mut s = AppState::new(vec![]);
    s.open = vec![EMBEDDED_SESSION];
    s.active = Some(EMBEDDED_SESSION);
    s.status = ConnStatus::Connected;
    s.sidebar = s
        .sidebar
        .with_databases(vec!["app".into(), "other".into()])
        .with_collections("app", vec!["people".into()])
        .toggled("app");
    s
}

fn event(state: AppState, ev: Event) -> (AppState, Vec<Command>) {
    update(state, Input::Event(ev))
}

fn contexts(commands: &[Command]) -> Vec<&'static str> {
    commands.iter().map(Command::context).collect()
}

fn admin_changed(db: &str) -> Event {
    Event::AdminChanged {
        session: EMBEDDED_SESSION,
        db: db.into(),
    }
}

#[test]
fn an_admin_change_lists_the_databases_and_the_changed_one_again() {
    let (state, commands) = event(browsing(), admin_changed("app"));
    assert_eq!(contexts(&commands), ["list_databases", "list_collections"]);
    assert!(matches!(
        &commands[1],
        Command::ListCollections { db, .. } if db == "app"
    ));
    assert!(state.loading.contains("app"));
}

#[test]
fn a_change_to_a_database_that_is_not_loaded_only_lists_the_databases() {
    let (state, commands) = event(browsing(), admin_changed("other"));
    assert_eq!(contexts(&commands), ["list_databases"]);
    assert!(state.loading.is_empty());
}

#[test]
fn a_second_change_while_the_collections_load_does_not_list_them_twice() {
    let (state, _) = event(browsing(), admin_changed("app"));
    let (_, commands) = event(state, admin_changed("app"));
    assert_eq!(contexts(&commands), ["list_databases"]);
}

#[test]
fn statistics_answers_change_nothing_yet() {
    let (state, commands) = event(
        browsing(),
        Event::CollectionStats {
            session: EMBEDDED_SESSION,
            ns: Namespace::new("app", "people"),
            stats: CollectionStats::default(),
            request: 1,
        },
    );
    assert!(commands.is_empty() && state.loading.is_empty());
    let (state, commands) = event(
        state,
        Event::DatabaseStats {
            session: EMBEDDED_SESSION,
            db: "app".into(),
            stats: DatabaseStats::default(),
            request: 2,
        },
    );
    assert!(commands.is_empty() && state.loading.is_empty());
}
