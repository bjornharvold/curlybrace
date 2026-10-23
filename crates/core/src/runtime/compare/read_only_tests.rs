//! Read-only connections in Data Compare: a read-only side may be the source but never the
//! target, and a lock reaches the open compare stores, even one sent mid-open.

use std::collections::HashMap;
use std::sync::Arc;

use bson::Bson;
use uuid::Uuid;

use super::apply_tests::{ApplyStore, op, send_apply, source_store, target_store, within};
use super::closed_tests::Hooks;
use super::export_tests::{collect_export, request, send_export};
use super::tests::{
    MapConnector, Tx, collect_run, next_compare, same_pair, send_start, start_with,
};
use crate::compare::apply::ApplyKind;
use crate::compare::protocol::{CompareCommand, CompareEvent, ExportFormat, PairOutcome};
use crate::connection::ConnectionConfig;
use crate::error::CoreError;
use crate::runtime::{Command, Event, RuntimeConfig, RuntimeHandle};
use crate::store::DocumentStore;

const SRC: &str = "mongodb://a";
const TGT: &str = "mongodb://b";

fn side(uri: &str, read_only: bool) -> ConnectionConfig {
    ConnectionConfig {
        read_only,
        ..ConnectionConfig::new(uri, uri)
    }
}

struct Rig {
    tx: Tx,
    h: RuntimeHandle,
    source: Arc<ApplyStore>,
    target: Arc<ApplyStore>,
}

fn connector(source: &Arc<ApplyStore>, target: &Arc<ApplyStore>) -> MapConnector {
    MapConnector {
        stores: HashMap::from([
            (SRC.to_string(), source.clone() as Arc<dyn DocumentStore>),
            (TGT.to_string(), target.clone() as Arc<dyn DocumentStore>),
        ]),
        ..MapConnector::default()
    }
}

fn rig_with(connector: MapConnector, source: Arc<ApplyStore>, target: Arc<ApplyStore>) -> Rig {
    let (tx, h) = start_with(connector, RuntimeConfig::default());
    Rig {
        tx,
        h,
        source,
        target,
    }
}

fn rig() -> Rig {
    let (source, target) = (
        ApplyStore::new(source_store()),
        ApplyStore::new(target_store()),
    );
    rig_with(connector(&source, &target), source, target)
}

fn send_open(tx: &Tx, source: &ConnectionConfig, target: &ConnectionConfig) {
    tx.send(Command::Compare(CompareCommand::Open {
        session: 1,
        source: source.clone(),
        target: target.clone(),
    }))
    .unwrap();
}

fn send_lock(tx: &Tx, connection: Uuid) {
    tx.send(Command::LockReadOnly { connection }).unwrap();
}

async fn open(r: &mut Rig, source: &ConnectionConfig, target: &ConnectionConfig) {
    send_open(&r.tx, source, target);
    assert_eq!(
        next_compare(&mut r.h).await,
        CompareEvent::Opened { session: 1 }
    );
}

async fn run_to_completion(r: &mut Rig) {
    send_start(&r.tx, 1, vec![same_pair(1)]);
    let done = collect_run(&mut r.h).await;
    assert!(
        done.iter()
            .all(|(_, o)| matches!(o, PairOutcome::Completed(_))),
        "{done:?}"
    );
}

async fn assert_apply_refused(r: &mut Rig) {
    send_apply(&r.tx, 5, vec![op(1, 1, ApplyKind::Insert)]);
    match next_compare(&mut r.h).await {
        CompareEvent::Failed {
            session: 1,
            context: "compare_apply",
            error,
        } => assert_eq!(error, CoreError::ReadOnly),
        other => panic!("{other:?}"),
    }
    assert_eq!(r.target.writes(), 0);
}

#[tokio::test]
async fn open_with_read_only_source_succeeds_and_reads() {
    let mut r = rig();
    open(&mut r, &side(SRC, true), &side(TGT, false)).await;
    run_to_completion(&mut r).await;
}

