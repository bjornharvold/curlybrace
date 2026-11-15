//! The Explain section through the reducer: a run sends the numbered command with the exact
//! stages (no `$limit`, no trailing write stage), a stale or foreign answer is ignored, a failure
//! stays inline, a lost server fails the session, and the plan survives a draft edit marked out
//! of date.
use crate::app::reducer::{Input, Out, update};
use crate::app::session::SessionView;
use crate::app::state::{AppState, ConnStatus};
use crate::app::{AggregateAction as A, ExplainAction};
use crate::viewmodel::aggregate_explain::{ExplainState, ResultTab};
use crate::viewmodel::{AggregateState, SidebarAction};
use bson::{Document, doc};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::ejson::editor_json_to_doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::{AggPlanOutcome, ExplainVerbosity};
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Explain, Namespace};
use uuid::Uuid;

fn people() -> Namespace {
    Namespace::new("app", "people")
}

fn ev(state: AppState, event: Event) -> Out {
    update(state, Input::Event(event))
}

fn act(state: AppState, action: A) -> Out {
    update(state, Input::Aggregate(action))
}

fn explain(state: AppState, action: ExplainAction) -> Out {
    act(state, A::Explain(action))
}

/// One saved connection, connected, `app.people` open.
fn browsing() -> (AppState, Uuid) {
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let (s, _) = update(AppState::new(vec![cfg]), Input::Connect(id));
    let epoch = s.epoch_counter;
    let connected = Event::Connected {
        session: id,
        epoch,
        topology: None,
    };
    let s = ev(s, connected).0;
    (
        update(s, Input::Sidebar(SidebarAction::Open(people()))).0,
        id,
    )
}

/// `browsing()` with `$match`, `$sort`, `$out` stages (previews are left unanswered).
fn drafted() -> (AppState, Uuid) {
    let (s, id) = browsing();
    let s = ["$match", "$sort", "$out"]
        .into_iter()
        .fold(s, |s, name| act(s, A::AddStage(name.into())).0);
    (s, id)
}

fn sent(commands: &[Command]) -> Vec<(u64, Vec<Document>, ExplainVerbosity)> {
    commands
        .iter()
        .filter_map(|c| match c {
            Command::AggregateExplain {
                request,
                stages,
                verbosity,
                ..
            } => Some((*request, stages.clone(), *verbosity)),
            _ => None,
        })
        .collect()
}

/// The request number of the one explain command in `commands`.
fn request_of(commands: &[Command]) -> u64 {
    match sent(commands).as_slice() {
        [(request, ..)] => *request,
        other => panic!("expected one AggregateExplain, got {other:?}"),
    }
}

fn plan_document() -> Document {
    editor_json_to_doc(include_str!(
        "../../../core/tests/fixtures/explain_aggregate/stages_collscan.json"
    ))
    .expect("fixture parses")
}

fn answer(id: Uuid, request: u64, outcome: Result<Explain, CoreError>) -> Event {
    Event::AggregateExplained {
        session: id,
        request,
        ns: people(),
        verbosity: ExplainVerbosity::QueryPlanner,
        outcome,
    }
}

fn plan_answer(id: Uuid, request: u64) -> Event {
    let document = plan_document();
    answer(
        id,
        request,
        Ok(Explain {
            document,
            plan_only: true,
        }),
    )
}

fn failed_event(id: Uuid, request: u64, error: CoreError) -> Event {
    Event::RequestFailed {
        session: id,
        context: "aggregate_explain",
        request,
        error,
    }
}

#[test]
fn run_sends_the_numbered_command_with_the_exact_stages_and_shows_the_plan_tab() {
    let (s, id) = drafted();
    let s = act(s, A::Select(1)).0;
    let expected = s.aggregate.stages[..2]
        .iter()
        .map(|st| editor_json_to_doc(&st.text).expect("stage text is JSON"))
        .collect::<Vec<_>>();
    let (s, commands) = explain(s, ExplainAction::Run);
    let sent = sent(&commands);
    assert_eq!(commands.len(), 1);
    assert_eq!(sent.len(), 1);
    let (request, stages, verbosity) = &sent[0];
    assert_eq!(stages, &expected, "no $limit, nothing after the selection");
    assert_eq!(*verbosity, ExplainVerbosity::QueryPlanner);
    assert!(s.aggregate.explain.awaits(*request));
    assert_eq!(s.aggregate.explain.tab, ResultTab::Plan);
    assert!(matches!(
        &commands[0],
        Command::AggregateExplain { session, ns, .. } if *session == id && *ns == people()
    ));
}

