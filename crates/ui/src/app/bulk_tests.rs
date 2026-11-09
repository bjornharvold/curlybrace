//! The bulk flow in the reducer: opening and gating, the requests it numbers and the answers it
//! trusts, the write through the guard, and what happens when the session changes under it.
use super::*;
use crate::app::bulk_state::BulkState;
use crate::app::reducer::{Input, update};
use crate::app::state::{Banner, ConnStatus, Pending};
use crate::viewmodel::{BulkStep, CountState, PreviewState};
use bson::doc;
use curlybrace_core::bulk::{MatchScope, PreviewDoc};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Page, Query};
use curlybrace_core::test_support::mock_objects::mock_connection;

const GOOD_UPDATE: &str = r#"{"$set": {"a": 1}}"#;

fn ns() -> Namespace {
    Namespace::new("shop", "orders")
}

fn config(production: bool, read_only: bool) -> ConnectionConfig {
    ConnectionConfig {
        production,
        read_only,
        ..mock_connection()
    }
}

/// `cfg` as the active, connected session with `shop.orders` on screen, filtered, sorted, paged.
fn browsing(cfg: ConnectionConfig) -> (AppState, Uuid) {
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
    s.page_query = Query {
        filter: doc! { "status": "old" },
        sort: Some(doc! { "_id": -1 }),
        projection: None,
        skip: 20,
        limit: 5,
    };
    (s, id)
}

fn plain() -> (AppState, Uuid) {
    browsing(config(false, false))
}

fn act(s: AppState, a: BulkAction) -> (AppState, Vec<Command>) {
    update(s, Input::Bulk(a))
}

fn ev(s: AppState, e: Event) -> (AppState, Vec<Command>) {
    update(s, Input::Event(e))
}

fn flow(s: &AppState) -> &crate::viewmodel::BulkFlow {
    &s.bulk.as_ref().expect("a bulk flow is open").flow
}

fn count_id(cmds: &[Command]) -> u64 {
    cmds.iter()
        .find_map(|c| match c {
            Command::BulkCount { request, .. } => Some(*request),
            _ => None,
        })
        .expect("a BulkCount was sent")
}

fn preview_id(cmds: &[Command]) -> u64 {
    cmds.iter()
        .find_map(|c| match c {
            Command::BulkPreview { request_id, .. } => Some(*request_id),
            Command::BulkSample { request, .. } => Some(*request),
            _ => None,
        })
        .expect("a preview or sample request was sent")
}

fn count_answer(id: Uuid, request: u64, count: Option<u64>) -> Event {
    Event::BulkCount {
        session: id,
        ns: ns(),
        request,
        count,
    }
}