#[tokio::test]
async fn open_with_read_only_target_fails_with_read_only() {
    let mut r = rig();
    send_open(&r.tx, &side(SRC, false), &side(TGT, true));
    assert_eq!(
        next_compare(&mut r.h).await,
        CompareEvent::OpenFailed {
            session: 1,
            error: CoreError::ReadOnly
        }
    );
}

#[tokio::test]
async fn apply_after_target_is_locked_is_refused_with_context_compare_apply() {
    let mut r = rig();
    let target = side(TGT, false);
    open(&mut r, &side(SRC, false), &target).await;
    run_to_completion(&mut r).await;
    send_lock(&r.tx, target.id);
    assert_apply_refused(&mut r).await;
}

#[tokio::test]
async fn lock_during_in_flight_compare_open_yields_a_locked_target() {
    let hooks = Arc::new(Hooks::default());
    let (source, target) = (
        ApplyStore::new(source_store()),
        ApplyStore::new(target_store()),
    );
    let mut conn = connector(&source, &target);
    conn.parked.insert(TGT.to_string(), hooks.clone());
    let mut r = rig_with(conn, source, target);
    let target_cfg = side(TGT, false);
    send_open(&r.tx, &side(SRC, false), &target_cfg);
    send_lock(&r.tx, target_cfg.id);
    hooks.entered.notified().await;
    // Both commands are already dispatched while the open is still blocked.
    hooks.release.notify_one();
    assert_eq!(
        next_compare(&mut r.h).await,
        CompareEvent::Opened { session: 1 }
    );
    run_to_completion(&mut r).await;
    assert_apply_refused(&mut r).await;
}

#[tokio::test]
async fn lock_during_in_flight_compare_open_keeps_a_locked_source_readable() {
    let hooks = Arc::new(Hooks::default());
    let (source, target) = (
        ApplyStore::new(source_store()),
        ApplyStore::new(target_store()),
    );
    let mut conn = connector(&source, &target);
    conn.parked.insert(SRC.to_string(), hooks.clone());
    let mut r = rig_with(conn, source, target);
    let source_cfg = side(SRC, false);
    send_open(&r.tx, &source_cfg, &side(TGT, false));
    send_lock(&r.tx, source_cfg.id);
    hooks.entered.notified().await;
    hooks.release.notify_one();
    assert_eq!(
        next_compare(&mut r.h).await,
        CompareEvent::Opened { session: 1 }
    );
    run_to_completion(&mut r).await;
    assert_eq!(r.source.writes(), 0);
}

#[tokio::test]
async fn one_lock_reaches_data_session_and_compare_store() {
    let mut r = rig();
    let saved = side(TGT, false);
    r.tx.send(Command::Connect(saved.clone())).unwrap();
    let connected = within(r.h.rx.recv()).await.expect("channel open");
    assert!(
        matches!(connected, Event::Connected { .. }),
        "{connected:?}"
    );
    open(&mut r, &side(SRC, false), &saved).await;
    run_to_completion(&mut r).await;
    send_lock(&r.tx, saved.id);
    assert_apply_refused(&mut r).await;
    r.tx.send(Command::Delete {
        session: saved.id,
        ns: crate::runtime::compare::tests::ns("d", "c"),
        id: Bson::Int32(2),
    })
    .unwrap();
    let event = within(r.h.rx.recv()).await.expect("channel open");
    assert!(
        matches!(
            event,
            Event::Failed {
                error: CoreError::ReadOnly,
                ..
            }
        ),
        "{event:?}"
    );
    assert_eq!(r.target.writes(), 0);
}

#[tokio::test]
async fn export_to_file_still_works_for_a_read_only_source() {
    let mut r = rig();
    open(&mut r, &side(SRC, true), &side(TGT, false)).await;
    run_to_completion(&mut r).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.js");
    let ops = vec![op(1, 1, ApplyKind::Insert)];
    send_export(&r.tx, 6, request(ExportFormat::Mongosh, ops), &path);
    let events = within(collect_export(&mut r.h)).await;
    assert!(
        matches!(events.last(), Some(CompareEvent::ExportDone { .. })),
        "{events:?}"
    );
    assert!(path.exists());
}