#[test]
fn a_trailing_write_stage_is_left_out_of_the_command() {
    let (s, _) = drafted();
    let (_, commands) = explain(s, ExplainAction::Run);
    let (_, stages, _) = &sent(&commands)[0];
    assert_eq!(stages.len(), 2);
    assert!(stages.iter().all(|st| !st.contains_key("$out")));
}

#[test]
fn the_chosen_verbosity_is_sent() {
    let (s, _) = drafted();
    let (s, none) = explain(
        s,
        ExplainAction::SetVerbosity(ExplainVerbosity::ExecutionStats),
    );
    assert!(none.is_empty());
    let (_, commands) = explain(s, ExplainAction::Run);
    assert_eq!(sent(&commands)[0].2, ExplainVerbosity::ExecutionStats);
}

#[test]
fn nothing_is_sent_without_a_pipeline_a_connection_or_a_collection() {
    let (s, _) = browsing();
    let (s, c) = explain(s, ExplainAction::Run);
    assert!(c.is_empty(), "no stages");
    assert!(!s.aggregate.explain.is_loading());

    let (mut s, _) = drafted();
    s.sidebar.selected = None;
    let (s, c) = explain(s, ExplainAction::Run);
    assert!(c.is_empty(), "no collection");
    assert!(!s.aggregate.explain.is_loading());

    let (mut s, _) = drafted();
    s.status = ConnStatus::Failed;
    assert!(explain(s, ExplainAction::Run).1.is_empty(), "no connection");

    let (s, _) = drafted();
    let s = act(s, A::Edit(0, "{ not json".into())).0;
    assert!(
        explain(s, ExplainAction::Run).1.is_empty(),
        "a stage that does not parse"
    );
}

#[test]
fn a_second_run_while_one_waits_sends_nothing() {
    let (s, _) = drafted();
    let (s, first) = explain(s, ExplainAction::Run);
    let request = request_of(&first);
    let (s, second) = explain(s, ExplainAction::Run);
    assert!(second.is_empty());
    assert!(s.aggregate.explain.awaits(request));
}

/// `s` after a Run, and the request number it sent.
fn ran(s: AppState) -> (AppState, u64) {
    let (s, commands) = explain(s, ExplainAction::Run);
    let request = request_of(&commands);
    (s, request)
}

#[test]
fn the_answer_is_shown_and_a_new_run_is_possible_after_it() {
    let (s, id) = drafted();
    let (s, request) = ran(s);
    let (s, c) = ev(s, plan_answer(id, request));
    assert!(c.is_empty());
    let done = s.aggregate.explain.done().expect("done");
    assert!(matches!(done.plan, AggPlanOutcome::Plan(_)));
    assert_eq!(done.raw, plan_document());
    let (s, again) = ran(s);
    assert!(again > request, "a newer number");
    assert!(s.aggregate.explain.awaits(again));
}

#[test]
fn an_answer_with_another_number_or_for_another_collection_is_ignored() {
    let (s, id) = drafted();
    let (s, request) = ran(s);
    let before = s.aggregate.explain.clone();
    let (s, _) = ev(s, plan_answer(id, request + 9));
    assert_eq!(s.aggregate.explain, before);
    let elsewhere = Event::AggregateExplained {
        session: id,
        request,
        ns: Namespace::new("app", "orders"),
        verbosity: ExplainVerbosity::QueryPlanner,
        outcome: Ok(Explain {
            document: plan_document(),
            plan_only: true,
        }),
    };
    let (s, _) = ev(s, elsewhere);
    assert_eq!(s.aggregate.explain, before);
}

#[test]
fn an_older_answer_never_overwrites_a_newer_state() {
    let (s, id) = drafted();
    let (s, first) = ran(s);
    let (s, _) = ev(s, plan_answer(id, first));
    let (s, second) = ran(s);
    let (s, _) = ev(s, plan_answer(id, first));
    assert!(
        s.aggregate.explain.awaits(second),
        "still waiting for the newer one"
    );
}

#[test]
fn a_failure_stays_inline_without_a_banner() {
    let (s, id) = drafted();
    let (s, request) = ran(s);
    let (s, _) = ev(
        s,
        answer(id, request, Err(CoreError::Database("boom".into()))),
    );
    assert!(matches!(
        s.aggregate.explain.state,
        ExplainState::Failed {
            reason: CoreError::Database(_)
        }
    ));
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(s.banner.is_none());

    let (s, request) = ran(s);
    let (s, _) = ev(s, failed_event(id, request, CoreError::ReadOnly));
    assert!(matches!(
        s.aggregate.explain.state,
        ExplainState::Failed {
            reason: CoreError::ReadOnly
        }
    ));
    assert!(s.banner.is_none());
}