fn preview_answer(id: Uuid, kind: BulkKind, request: u64) -> Event {
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
fn defining(kind: BulkKind, cfg: ConnectionConfig) -> (AppState, Uuid) {
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
fn previewing(kind: BulkKind, cfg: ConnectionConfig) -> (AppState, Uuid) {
    let (s, id) = defining(kind, cfg);
    let (s, cmds) = act(s, BulkAction::Next);
    let (s, _) = ev(s, preview_answer(id, kind, preview_id(&cmds)));
    (s, id)
}

/// A flow in Confirm.
fn confirming(kind: BulkKind, cfg: ConnectionConfig) -> (AppState, Uuid) {
    let (s, id) = previewing(kind, cfg);
    let (s, _) = act(s, BulkAction::Next);
    assert_eq!(flow(&s).step, BulkStep::Confirm);
    (s, id)
}

/// A delete in Confirm with the collection name typed: ready to run.
fn ready_delete(cfg: ConnectionConfig) -> (AppState, Uuid) {
    let (s, id) = confirming(BulkKind::Delete, cfg);
    (act(s, BulkAction::SetTyped("orders".into())).0, id)
}

fn running_delete() -> (AppState, Uuid, BulkRunId) {
    let (s, id) = ready_delete(config(false, false));
    let (s, cmds) = act(s, BulkAction::Run);
    let run = match &cmds[..] {
        [Command::BulkDelete { run, .. }] => *run,
        other => panic!("expected one BulkDelete, got {other:?}"),
    };
    (s, id, run)
}

fn finish(id: Uuid, run: BulkRunId, outcome: Result<BulkOutcome, CoreError>) -> Event {
    Event::BulkFinished {
        session: id,
        ns: ns(),
        run,
        outcome,
    }
}

fn applied() -> BulkOutcome {
    BulkOutcome::Applied {
        matched: 5,
        modified: 0,
        upserted: Vec::new(),
        deleted: 5,
    }
}

fn is_find(c: &Command) -> bool {
    matches!(c, Command::Find { .. })
}

// ---------------------------------------------------------------- open

#[test]
fn open_is_refused_on_read_only_with_the_banner() {
    let (s, _) = browsing(config(false, true));
    let (s, cmds) = act(s, BulkAction::Open(BulkKind::Delete));
    assert!(s.bulk.is_none());
    assert_eq!(s.banner, Some(Banner::ReadOnly));
    assert!(cmds.is_empty());
}

#[test]
fn open_does_nothing_on_a_projected_page() {
    let (mut s, _) = plain();
    s.page_query.projection = Some(doc! { "a": 1 });
    let (s, cmds) = act(s, BulkAction::Open(BulkKind::Update));
    assert!(s.bulk.is_none());
    assert!(s.banner.is_none());
    assert!(cmds.is_empty());
}

#[test]
fn open_does_nothing_without_a_selected_collection() {
    let (mut s, _) = plain();
    s.sidebar = s.sidebar.with_databases(vec!["shop".into()]);
    s.sidebar.selected = None;
    let (s, cmds) = act(s, BulkAction::Open(BulkKind::Update));
    assert!(s.bulk.is_none());
    assert!(cmds.is_empty());
}

#[test]
fn open_does_nothing_while_a_write_is_in_flight_or_parked() {
    let (mut s, _) = plain();
    s.pending = Some(Pending {
        ns: ns(),
        editor: None,
    });
    let (s, cmds) = act(s, BulkAction::Open(BulkKind::Delete));
    assert!(s.bulk.is_none() && cmds.is_empty());

    let (s, _) = browsing(config(true, false));
    let (s, _) = update(
        s,
        Input::Editor(crate::viewmodel::EditorAction::Save {
            original_id: None,
            text: "{}".into(),
        }),
    );
    assert!(s.write_warning.is_some());
    let (s, cmds) = act(s, BulkAction::Open(BulkKind::Delete));
    assert!(s.bulk.is_none() && cmds.is_empty());
}

#[test]
fn open_is_refused_while_another_bulk_flow_is_open() {
    let (s, _) = plain();
    let (s, _) = act(s, BulkAction::Open(BulkKind::Delete));
    let counter = s.bulk_requests;
    let (s, cmds) = act(s, BulkAction::Open(BulkKind::Update));
    assert_eq!(flow(&s).kind, BulkKind::Delete);
    assert_eq!(s.bulk_requests, counter);
    assert!(cmds.is_empty());
}

#[test]
fn open_uses_the_filter_and_ignores_sort_skip_and_limit() {
    let (s, _) = plain();
    let (s, cmds) = act(s, BulkAction::Open(BulkKind::Update));
    assert_eq!(flow(&s).filter, doc! { "status": "old" });
    assert_eq!(flow(&s).ns, ns());
    assert!(matches!(
        &cmds[..],
        [Command::BulkCount { filter, .. }] if *filter == doc! { "status": "old" }
    ));
}

#[test]
fn open_sends_count_for_the_filter() {
    let (s, id) = plain();
    let (s, cmds) = act(s, BulkAction::Open(BulkKind::Delete));
    let request = count_id(&cmds);
    assert!(matches!(
        &cmds[..],
        [Command::BulkCount { session, ns: n, .. }] if *session == id && *n == ns()
    ));
    let bulk = s.bulk.as_ref().unwrap();
    assert_eq!(bulk.count_request, Some(request));
    assert_eq!(bulk.session, id);
    assert_eq!(flow(&s).count, CountState::Pending);
}

#[test]
fn open_records_whether_the_connection_is_production() {
    let (s, _) = browsing(config(true, false));
    let (s, _) = act(s, BulkAction::Open(BulkKind::Update));
    assert!(flow(&s).production);
    let (s, _) = plain();
    let (s, _) = act(s, BulkAction::Open(BulkKind::Update));
    assert!(!flow(&s).production);
}

// ---------------------------------------------------------------- edits and steps

#[test]
fn edits_reach_the_flow() {
    let (s, _) = defining(BulkKind::Update, config(false, false));
    let (s, _) = act(s, BulkAction::SetUpsert(true));
    assert!(flow(&s).upsert);
    assert_eq!(flow(&s).update_text, GOOD_UPDATE);
    let (s, _) = act(s, BulkAction::SetAllDocuments(true));
    assert!(flow(&s).all_documents_ticked);
}

#[test]
fn next_from_define_sends_bulk_preview_for_update() {
    let (s, id) = defining(BulkKind::Update, config(false, false));
    let (s, _) = act(s, BulkAction::SetUpsert(true));
    let (s, cmds) = act(s, BulkAction::Next);
    assert_eq!(flow(&s).step, BulkStep::Preview);
    let [
        Command::BulkPreview {
            session,
            ns: n,
            request,
            request_id,
        },
    ] = &cmds[..]
    else {
        panic!("expected one BulkPreview, got {cmds:?}");
    };
    assert_eq!((*session, n), (id, &ns()));
    assert_eq!(request.filter, doc! { "status": "old" });
    assert!(request.upsert);
    assert_eq!(request.scope, MatchScope::Filtered);
    assert_eq!(s.bulk.as_ref().unwrap().preview_request, Some(*request_id));
}

#[test]
fn next_from_define_previews_an_empty_filter_over_all_documents() {
    let (mut s, id) = browsing(config(false, false));
    s.page_query.filter = doc! {};
    let (s, cmds) = act(s, BulkAction::Open(BulkKind::Update));
    let (s, _) = ev(s, count_answer(id, count_id(&cmds), Some(5)));
    let (s, _) = act(s, BulkAction::SetUpdateText(GOOD_UPDATE.into()));
    let (_, cmds) = act(s, BulkAction::Next);
    assert!(matches!(
        &cmds[..],
        [Command::BulkPreview { request, .. }] if request.scope == MatchScope::AllDocuments
    ));
}

#[test]
fn next_from_define_with_an_invalid_update_sends_nothing() {
    let (s, _) = defining(BulkKind::Update, config(false, false));
    let (s, _) = act(s, BulkAction::SetUpdateText("not json".into()));
    let (s, cmds) = act(s, BulkAction::Next);
    assert_eq!(flow(&s).step, BulkStep::Define);
    assert!(cmds.is_empty());
    assert_eq!(s.bulk.as_ref().unwrap().preview_request, None);
}

#[test]
fn next_from_define_sends_bulk_sample_for_delete() {
    let (s, id) = defining(BulkKind::Delete, config(false, false));
    let (s, cmds) = act(s, BulkAction::Next);
    assert_eq!(flow(&s).step, BulkStep::Preview);
    let [
        Command::BulkSample {
            session,
            ns: n,
            filter,
            request,
        },
    ] = &cmds[..]
    else {
        panic!("expected one BulkSample, got {cmds:?}");
    };
    assert_eq!((*session, n), (id, &ns()));
    assert_eq!(*filter, doc! { "status": "old" });
    assert_eq!(s.bulk.as_ref().unwrap().preview_request, Some(*request));
}

#[test]
fn every_advance_from_define_numbers_a_fresh_request() {
    let (s, _) = defining(BulkKind::Delete, config(false, false));
    let (s, first) = act(s, BulkAction::Next);
    let (s, _) = act(s, BulkAction::Back);
    let (_, second) = act(s, BulkAction::Next);
    assert_ne!(preview_id(&first), preview_id(&second));
}

#[test]
fn preview_answers_are_stored() {
    let (s, _) = previewing(BulkKind::Delete, config(false, false));
    assert_eq!(
        flow(&s).preview,
        PreviewState::Sample(vec![doc! { "_id": 1 }])
    );
    let (s, _) = previewing(BulkKind::Update, config(false, false));
    assert!(matches!(flow(&s).preview, PreviewState::Update(_)));
}

#[test]
fn a_failed_preview_outcome_becomes_the_failed_state() {
    let (s, id) = defining(BulkKind::Update, config(false, false));
    let (s, cmds) = act(s, BulkAction::Next);
    let (s, _) = ev(
        s,
        Event::BulkPreview {
            session: id,
            ns: ns(),
            request_id: preview_id(&cmds),
            outcome: Err(CoreError::BulkPreviewUnavailable),
        },
    );
    assert_eq!(flow(&s).preview, PreviewState::Failed);
}

#[test]
fn next_from_preview_moves_to_confirm_and_back_steps_down() {
    let (s, _) = previewing(BulkKind::Delete, config(false, false));
    let (s, _) = act(s, BulkAction::Next);
    assert_eq!(flow(&s).step, BulkStep::Confirm);
    let (s, _) = act(s, BulkAction::Back);
    assert_eq!(flow(&s).step, BulkStep::Preview);
    assert!(matches!(flow(&s).preview, PreviewState::Sample(_)));
    let (s, _) = act(s, BulkAction::Back);
    assert_eq!(flow(&s).step, BulkStep::Define);
    assert_eq!(flow(&s).preview, PreviewState::Pending);
}

// ---------------------------------------------------------------- stale answers

#[test]
fn stale_preview_answer_is_ignored() {
    let (s, id) = defining(BulkKind::Delete, config(false, false));
    let (s, cmds) = act(s, BulkAction::Next);
    let old = preview_id(&cmds);
    let (s, _) = ev(s, preview_answer(id, BulkKind::Delete, old + 100));
    assert_eq!(flow(&s).preview, PreviewState::Pending);
    let (s, _) = ev(s, preview_answer(id, BulkKind::Delete, old));
    assert!(matches!(flow(&s).preview, PreviewState::Sample(_)));
}

#[test]
fn a_preview_answer_is_taken_once() {
    let (s, id) = defining(BulkKind::Delete, config(false, false));
    let (s, cmds) = act(s, BulkAction::Next);
    let request = preview_id(&cmds);
    let (s, _) = ev(s, preview_answer(id, BulkKind::Delete, request));
    assert_eq!(s.bulk.as_ref().unwrap().preview_request, None);
}

#[test]
fn preview_after_back_edit_advance_ignores_the_old_answer() {
    let (s, id) = defining(BulkKind::Update, config(false, false));
    let (s, first) = act(s, BulkAction::Next);
    let old = preview_id(&first);
    let (s, _) = act(s, BulkAction::Back);
    assert_eq!(s.bulk.as_ref().unwrap().preview_request, None);
    // The old answer arrives while the person is editing: nothing to apply it to.
    let (s, _) = ev(s, preview_answer(id, BulkKind::Update, old));
    assert_eq!(flow(&s).preview, PreviewState::Pending);
    let (s, _) = act(s, BulkAction::SetUpdateText(r#"{"$set": {"b": 2}}"#.into()));
    let (s, second) = act(s, BulkAction::Next);
    // ...and arrives again after the new advance: still not the awaited request.
    let (s, _) = ev(s, preview_answer(id, BulkKind::Update, old));
    assert_eq!(flow(&s).preview, PreviewState::Pending);
    assert_eq!(flow(&s).step, BulkStep::Preview);
    let (s, _) = ev(s, preview_answer(id, BulkKind::Update, preview_id(&second)));
    assert!(matches!(flow(&s).preview, PreviewState::Update(_)));
}

#[test]
fn stale_count_answer_is_ignored() {
    let (s, id) = plain();
    let (s, cmds) = act(s, BulkAction::Open(BulkKind::Delete));
    let request = count_id(&cmds);
    let (s, _) = ev(s, count_answer(id, request + 7, Some(3)));
    assert_eq!(flow(&s).count, CountState::Pending);
    let (s, _) = ev(s, count_answer(id, request, Some(3)));
    assert_eq!(flow(&s).count, CountState::Known(3));
}

#[test]
fn an_answer_for_another_collection_or_session_is_ignored() {
    let (s, id) = plain();
    let (s, cmds) = act(s, BulkAction::Open(BulkKind::Delete));
    let request = count_id(&cmds);
    let other_ns = Event::BulkCount {
        session: id,
        ns: Namespace::new("shop", "other"),
        request,
        count: Some(1),
    };
    let (s, _) = ev(s, other_ns);
    assert_eq!(flow(&s).count, CountState::Pending);
    let mut s = s;
    s.bulk = s.bulk.map(|b| BulkState {
        session: Uuid::new_v4(),
        ..b
    });
    let applied = crate::app::bulk_actions::count_arrived(s, ns(), request, Some(1));
    assert_eq!(flow(&applied).count, CountState::Pending);
}

#[test]
fn a_count_without_an_answer_from_the_store_is_unavailable() {
    let (s, id) = plain();
    let (s, cmds) = act(s, BulkAction::Open(BulkKind::Delete));
    let (s, _) = ev(s, count_answer(id, count_id(&cmds), None));
    assert_eq!(flow(&s).count, CountState::Unavailable);
}

fn count_timeout(id: Uuid, request: u64) -> Event {
    Event::RequestFailed {
        session: id,
        context: "bulk_count",
        request,
        error: CoreError::Database("timed out".into()),
    }
}

#[test]
fn bulk_count_timeout_request_failed_marks_count_unavailable() {
    let (s, id) = plain();
    let (s, cmds) = act(s, BulkAction::Open(BulkKind::Delete));
    let (s, _) = ev(s, count_timeout(id, count_id(&cmds)));
    assert_eq!(flow(&s).count, CountState::Unavailable);
    assert!(s.banner.is_none());
}

#[test]
fn a_stale_count_request_failed_is_ignored() {
    let (s, id) = plain();
    let (s, cmds) = act(s, BulkAction::Open(BulkKind::Delete));
    let (s, _) = ev(s, count_timeout(id, count_id(&cmds) + 5));
    assert_eq!(flow(&s).count, CountState::Pending);
}

// ---------------------------------------------------------------- run

#[test]
fn run_without_typed_confirmation_sends_nothing() {
    let (s, _) = confirming(BulkKind::Delete, config(false, false));
    let (s, cmds) = act(s, BulkAction::Run);
    assert!(cmds.is_empty());
    assert_eq!(flow(&s).step, BulkStep::Confirm);
    assert!(s.pending.is_none());
}

#[test]
fn run_is_a_noop_when_can_run_is_false() {
    let (s, _) = defining(BulkKind::Delete, config(false, false));
    let (s, cmds) = act(s, BulkAction::Run);
    assert!(cmds.is_empty());
    assert_eq!(flow(&s).step, BulkStep::Define);
}

#[test]
fn run_with_confirmation_sends_bulk_delete_once() {
    let (s, id) = ready_delete(config(false, false));
    let (s, cmds) = act(s, BulkAction::Run);
    let [
        Command::BulkDelete {
            session,
            ns: n,
            request,
            run,
        },
    ] = &cmds[..]
    else {
        panic!("expected one BulkDelete, got {cmds:?}");
    };
    assert_eq!((*session, n), (id, &ns()));
    assert_eq!(request.filter, doc! { "status": "old" });
    assert_eq!(request.scope, MatchScope::Filtered);
    assert_eq!(flow(&s).step, BulkStep::Running);
    assert_eq!(flow(&s).run, Some(*run));
    assert_eq!(s.pending.as_ref().map(|p| &p.ns), Some(&ns()));
    let (_, again) = act(s, BulkAction::Run);
    assert!(again.is_empty());
}

#[test]
fn run_sends_bulk_update_with_the_parsed_update() {
    let (s, _) = confirming(BulkKind::Update, config(false, false));
    let (s, _) = act(s, BulkAction::Back);
    let (s, _) = act(s, BulkAction::Back);
    let (s, _) = act(s, BulkAction::SetUpsert(true));
    let id = s.bulk.as_ref().unwrap().session;
    let (s, cmds) = act(s, BulkAction::Next);
    let (s, _) = ev(s, preview_answer(id, BulkKind::Update, preview_id(&cmds)));
    let (s, _) = act(s, BulkAction::Next);
    let (s, cmds) = act(s, BulkAction::Run);
    let [Command::BulkUpdate { request, run, .. }] = &cmds[..] else {
        panic!("expected one BulkUpdate, got {cmds:?}");
    };
    assert!(request.upsert);
    assert_eq!(request.filter, doc! { "status": "old" });
    assert_eq!(flow(&s).run, Some(*run));
}

#[test]
fn run_on_production_sends_without_a_second_warning() {
    let (s, _) = ready_delete(config(true, false));
    let (s, cmds) = act(s, BulkAction::Run);
    assert!(s.write_warning.is_none());
    assert!(matches!(&cmds[..], [Command::BulkDelete { .. }]));
}

#[test]
fn run_is_refused_when_the_connection_became_read_only() {
    let (s, id) = ready_delete(config(false, false));
    let mut s = s;
    s.conn.connections.iter_mut().for_each(|c| {
        if c.id == id {
            c.read_only = true;
        }
    });
    let (s, cmds) = act(s, BulkAction::Run);
    assert!(cmds.is_empty());
    assert_eq!(s.banner, Some(Banner::ReadOnly));
    assert_eq!(flow(&s).step, BulkStep::Confirm);
}

#[test]
fn run_is_ignored_while_another_write_is_in_flight() {
    let (mut s, _) = ready_delete(config(false, false));
    s.pending = Some(Pending {
        ns: ns(),
        editor: None,
    });
    let (s, cmds) = act(s, BulkAction::Run);
    assert!(cmds.is_empty());
    assert_eq!(flow(&s).step, BulkStep::Confirm);
}

#[test]
fn epoch_change_closes_the_flow_and_run_sends_nothing() {
    let (mut s, _) = ready_delete(config(false, false));
    s.latest_requested_epoch += 1;
    let (s, cmds) = act(s, BulkAction::Run);
    assert!(cmds.is_empty());
    assert!(s.bulk.is_none());
    assert!(s.pending.is_none());
}

// ---------------------------------------------------------------- session changes

#[test]
fn a_lost_connection_closes_an_idle_flow() {
    let (s, id) = confirming(BulkKind::Delete, config(false, false));
    let (s, _) = ev(
        s,
        Event::Failed {
            session: id,
            context: "find",
            error: CoreError::Unreachable,
        },
    );
    assert!(s.bulk.is_none());
    assert_eq!(s.status_of(id), Some(ConnStatus::Failed));
}

#[test]
fn disconnect_closes_the_flow_and_a_reconnect_does_not_bring_it_back() {
    let (s, id) = confirming(BulkKind::Delete, config(false, false));
    let (s, _) = update(s, Input::Disconnect(id));
    assert!(s.bulk.is_none());
    assert_eq!(s.bulk_requests, 2, "the counter is not reset");
}

#[test]
fn a_reconnect_closes_the_flow() {
    let (s, id) = confirming(BulkKind::Delete, config(false, false));
    let (s, _) = ev(
        s,
        Event::Failed {
            session: id,
            context: "find",
            error: CoreError::Unreachable,
        },
    );
    let (s, _) = update(s, Input::Connect(id));
    assert!(s.bulk.is_none());
}

#[test]
fn session_loss_during_running_still_reports_the_finish() {
    let (s, id, run) = running_delete();
    let (s, _) = ev(
        s,
        Event::Failed {
            session: id,
            context: "find",
            error: CoreError::Unreachable,
        },
    );
    assert_eq!(s.status_of(id), Some(ConnStatus::Failed));
    assert_eq!(flow(&s).step, BulkStep::Running, "the run is still ours");
    let unknown = BulkOutcome::Unknown { cancelled: false };
    let (s, cmds) = ev(s, finish(id, run, Ok(unknown.clone())));
    assert_eq!(flow(&s).step, BulkStep::Report);
    assert_eq!(flow(&s).report, Some(Ok(unknown)));
    assert!(cmds.is_empty(), "a lost session has nothing to re-query");
}

#[test]
fn a_flow_closed_with_its_session_gets_no_report_and_no_crash() {
    let (s, id, run) = running_delete();
    let (s, _) = update(s, Input::Disconnect(id));
    let (s, cmds) = ev(s, finish(id, run, Ok(applied())));
    assert!(s.bulk.is_none());
    assert!(cmds.is_empty());
}

// ---------------------------------------------------------------- finish

#[test]
fn finished_unknown_shows_unknown_report_and_requeries() {
    let (s, id, run) = running_delete();
    let unknown = BulkOutcome::Unknown { cancelled: false };
    let (s, cmds) = ev(s, finish(id, run, Ok(unknown.clone())));
    assert_eq!(flow(&s).step, BulkStep::Report);
    assert_eq!(flow(&s).report, Some(Ok(unknown)));
    assert!(s.pending.is_none());
    assert!(matches!(
        &cmds[..],
        [Command::Find { query, .. }] if *query == s.page_query
    ));
}

#[test]
fn finished_error_is_reported_and_requeries() {
    let (s, id, run) = running_delete();
    let (s, cmds) = ev(s, finish(id, run, Err(CoreError::BulkEmptyFilter)));
    assert_eq!(flow(&s).report, Some(Err(CoreError::BulkEmptyFilter)));
    assert!(s.pending.is_none());
    assert!(cmds.iter().any(is_find));
}

#[test]
fn finished_for_unknown_run_requeries_without_a_flow() {
    let (s, id) = plain();
    let stray = BulkRunId(Uuid::new_v4());
    let (s, cmds) = ev(s, finish(id, stray, Ok(applied())));
    assert!(s.bulk.is_none());
    assert!(matches!(&cmds[..], [Command::Find { .. }]));
}

#[test]
fn finished_for_another_run_changes_no_flow_and_leaves_the_pending_write() {
    let (s, id, _) = running_delete();
    let (s, cmds) = ev(s, finish(id, BulkRunId(Uuid::new_v4()), Ok(applied())));
    assert_eq!(flow(&s).step, BulkStep::Running);
    assert!(s.pending.is_some());
    assert!(cmds.iter().any(is_find));
}

#[test]
fn finished_on_an_unselected_collection_does_not_requery() {
    let (s, id) = plain();
    let event = Event::BulkFinished {
        session: id,
        ns: Namespace::new("shop", "other"),
        run: BulkRunId(Uuid::new_v4()),
        outcome: Ok(applied()),
    };
    let (_, cmds) = ev(s, event);
    assert!(cmds.is_empty());
}

// ---------------------------------------------------------------- cancel and close

#[test]
fn cancel_while_running_sends_bulk_cancel() {
    let (s, id, run) = running_delete();
    let (s, cmds) = act(s, BulkAction::Cancel);
    assert!(matches!(
        &cmds[..],
        [Command::BulkCancel { session, run: r }] if *session == id && *r == run
    ));
    assert_eq!(flow(&s).step, BulkStep::Running);
}

#[test]
fn close_while_running_does_nothing() {
    let (s, _, _) = running_delete();
    let (s, cmds) = act(s, BulkAction::Close);
    assert!(cmds.is_empty());
    assert_eq!(flow(&s).step, BulkStep::Running);
}

#[test]
fn escape_and_close_cancel_from_every_step_except_running() {
    for action in [BulkAction::Cancel, BulkAction::Close] {
        let (define, _) = defining(BulkKind::Delete, config(false, false));
        let (preview, _) = previewing(BulkKind::Delete, config(false, false));
        let (confirm, _) = confirming(BulkKind::Delete, config(false, false));
        let (running, id, run) = running_delete();
        let (report, _) = ev(running, finish(id, run, Ok(applied())));
        for s in [define, preview, confirm, report] {
            let (s, cmds) = act(s, action.clone());
            assert!(s.bulk.is_none(), "{action:?} must close the flow");
            assert!(cmds.is_empty());
        }
    }
}

#[test]
fn closing_drops_the_outstanding_preview_id() {
    let (s, id) = defining(BulkKind::Delete, config(false, false));
    let (s, cmds) = act(s, BulkAction::Next);
    let request = preview_id(&cmds);
    let (s, _) = act(s, BulkAction::Close);
    let (s, _) = ev(s, preview_answer(id, BulkKind::Delete, request));
    assert!(s.bulk.is_none());
}

#[test]
fn cancel_unavailable_is_recorded_and_run_continues() {
    let (s, id, run) = running_delete();
    let (s, cmds) = ev(
        s,
        Event::BulkCancelResult {
            session: id,
            run,
            outcome: Ok(()),
        },
    );
    assert!(!flow(&s).cancel_unavailable && cmds.is_empty());
    let (s, cmds) = ev(
        s,
        Event::BulkCancelResult {
            session: id,
            run,
            outcome: Err(CoreError::Unreachable),
        },
    );
    assert!(flow(&s).cancel_unavailable);
    assert_eq!(flow(&s).step, BulkStep::Running);
    assert!(cmds.is_empty());
}

#[test]
fn a_cancel_result_for_another_run_is_ignored() {
    let (s, id, _) = running_delete();
    let (s, _) = ev(
        s,
        Event::BulkCancelResult {
            session: id,
            run: BulkRunId(Uuid::new_v4()),
            outcome: Err(CoreError::Unreachable),
        },
    );
    assert!(!flow(&s).cancel_unavailable);
}

// ---------------------------------------------------------------- failures

fn failed(id: Uuid, context: &'static str, error: CoreError) -> Event {
    Event::Failed {
        session: id,
        context,
        error,
    }
}

#[test]
fn failed_read_only_closes_the_flow_with_the_banner() {
    let (s, id, _) = running_delete();
    let (s, cmds) = ev(s, failed(id, "bulk_delete", CoreError::ReadOnly));
    assert!(s.bulk.is_none());
    assert!(s.pending.is_none());
    assert_eq!(s.banner, Some(Banner::ReadOnly));
    assert!(cmds.is_empty());
}

#[test]
fn another_failed_write_is_reported_in_the_failure_banner_and_closes_the_flow() {
    let (s, id, _) = running_delete();
    let (s, _) = ev(s, failed(id, "bulk_delete", CoreError::BulkScopeMismatch));
    assert!(s.bulk.is_none());
    assert!(s.pending.is_none());
    assert_eq!(
        s.banner,
        Some(Banner::Failure {
            context: "bulk_delete",
            error: CoreError::BulkScopeMismatch
        })
    );
}

#[test]
fn a_failed_preview_command_marks_the_preview_failed_and_keeps_the_flow() {
    let (s, id) = defining(BulkKind::Delete, config(false, false));
    let (s, _) = act(s, BulkAction::Next);
    let (s, _) = ev(
        s,
        failed(id, "bulk_sample", CoreError::Database("x".into())),
    );
    assert_eq!(flow(&s).preview, PreviewState::Failed);
    assert!(s.banner.is_none());
}

#[test]
fn a_failed_count_command_marks_the_count_unavailable() {
    let (s, id) = plain();
    let (s, _) = act(s, BulkAction::Open(BulkKind::Delete));
    let (s, _) = ev(s, failed(id, "bulk_count", CoreError::Database("x".into())));
    assert_eq!(flow(&s).count, CountState::Unavailable);
}

#[test]
fn a_failed_cancel_only_raises_the_banner() {
    let (s, id, _) = running_delete();
    let (s, _) = ev(
        s,
        failed(id, "bulk_cancel", CoreError::Database("x".into())),
    );
    assert_eq!(flow(&s).step, BulkStep::Running);
    assert!(matches!(s.banner, Some(Banner::Failure { .. })));
}
