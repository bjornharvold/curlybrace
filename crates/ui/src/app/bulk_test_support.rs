//! Shared fixtures of the bulk reducer tests: a browsing session and helpers that drive a flow
//! to each of its steps.
pub(super) use super::bulk_actions::BulkAction;
pub(super) use super::bulk_state::BulkState;
pub(super) use super::reducer::{Input, update};
pub(super) use super::state::{AppState, Banner, ConnStatus, Pending};
pub(super) use crate::viewmodel::{
    BulkFlow, BulkKind, BulkStep, CancelNote, CountState, PreviewState,
};
pub(super) use bson::doc;
pub(super) use curlybrace_core::bulk::{
    BulkOutcome, BulkRunId, CancelEffect, MatchScope, PreviewDoc, UpdatePreview,
};
pub(super) use curlybrace_core::connection::ConnectionConfig;
pub(super) use curlybrace_core::error::CoreError;
pub(super) use curlybrace_core::runtime::{Command, Event};
pub(super) use curlybrace_core::store::{Namespace, Page, Query};
pub(super) use curlybrace_core::test_support::mock_objects::mock_connection;
pub(super) use uuid::Uuid;

pub(super) const GOOD_UPDATE: &str = r#"{"$set": {"a": 1}}"#;

pub(super) fn ns() -> Namespace {
    Namespace::new("shop", "orders")
}

pub(super) fn config(production: bool, read_only: bool) -> ConnectionConfig {
    ConnectionConfig {
        production,
        read_only,
        ..mock_connection()
    }
}

/// `cfg` as the active, connected session with `shop.orders` on screen, filtered, sorted, paged.
pub(super) fn browsing(cfg: ConnectionConfig) -> (AppState, Uuid) {
    let id = cfg.id;
    let (s, _) = update(AppState::new(vec![cfg]), Input::Connect(id));
    let epoch = s.epoch_counter;
    let (mut s, _) = update(
        s,
        Input::Event(Event::Connected {
            session: id,
            epoch,
            topology: None,
        }),
    );
    s.sidebar = s
        .sidebar
        .with_databases(vec!["shop".into()])
        .with_collections("shop", vec!["orders".into()])
        .selecting(ns());
    s.page = Some(Page {
        docs: vec![doc! { "_id": 1 }],
        skip: 20,
        has_more: true,
    });
    // Undo is on: the runtime was given a folder for the copies.
    s.snapshot_dir = Some(std::path::PathBuf::from("/snapshots"));
    s.page_query = Query {
        filter: doc! { "status": "old" },
        sort: Some(doc! { "_id": -1 }),
        projection: None,
        skip: 20,
        limit: 5,
    };
    (s, id)
}

pub(super) fn plain() -> (AppState, Uuid) {
    browsing(config(false, false))
}

pub(super) fn act(s: AppState, a: BulkAction) -> (AppState, Vec<Command>) {
    update(s, Input::Bulk(a))
}

pub(super) fn ev(s: AppState, e: Event) -> (AppState, Vec<Command>) {
    update(s, Input::Event(e))
}

pub(super) fn flow(s: &AppState) -> &BulkFlow {
    &s.bulk.as_ref().expect("a bulk flow is open").flow
}

pub(super) fn count_id(cmds: &[Command]) -> u64 {
    cmds.iter()
        .find_map(|c| match c {
            Command::BulkCount { request, .. } => Some(*request),
            _ => None,
        })
        .expect("a BulkCount was sent")
}

pub(super) fn preview_id(cmds: &[Command]) -> u64 {
    cmds.iter()
        .find_map(|c| match c {
            Command::BulkPreview { request_id, .. } => Some(*request_id),
            Command::BulkSample { request, .. } => Some(*request),
            _ => None,
        })
        .expect("a preview or sample request was sent")
}

