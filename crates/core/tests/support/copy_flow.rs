//! Helpers for the copy integration tests: connect sessions, drive `Command::Copy` to its
//! terminal event, and seed / read back collections through the driver.
//! Document values and `_id`s are never printed by these helpers.
#![allow(dead_code)]

use std::time::Duration;

use bson::{Document, doc};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::copy::{
    ConflictPolicy, CopyCommand, CopyEvent, CopyItem, CopyReport, CopyRequest, CopyScope,
};
use curlybrace_core::runtime::{Command, Event, RuntimeHandle, SessionId};
use curlybrace_core::store::Namespace;
use futures::TryStreamExt;
use mongodb::Client;

use super::{next_event, no_repaint};

const LIMIT: Duration = Duration::from_secs(60);

/// A runtime wired to the real driver, with no sessions open yet.
pub fn runtime() -> RuntimeHandle {
    curlybrace_core::runtime::spawn_runtime_with_config(
        None,
        None,
        no_repaint(),
        curlybrace_core::runtime::RuntimeConfig::default(),
    )
}

/// Opens the session of `cfg` and waits until it is connected; returns its id.
pub async fn connect(rt: &mut RuntimeHandle, cfg: &ConnectionConfig) -> SessionId {
    rt.tx.send(Command::Connect(cfg.clone())).unwrap();
    loop {
        match next_event(&mut rt.rx, LIMIT, "connect").await {
            Event::Connected { session, .. } if session == cfg.id => return session,
            Event::ConnectFailed { .. } => panic!("connect failed"),
            _ => {}
        }
    }
}

pub fn item(source: &Namespace, target: &Namespace) -> CopyItem {
    CopyItem {
        source: source.clone(),
        target: target.clone(),
        filter: doc! {},
    }
}

pub fn request(
    source: SessionId,
    target: SessionId,
    scope: CopyScope,
    policy: ConflictPolicy,
) -> CopyRequest {
    CopyRequest {
        source,
        target,
        scope,
        policy,
        with_indexes_and_options: false,
    }
}

pub fn start(rt: &RuntimeHandle, run: u64, request: CopyRequest) {
    rt.tx
        .send(Command::Copy(CopyCommand::Start { run, request }))
        .unwrap();
}

pub fn cancel(rt: &RuntimeHandle, run: u64) {
    rt.tx
        .send(Command::Copy(CopyCommand::Cancel { run }))
        .unwrap();
}

/// The next copy event; other runtime events are skipped.
pub async fn next_copy(rt: &mut RuntimeHandle) -> CopyEvent {
    loop {
        if let Event::Copy(e) = next_event(&mut rt.rx, LIMIT, "a copy event").await {
            return e;
        }
    }
}

pub fn is_terminal(event: &CopyEvent) -> bool {
    matches!(
        event,
        CopyEvent::Done { .. } | CopyEvent::Failed { .. } | CopyEvent::PreviewDone { .. }
    )
}

/// Every event of a run, up to and including its terminal one.
pub async fn collect(rt: &mut RuntimeHandle) -> Vec<CopyEvent> {
    let mut events = Vec::new();
    loop {
        let event = next_copy(rt).await;
        let last = is_terminal(&event);
        events.push(event);
        if last {
            return events;
        }
    }
}

/// Starts a run and collects all its events.
pub async fn run_copy(rt: &mut RuntimeHandle, run: u64, req: CopyRequest) -> Vec<CopyEvent> {
    start(rt, run, req);
    collect(rt).await
}

/// The reports of the run's `ItemDone` events, in order.
pub fn reports(events: &[CopyEvent]) -> Vec<CopyReport> {
    events
        .iter()
        .filter_map(|e| match e {
            CopyEvent::ItemDone { report, .. } => Some(report.clone()),
            _ => None,
        })
        .collect()
}

pub fn has_planned(events: &[CopyEvent]) -> bool {
    events
        .iter()
        .any(|e| matches!(e, CopyEvent::Planned { .. }))
}

/// Inserts `docs` into `ns` through the driver (unordered, so one bad document cannot hide
/// the rest).
pub async fn seed(client: &Client, ns: &Namespace, docs: Vec<Document>) {
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .insert_many(docs)
        .ordered(false)
        .await
        .expect("seed");
}

/// Every document of `ns` through the driver, in natural order.
pub async fn all_docs(client: &Client, ns: &Namespace) -> Vec<Document> {
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .find(doc! {})
        .await
        .expect("find")
        .try_collect()
        .await
        .expect("collect")
}

pub async fn count(client: &Client, ns: &Namespace) -> u64 {
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .count_documents(doc! {})
        .await
        .expect("count")
}

/// The document of `ns` with this `_id`, if any.
pub async fn by_id(client: &Client, ns: &Namespace, id: &bson::Bson) -> Option<Document> {
    client
        .database(&ns.db)
        .collection::<Document>(&ns.collection)
        .find_one(doc! { "_id": id.clone() })
        .await
        .expect("find_one")
}

/// A document with a small integer id and a text value.
pub fn numbered(id: i32, v: &str) -> Document {
    doc! { "_id": id, "v": v }
}

/// `_id` -> `v` for ids `from..=to`.
pub fn numbered_range(from: i32, to: i32, v: &str) -> Vec<Document> {
    (from..=to).map(|id| numbered(id, v)).collect()
}

/// The `(id, v)` pairs of `docs` as sorted `(i32, String)`, for compact assertions.
pub fn id_v(docs: &[Document]) -> Vec<(i32, String)> {
    let mut out: Vec<(i32, String)> = docs
        .iter()
        .map(|d| {
            (
                d.get_i32("_id").expect("i32 id"),
                d.get_str("v").expect("v").to_string(),
            )
        })
        .collect();
    out.sort();
    out
}

pub async fn db_names(client: &Client) -> Vec<String> {
    client.list_database_names().await.expect("list databases")
}

pub async fn collection_names(client: &Client, db: &str) -> Vec<String> {
    let mut names = client
        .database(db)
        .list_collection_names()
        .await
        .expect("list collections");
    names.sort();
    names
}
