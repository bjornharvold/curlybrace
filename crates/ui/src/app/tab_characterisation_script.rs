//! The scripted steps and the answers to them that the characterisation tests share: a session
//! that browses (connect, open a collection, page, edit and save, Indexes, Aggregate), built
//! from the commands the reducer sent so every answer echoes the right request.
use super::transcript_support::{Script, step};
use super::{Input, update};
use crate::app::state::AppState;
use crate::viewmodel::SidebarAction;
use bson::{Document, doc};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::history::ChartRange;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Namespace, Page, Query};
use uuid::Uuid;

pub(super) fn users() -> Namespace {
    Namespace::new("app", "users")
}

pub(super) fn orders() -> Namespace {
    Namespace::new("app", "orders")
}

pub(super) fn connections(names: &[&str]) -> (AppState, Vec<Uuid>) {
    let cfgs: Vec<_> = names
        .iter()
        .map(|n| ConnectionConfig::new(n, "mongodb://localhost"))
        .collect();
    let ids = cfgs.iter().map(|c| c.id).collect();
    (AppState::new(cfgs), ids)
}

pub(super) fn page(docs: Vec<Document>, skip: u64, has_more: bool) -> Page {
    Page {
        docs,
        skip,
        has_more,
    }
}

pub(super) fn people(n: i32) -> Vec<Document> {
    (1..=n)
        .map(|i| doc! { "_id": i, "name": format!("user {i}") })
        .collect()
}

/// The command last sent that `pick` recognises.
pub(super) fn last<T>(cmds: &[Command], pick: impl Fn(&Command) -> Option<T>) -> T {
    cmds.iter()
        .rev()
        .find_map(pick)
        .expect("the script sent that command before answering it")
}

/// The newest find `session` sent, as (ns, query, request).
pub(super) fn last_find_of(cmds: &[Command], session: Uuid) -> (Namespace, Query, u64) {
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
pub(super) fn found_for(
    session: Uuid,
    cmds: &[Command],
    docs: Vec<Document>,
    has_more: bool,
) -> Event {
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
pub(super) fn found(cmds: &[Command], docs: Vec<Document>, has_more: bool) -> Event {
    let session = last(cmds, |c| match c {
        Command::Find { session, .. } => Some(*session),
        _ => None,
    });
    found_for(session, cmds, docs, has_more)
}

pub(super) fn indexes_answer(cmds: &[Command]) -> Event {
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

pub(super) fn history_answer(cmds: &[Command]) -> Event {
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

pub(super) fn previewed(cmds: &[Command]) -> Event {
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

pub(super) fn aggregate_explained(cmds: &[Command]) -> Event {
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

pub(super) fn connect(id: Uuid) -> Input {
    Input::Connect(id)
}

pub(super) fn connected(id: Uuid, state: &AppState) -> Input {
    Input::Event(Event::Connected {
        session: id,
        epoch: state.epoch_counter,
        topology: None,
    })
}

pub(super) fn open(ns: Namespace) -> Input {
    Input::Sidebar(SidebarAction::Open(ns))
}

pub(super) fn event(e: Event) -> Input {
    Input::Event(e)
}

pub(super) fn databases(session: Uuid, names: &[&str]) -> Input {
    event(Event::Databases {
        session,
        names: names.iter().map(|n| n.to_string()).collect(),
    })
}

pub(super) fn collections(session: Uuid, db: &str, names: &[&str]) -> Input {
    event(Event::Collections {
        session,
        db: db.into(),
        names: names.iter().map(|n| n.to_string()).collect(),
    })
}

pub(super) fn lost(session: Uuid, context: &'static str) -> Input {
    event(Event::Failed {
        session,
        context,
        error: CoreError::Unreachable,
    })
}

/// A session that is connected, with `app` loaded and `app.users` open on its first page.
pub(super) fn browsing_steps(id: Uuid) -> Script {
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

/// Connects `a` and opens `app.users` on a first page, through the reducer.
pub(crate) fn browsing_one() -> (AppState, Uuid, Vec<Command>) {
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
