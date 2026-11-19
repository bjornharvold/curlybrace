//! What the reducer does today with one view per session, recorded before the tab model is
//! introduced (#159, slice 1). The transcripts and the direct checks below are the proof that
//! the refactor changes nothing a person can see: they pass on the unchanged code and must pass,
//! unedited, after it.
//!
//! They pin, per session event that reaches a view: a lost connection, a reconnect, a
//! disconnect, a write answered (the page is re-fetched), a change of the database list, and the
//! stale answers every numbered read can receive.
use super::Input;
use super::tab_characterisation_script::*;
use super::transcript_support::{assert_golden, step, transcript};
use crate::app::{AggregateAction, ExplainAction};
use crate::viewmodel::{CollectionTab, DocAction, EditorAction, FindAction, IndexesAction};
use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::Query;

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
        // The run is held back by the guard, so the newest find is still the one that opened the
        // collection: its answer (echoing its own number) is the awaited one and is applied.
        step("older results", move |_, c| {
            event(found_for(a, c, people(7), false))
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
