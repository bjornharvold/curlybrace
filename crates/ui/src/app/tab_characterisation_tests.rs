//! What the reducer does today with one view per session, recorded before the tab model is
//! introduced (#159, slice 1). The transcripts and the direct checks below are the proof that
//! the refactor changes nothing a person can see: they pass on the unchanged code and must pass,
//! unedited, after it.
//!
//! They pin, per session event that reaches a view: a lost connection, a reconnect, a
//! disconnect, a write answered (the page is re-fetched), a change of the database list, and the
//! stale answers every numbered read can receive.
use super::transcript_support::{Script, assert_golden, step, transcript};
use super::{Input, update};
use crate::app::state::{AppState, Banner, ConnStatus};
use crate::app::{AggregateAction, ExplainAction};
use crate::viewmodel::{
    CollectionTab, DocAction, EditorAction, FindAction, IndexesAction, SidebarAction,
};
use bson::{Document, doc};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::history::ChartRange;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Namespace, Page, Query};
use uuid::Uuid;

fn users() -> Namespace {
    Namespace::new("app", "users")
}

fn orders() -> Namespace {
    Namespace::new("app", "orders")
}

fn connections(names: &[&str]) -> (AppState, Vec<Uuid>) {
    let cfgs: Vec<_> = names
        .iter()
        .map(|n| ConnectionConfig::new(n, "mongodb://localhost"))
        .collect();
    let ids = cfgs.iter().map(|c| c.id).collect();
    (AppState::new(cfgs), ids)
}

fn page(docs: Vec<Document>, skip: u64, has_more: bool) -> Page {
    Page {
        docs,
        skip,
        has_more,
    }
}

fn people(n: i32) -> Vec<Document> {
    (1..=n)
        .map(|i| doc! { "_id": i, "name": format!("user {i}") })
        .collect()
}

/// The command last sent that `pick` recognises.
fn last<T>(cmds: &[Command], pick: impl Fn(&Command) -> Option<T>) -> T {
    cmds.iter()
        .rev()
        .find_map(pick)
        .expect("the script sent that command before answering it")
}

/// The newest find `session` sent, as (ns, query, request).
fn last_find_of(cmds: &[Command], session: Uuid) -> (Namespace, Query, u64) {
    last(cmds, |c| match c {
        Command::Find {
            session: s,
            ns,
            query,
            request,
        } if *s == session => Some((ns.clone(), query.clone(), *request)),
        _ => None,
    })
}

/// The answer to the newest find of `session`, with `docs`: it echoes the find's request.
fn found_for(session: Uuid, cmds: &[Command], docs: Vec<Document>, has_more: bool) -> Event {
    let (ns, query, request) = last_find_of(cmds, session);
    let skip = query.skip;
    Event::Results {
        session,
        ns,
        query,
        page: page(docs, skip, has_more),
        request,
    }
}

/// The answer to the newest find of the newest session, with `docs`.
fn found(cmds: &[Command], docs: Vec<Document>, has_more: bool) -> Event {
    let session = last(cmds, |c| match c {
        Command::Find { session, .. } => Some(*session),
        _ => None,
    });
    found_for(session, cmds, docs, has_more)
}

fn indexes_answer(cmds: &[Command]) -> Event {
    let (session, ns, request) = last(cmds, |c| match c {
        Command::ListIndexes {
            session,
            ns,
            request,
        } => Some((*session, ns.clone(), *request)),
        _ => None,
    });
    Event::Indexes {
        session,
        ns,
        indexes: Vec::new(),
        request,
    }
}

fn history_answer(cmds: &[Command]) -> Event {
    let (session, ns, request) = last(cmds, |c| match c {
        Command::IndexHistory {
            session,
            ns,
            request,
            ..
        } => Some((*session, ns.clone(), *request)),
        _ => None,
    });
    Event::IndexHistory {
        session,
        ns,
        range: ChartRange::Day,
        series: Vec::new(),
        request,
    }
}

fn previewed(cmds: &[Command]) -> Event {
    let (session, request) = last(cmds, |c| match c {
        Command::AggregatePreview {
            session, request, ..
        } => Some((*session, *request)),
        _ => None,
    });
    Event::AggregatePreviewed {
        session,
        request,
        outcome: Ok(vec![doc! { "n": 1 }]),
    }
}

fn aggregate_explained(cmds: &[Command]) -> Event {
    let (session, request, ns, verbosity) = last(cmds, |c| match c {
        Command::AggregateExplain {
            session,
            request,
            ns,
            verbosity,
            ..
        } => Some((*session, *request, ns.clone(), *verbosity)),
        _ => None,
    });
    Event::AggregateExplained {
        session,
        request,
        ns,
        verbosity,
        outcome: Err(CoreError::Database("no plan".into())),
    }
}

fn connect(id: Uuid) -> Input {
    Input::Connect(id)
}

