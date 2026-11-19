//! What the reducer does today at each site that acts on a session's view, pinned before the tab
//! model (#159, slice 1): a lost connection (with and without an editor open), a reconnect, a
//! disconnect, a write answered, a change of the database list, and stale or late answers.
use super::tab_characterisation_script::*;
use super::{Input, update};
use crate::app::AggregateAction;
use crate::app::state::{Banner, ConnStatus};
use crate::viewmodel::{CollectionTab, DocAction};
use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::Query;

#[test]
fn a_lost_connection_clears_what_the_server_showed_and_keeps_the_draft() {
    let (state, a, _) = browsing_one();
    let (state, _) = update(state, Input::Tab(CollectionTab::Aggregate));
    let (state, _) = update(
        state,
        Input::Aggregate(AggregateAction::AddStage("$match".into())),
    );
    let (state, _) = update(state, lost(a, "find"));
    assert_eq!(state.status, ConnStatus::Failed);
    assert_eq!(state.failure, Some(CoreError::Unreachable));
    // Slice 2 (T8) changes this on purpose: the tab keeps its collection and sub-tab.
    assert_eq!(state.selected_ns(), Some(&users()));
    assert!(state.page.is_none() && state.page_key.is_none());
    assert_eq!(state.tab, CollectionTab::Aggregate);
    assert_eq!(state.aggregate.stages.len(), 1, "the draft is the person's");
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "find",
            ..
        })
    ));
}

#[test]
fn a_lost_connection_with_an_editor_open_keeps_the_view_and_the_text() {
    let (state, a, _) = browsing_one();
    let (state, _) = update(state, Input::Doc(DocAction::Edit(doc! { "_id": 1 })));
    let (state, _) = update(state, lost(a, "find"));
    assert_eq!(
        state.status,
        ConnStatus::Connected,
        "one blip is not an outage"
    );
    assert!(state.editor.is_some());
    assert_eq!(state.selected_ns(), Some(&users()));
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "find",
            ..
        })
    ));
}

#[test]
fn a_reconnect_keeps_the_draft_and_the_collection_dormant() {
    let (state, a, _) = browsing_one();
    let (state, _) = update(state, Input::Tab(CollectionTab::Aggregate));
    let (state, _) = update(
        state,
        Input::Aggregate(AggregateAction::AddStage("$match".into())),
    );
    let (state, _) = update(state, lost(a, "find"));
    let (state, commands) = update(state, connect(a));
    assert!(matches!(commands.as_slice(), [Command::Connect(cfg)] if cfg.id == a));
    assert_eq!(state.status, ConnStatus::Connecting);
    // Slice 2 (T8): a reconnect keeps the tab's collection too (dormant until Run).
    assert_eq!(state.selected_ns(), Some(&users()));
    assert_eq!(state.aggregate.stages.len(), 1);
}

#[test]
fn a_disconnect_drops_the_session_view() {
    let (state, a, _) = browsing_one();
    let (state, commands) = update(state, Input::Disconnect(a));
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, Command::Disconnect { session } if *session == a))
    );
    assert!(state.open.is_empty() && state.active.is_none() && state.parked.is_empty());
    assert_eq!(state.selected_ns(), None);
    assert!(state.page.is_none());
}

#[test]
fn a_write_answer_refetches_the_page_of_the_selected_collection() {
    let (state, a, _) = browsing_one();
    let before = state.page_query.clone();
    let (state, commands) = update(
        state,
        event(Event::Mutated {
            session: a,
            ns: users(),
        }),
    );
    assert!(
        matches!(commands.as_slice(), [Command::Find { ns, query, .. }] if *ns == users() && *query == before)
    );
    assert!(state.loading_page);
}

#[test]
fn a_write_answer_for_another_collection_sends_nothing() {
    let (state, a, _) = browsing_one();
    let (_, commands) = update(
        state,
        event(Event::Mutated {
            session: a,
            ns: orders(),
        }),
    );
    assert!(commands.is_empty());
}

#[test]
fn a_database_list_without_the_selected_database_drops_the_selection() {
    let (state, a, _) = browsing_one();
    let (state, commands) = update(state, databases(a, &["other"]));
    assert!(commands.is_empty());
    assert_eq!(state.selected_ns(), None);
}

#[test]
fn a_database_list_with_the_selected_database_keeps_the_selection() {
    let (state, a, _) = browsing_one();
    let (state, _) = update(state, databases(a, &["app", "more"]));
    assert_eq!(state.selected_ns(), Some(&users()));
}

#[test]
fn a_stale_results_answer_changes_nothing() {
    let (state, a, _) = browsing_one();
    let shown = state.page.as_ref().map(|p| p.docs.len());
    let (state, _) = update(
        state,
        event(Event::Results {
            session: a,
            ns: users(),
            query: Query {
                skip: 50,
                ..Query::default()
            },
            page: page(people(9), 50, false),
            request: 0,
        }),
    );
    assert_eq!(state.page.as_ref().map(|p| p.docs.len()), shown);
}

#[test]
fn an_answer_for_a_closed_session_is_dropped() {
    let (state, a, _) = browsing_one();
    let (state, _) = update(state, Input::Disconnect(a));
    let (state, commands) = update(
        state,
        event(Event::Results {
            session: a,
            ns: users(),
            query: Query::default(),
            page: page(people(1), 0, false),
            request: 0,
        }),
    );
    assert!(commands.is_empty() && state.page.is_none() && state.open.is_empty());
}