pub(super) fn count_answer(id: Uuid, request: u64, count: Option<u64>) -> Event {
    Event::BulkCount {
        session: id,
        ns: ns(),
        request,
        count,
    }
}

pub(super) fn preview_answer(id: Uuid, kind: BulkKind, request: u64) -> Event {
    match kind {
        BulkKind::Delete => Event::BulkSample {
            session: id,
            ns: ns(),
            request,
            docs: vec![doc! { "_id": 1 }],
        },
        BulkKind::Update => Event::BulkPreview {
            session: id,
            ns: ns(),
            request_id: request,
            outcome: Ok(UpdatePreview {
                docs: vec![PreviewDoc {
                    before: doc! { "_id": 1, "a": 0 },
                    after: Some(doc! { "_id": 1, "a": 1 }),
                }],
                gap: None,
            }),
        },
    }
}

/// A flow in Define whose count answered 5.
pub(super) fn defining(kind: BulkKind, cfg: ConnectionConfig) -> (AppState, Uuid) {
    let (s, id) = browsing(cfg);
    let (s, cmds) = act(s, BulkAction::Open(kind));
    let (s, _) = ev(s, count_answer(id, count_id(&cmds), Some(5)));
    let s = match kind {
        BulkKind::Update => act(s, BulkAction::SetUpdateText(GOOD_UPDATE.into())).0,
        BulkKind::Delete => s,
    };
    (s, id)
}

/// A flow in Preview with its preview answered.
pub(super) fn previewing(kind: BulkKind, cfg: ConnectionConfig) -> (AppState, Uuid) {
    let (s, id) = defining(kind, cfg);
    let (s, cmds) = act(s, BulkAction::Next);
    let (s, _) = ev(s, preview_answer(id, kind, preview_id(&cmds)));
    (s, id)
}

/// A flow in Confirm.
pub(super) fn confirming(kind: BulkKind, cfg: ConnectionConfig) -> (AppState, Uuid) {
    let (s, id) = previewing(kind, cfg);
    let (s, _) = act(s, BulkAction::Next);
    assert_eq!(flow(&s).step, BulkStep::Confirm);
    (s, id)
}

/// A delete in Confirm with the collection name typed: ready to run.
pub(super) fn ready_delete(cfg: ConnectionConfig) -> (AppState, Uuid) {
    let (s, id) = confirming(BulkKind::Delete, cfg);
    (act(s, BulkAction::SetTyped("orders".into())).0, id)
}

pub(super) fn running_delete() -> (AppState, Uuid, BulkRunId) {
    let (s, id) = ready_delete(config(false, false));
    let (s, cmds) = act(s, BulkAction::Run);
    let run = match &cmds[..] {
        [Command::BulkDelete { run, .. }] => *run,
        other => panic!("expected one BulkDelete, got {other:?}"),
    };
    (s, id, run)
}

pub(super) fn finish(id: Uuid, run: BulkRunId, outcome: Result<BulkOutcome, CoreError>) -> Event {
    Event::BulkFinished {
        session: id,
        ns: ns(),
        run,
        outcome,
        undo: curlybrace_core::bulk::UndoStatus::NotRequested,
    }
}

pub(super) fn applied() -> BulkOutcome {
    BulkOutcome::Applied {
        matched: 5,
        modified: 0,
        upserted: Vec::new(),
        deleted: 5,
    }
}

pub(super) fn is_find(c: &Command) -> bool {
    matches!(c, Command::Find { .. })
}

/// `RequestFailed` of a numbered bulk read, as the runtime sends it.
pub(super) fn read_failed(id: Uuid, context: &'static str, request: u64) -> Event {
    Event::RequestFailed {
        session: id,
        context,
        request,
        error: CoreError::Database("failed".into()),
    }
}

/// `Failed` of a bulk command that has no request id.
pub(super) fn failed(id: Uuid, context: &'static str, error: CoreError) -> Event {
    Event::Failed {
        session: id,
        context,
        error,
    }
}