fn connected(id: Uuid, state: &AppState) -> Input {
    Input::Event(Event::Connected {
        session: id,
        epoch: state.epoch_counter,
        topology: None,
    })
}

fn open(ns: Namespace) -> Input {
    Input::Sidebar(SidebarAction::Open(ns))
}

fn event(e: Event) -> Input {
    Input::Event(e)
}

fn databases(session: Uuid, names: &[&str]) -> Input {
    event(Event::Databases {
        session,
        names: names.iter().map(|n| n.to_string()).collect(),
    })
}

fn collections(session: Uuid, db: &str, names: &[&str]) -> Input {
    event(Event::Collections {
        session,
        db: db.into(),
        names: names.iter().map(|n| n.to_string()).collect(),
    })
}

fn lost(session: Uuid, context: &'static str) -> Input {
    event(Event::Failed {
        session,
        context,
        error: CoreError::Unreachable,
    })
}

/// A session that is connected, with `app` loaded and `app.users` open on its first page.
fn browsing_steps(id: Uuid) -> Script {
    vec![
        step("connect", move |_, _| connect(id)),
        step("connected", move |s, _| connected(id, s)),
        step("databases", move |_, _| databases(id, &["app", "other"])),
        step("expand app", |_, _| {
            Input::Sidebar(SidebarAction::LoadCollections("app".into()))
        }),
        step("collections", move |_, _| {
            collections(id, "app", &["users", "orders"])
        }),
        step("open users", |_, _| open(users())),
        step("first page", |_, c| event(found(c, people(3), true))),
    ]
}