#[test]
fn a_lost_server_fails_the_session_and_clears_the_explain() {
    for error in [CoreError::Unreachable, CoreError::TlsHandshake] {
        let (s, id) = drafted();
        let (s, request) = ran(s);
        let (s, _) = ev(s, failed_event(id, request, error));
        assert_eq!(s.status, ConnStatus::Failed);
        assert_eq!(s.aggregate.stages.len(), 3, "the draft survives");
        assert_eq!(s.aggregate.explain.state, ExplainState::Idle);
        assert!(!s.aggregate.explain.awaits(request));
    }
}

#[test]
fn a_loss_for_a_request_nobody_awaits_changes_nothing() {
    let (s, id) = drafted();
    let (s, request) = ran(s);
    let (s, _) = ev(s, failed_event(id, request + 7, CoreError::Unreachable));
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(s.aggregate.explain.awaits(request));
}

#[test]
fn a_superseded_request_frees_the_pane_without_a_message() {
    let (s, id) = drafted();
    let (s, request) = ran(s);
    let (s, _) = ev(s, failed_event(id, request, CoreError::Superseded));
    assert_eq!(s.aggregate.explain.state, ExplainState::Idle);
    assert!(s.banner.is_none());
}

#[test]
fn a_draft_edit_keeps_the_plan_and_makes_it_out_of_date() {
    let (s, id) = drafted();
    let (s, request) = ran(s);
    let (s, _) = ev(s, plan_answer(id, request));
    let sent_pipeline = s.aggregate.explain.done().expect("done").for_draft.clone();
    let s = act(s, A::Edit(0, "{ \"$match\": { \"a\": 1 } }".into())).0;
    let done = s.aggregate.explain.done().expect("still shown");
    assert_eq!(done.for_draft, sent_pipeline);
    let now = crate::viewmodel::aggregate_explain_block::explain_request(
        &s.aggregate,
        &s.aggregate.stages,
    )
    .expect("valid");
    assert!(s.aggregate.explain.is_out_of_date(Some(&now)));
}

#[test]
fn another_collection_or_a_new_connection_forgets_the_plan() {
    let (s, id) = drafted();
    let (s, request) = ran(s);
    let (s, _) = ev(s, plan_answer(id, request));
    let (s, _) = update(
        s,
        Input::Sidebar(SidebarAction::Open(Namespace::new("app", "orders"))),
    );
    assert_eq!(s.aggregate.explain.state, ExplainState::Idle);
    assert_eq!(s.aggregate.explain.tab, ResultTab::Preview);
}

#[test]
fn the_tab_and_the_verbosity_are_kept_per_session() {
    let (s, _) = drafted();
    let (s, _) = explain(s, ExplainAction::SelectTab(ResultTab::Raw));
    assert_eq!(s.aggregate.explain.tab, ResultTab::Raw);
    let (s, c) = explain(s, ExplainAction::CopyRaw);
    assert!(c.is_empty());
    assert_eq!(s.aggregate.explain.tab, ResultTab::Raw);
}

#[test]
fn an_answer_for_a_parked_session_lands_in_its_own_draft() {
    let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let (draft_b, _) = AggregateState::default().add_stage("$sort").begin_preview();
    let (ui_b, request, _) = draft_b.explain.clone().begun(vec![doc! { "$sort": {} }]);
    let mut draft_b = draft_b;
    draft_b.explain = ui_b;
    let view_b = SessionView {
        status: ConnStatus::Connected,
        aggregate: Box::new(draft_b),
        sidebar: crate::viewmodel::SidebarState::default()
            .with_databases(vec!["app".into()])
            .with_collections("app", vec!["people".into()])
            .toggled("app")
            .selecting(people()),
        ..SessionView::default()
    };
    let (s, _) = drafted();
    let s = AppState {
        open: vec![a],
        active: Some(a),
        ..s
    };
    let s = s.opened_with(b, view_b).activated(a);
    let mine = (*s.aggregate).clone();
    let (s, c) = ev(s, plan_answer(b, request));
    assert!(c.is_empty());
    assert_eq!(s.active, Some(a));
    assert_eq!(*s.aggregate, mine, "the active draft is untouched");
    assert!(s.parked[&b].aggregate.explain.done().is_some());
}