#[test]
fn scripted_browse_session_transcript_is_unchanged() {
    let (start, ids) = connections(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let mut script = browsing_steps(a);
    script.extend(vec![
        step("run a filter", |_, _| {
            Input::Find(FindAction::Run(Query {
                filter: doc! { "name": "user 2" },
                ..Query::default()
            }))
        }),
        step("guard answer", move |_, c| {
            let (session, request) = last(c, |c| match c {
                Command::GuardCheck {
                    session, request, ..
                } => Some((*session, *request)),
                _ => None,
            });
            event(Event::GuardChecked {
                session,
                request,
                outcome: Err(CoreError::Database("no plan".into())),
            })
        }),
        step("filtered page", |_, c| event(found(c, people(1), true))),
        step("explain answer", |_, c| {
            let (session, ns, query, request) = last(c, |c| match c {
                Command::ExplainFind {
                    session,
                    ns,
                    query,
                    request,
                } => Some((*session, ns.clone(), query.clone(), *request)),
                _ => None,
            });
            event(Event::Explain {
                session,
                ns,
                query,
                outcome: Err(CoreError::Database("no plan".into())),
                request,
            })
        }),
        step("next page", |_, _| Input::Find(FindAction::Next)),
        step("next page answer", |_, c| event(found(c, people(1), false))),
        step("edit a document", |_, _| {
            Input::Doc(DocAction::Edit(doc! { "_id": 1, "name": "user 1" }))
        }),
        step("save it", |_, _| {
            Input::Editor(EditorAction::Save {
                original_id: Some(bson::Bson::Int32(1)),
                text: r#"{"_id":1,"name":"renamed"}"#.into(),
            })
        }),
        step("write answered", move |_, _| {
            event(Event::Mutated {
                session: a,
                ns: users(),
            })
        }),
        step("refetched page", |_, c| event(found(c, people(2), false))),
        step("close the editor", |_, _| {
            Input::Editor(EditorAction::Cancel)
        }),
        step("indexes tab", |_, _| Input::Tab(CollectionTab::Indexes)),
        step("indexes", |_, c| event(indexes_answer(c))),
        step("index history", |_, c| event(history_answer(c))),
        step("aggregate tab", |_, _| Input::Tab(CollectionTab::Aggregate)),
        step("add a stage", |_, _| {
            Input::Aggregate(AggregateAction::AddStage("$match".into()))
        }),
        step("previewed", |_, c| event(previewed(c))),
        step("explain the pipeline", |_, _| {
            Input::Aggregate(AggregateAction::Explain(ExplainAction::Run))
        }),
        step("pipeline explained", |_, c| event(aggregate_explained(c))),
        step("lost server", move |_, _| lost(a, "find")),
        step("reconnect", move |_, _| connect(a)),
        step("reconnected", move |s, _| connected(a, s)),
        step("a second session", move |_, _| connect(b)),
        step("second connected", move |s, _| connected(b, s)),
        step("second databases", move |_, _| databases(b, &["other"])),
        step("back to the first", move |_, _| Input::Activate(a)),
        step("disconnect it", move |_, _| Input::Disconnect(a)),
    ]);
    assert_golden("scripted_browse_session.txt", &transcript(start, script));
}

#[test]
fn background_session_transcript_is_unchanged() {
    let (start, ids) = connections(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let mut script = browsing_steps(b);
    script.extend(browsing_steps(a));
    script.extend(vec![
        step("focus the first", move |_, _| Input::Activate(b)),
        // From here on the second connection's session `a` is in the background.
        step("background collections", move |_, _| {
            collections(a, "other", &["x", "y"])
        }),
        step("background results", move |_, c| {
            event(found_for(a, c, people(2), false))
        }),
        step("background stale results", move |_, _| {
            event(Event::Results {
                session: a,
                ns: users(),
                query: Query {
                    skip: 50,
                    ..Query::default()
                },
                page: page(people(9), 50, false),
                request: 0,
            })
        }),
        step("background results of another collection", move |_, _| {
            event(Event::Results {
                session: a,
                ns: orders(),
                query: Query::default(),
                page: page(people(9), 0, false),
                request: 0,
            })
        }),
        step("background old indexes", move |_, _| {
            event(Event::Indexes {
                session: a,
                ns: users(),
                indexes: Vec::new(),
                request: 0,
            })
        }),
        step("background write answered", move |_, _| {
            event(Event::Mutated {
                session: a,
                ns: users(),
            })
        }),
        step("background database list", move |_, _| {
            databases(a, &["other"])
        }),
        step("background lost server", move |_, _| lost(a, "find")),
        step("background late answer", move |_, _| {
            event(Event::Results {
                session: a,
                ns: users(),
                query: Query::default(),
                page: page(people(1), 0, false),
                request: 0,
            })
        }),
    ]);
    assert_golden("background_session.txt", &transcript(start, script));
}

#[test]
fn stale_answers_transcript_is_unchanged() {
    let (start, ids) = connections(&["a"]);
    let a = ids[0];
    let mut script = browsing_steps(a);
    script.extend(vec![
        step("run twice", |_, _| {
            Input::Find(FindAction::Run(Query {
                filter: doc! { "n": 1 },
                ..Query::default()
            }))
        }),
        step("older results", move |_, _| {
            event(Event::Results {
                session: a,
                ns: users(),
                query: Query::default(),
                page: page(people(7), 0, false),
                request: 0,
            })
        }),
        step("indexes tab", |_, _| Input::Tab(CollectionTab::Indexes)),
        step("refresh the indexes", |_, _| {
            Input::Indexes(IndexesAction::Refresh)
        }),
        step("older indexes", move |_, _| {
            event(Event::Indexes {
                session: a,
                ns: users(),
                indexes: Vec::new(),
                request: 0,
            })
        }),
        step("aggregate tab", |_, _| Input::Tab(CollectionTab::Aggregate)),
        step("add a stage", |_, _| {
            Input::Aggregate(AggregateAction::AddStage("$match".into()))
        }),
        step("edit the stage", |_, _| {
            Input::Aggregate(AggregateAction::Edit(0, r#"{"$match":{"a":1}}"#.into()))
        }),
        step("older preview", move |_, _| {
            event(Event::AggregatePreviewed {
                session: a,
                request: 0,
                outcome: Ok(vec![doc! { "old": true }]),
            })
        }),
        step("a failed find", move |_, _| {
            event(Event::Failed {
                session: a,
                context: "find",
                error: CoreError::Database("boom".into()),
            })
        }),
        step("disconnect", move |_, _| Input::Disconnect(a)),
        step("answer after the disconnect", move |_, _| {
            event(Event::Results {
                session: a,
                ns: users(),
                query: Query::default(),
                page: page(people(1), 0, false),
                request: 0,
            })
        }),
        step("indexes after the disconnect", move |_, _| {
            event(Event::Indexes {
                session: a,
                ns: users(),
                indexes: Vec::new(),
                request: 99,
            })
        }),
    ]);
    assert_golden("stale_answers.txt", &transcript(start, script));
}

/// Connects `a` and opens `app.users` on a first page, through the reducer.
pub(super) fn browsing_one() -> (AppState, Uuid, Vec<Command>) {
    let (mut state, ids) = connections(&["a"]);
    let a = ids[0];
    let mut sent = Vec::new();
    for (_, make) in browsing_steps(a) {
        let input = make(&state, &sent);
        let (next, commands) = update(state, input);
        sent.extend(commands);
        state = next;
    }
    (state, a, sent)
}

#[test]
fn a_lost_connection_clears_the_view_and_keeps_the_aggregate_draft() {
    let (state, a, _) = browsing_one();
    let (state, _) = update(state, Input::Tab(CollectionTab::Aggregate));
    let (state, _) = update(
        state,
        Input::Aggregate(AggregateAction::AddStage("$match".into())),
    );
    let (state, _) = update(state, lost(a, "find"));
    assert_eq!(state.status, ConnStatus::Failed);
    assert_eq!(state.failure, Some(CoreError::Unreachable));
    assert_eq!(state.selected_ns(), None);
    assert!(state.page.is_none() && state.page_key.is_none());
    assert_eq!(state.tab, CollectionTab::Documents);
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
fn a_reconnect_keeps_the_draft_and_forgets_the_collection() {
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
    assert_eq!(state.selected_ns(), None);
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
