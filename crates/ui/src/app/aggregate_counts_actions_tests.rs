//! The per-stage counts through the reducer: Start sends the first stage and each answer the next
//! one, never two at once; a stale or foreign answer is ignored; a failure stays on its card and
//! the sequence goes on; a draft edit, Cancel, a collection change, a lost server and a session
//! switch end it; a write stage is never sent.
use crate::app::reducer::{Input, Out, update};
use crate::app::session::SessionView;
use crate::app::state::{AppState, ConnStatus};
use crate::app::{AggregateAction as A, CountsAction};
use crate::viewmodel::aggregate_counts::StageCount;
use crate::viewmodel::aggregate_counts_plan::NotCounted;
use crate::viewmodel::{AggregateState, SidebarAction};
use bson::{Document, doc};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::COUNT_CAP;
use curlybrace_core::runtime::{COMMAND_TIMEOUT_MESSAGE, Command, Event};
use curlybrace_core::store::Namespace;
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

fn counts(state: AppState, action: CountsAction) -> Out {
    act(state, A::Counts(action))
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

/// `browsing()` with `$match`, `$sort`, `$limit` stages.
fn drafted() -> (AppState, Uuid) {
    let (s, id) = browsing();
    let s = ["$match", "$sort", "$limit"]
        .into_iter()
        .fold(s, |s, name| act(s, A::AddStage(name.into())).0);
    (s, id)
}

fn sent(commands: &[Command]) -> Vec<(u64, usize, Vec<Document>)> {
    commands
        .iter()
        .filter_map(|c| match c {
            Command::AggregateCount {
                request,
                stage_index,
                pipeline,
                ..
            } => Some((*request, *stage_index, pipeline.clone())),
            _ => None,
        })
        .collect()
}

/// The one count command in `commands`: its request and stage.
fn one(commands: &[Command]) -> (u64, usize) {
    match sent(commands).as_slice() {
        [(request, stage, _)] => (*request, *stage),
        other => panic!("expected one AggregateCount, got {other:?}"),
    }
}

fn answer(id: Uuid, request: u64, stage_index: usize, outcome: Result<u64, CoreError>) -> Event {
    Event::AggregateCounted {
        session: id,
        request,
        ns: people(),
        stage_index,
        outcome,
    }
}

fn failed_event(id: Uuid, request: u64, error: CoreError) -> Event {
    Event::RequestFailed {
        session: id,
        context: "aggregate_count",
        request,
        error,
    }
}

fn started(s: AppState) -> (AppState, u64) {
    let (s, commands) = counts(s, CountsAction::Start);
    let (request, stage) = one(&commands);
    assert_eq!(stage, 0);
    (s, request)
}

fn result_of(s: &AppState, index: usize) -> Option<StageCount> {
    s.aggregate
        .counts
        .result_of(s.aggregate.keys[index])
        .cloned()
}

#[test]
fn start_sends_stage_zero_with_the_bounded_pipeline_and_nothing_else() {
    let (s, id) = drafted();
    let (s, commands) = counts(s, CountsAction::Start);
    assert_eq!(commands.len(), 1, "one at a time");
    let (request, stage, pipeline) = sent(&commands).remove(0);
    assert_eq!(stage, 0);
    assert_eq!(pipeline.len(), 3, "the stage, a limit and a count");
    assert_eq!(pipeline[1], doc! { "$limit": COUNT_CAP });
    assert_eq!(pipeline[2], doc! { "$count": "n" });
    assert!(s.aggregate.counts.awaits(request));
    assert!(matches!(
        &commands[0],
        Command::AggregateCount { session, ns, .. } if *session == id && *ns == people()
    ));
}

#[test]
fn each_answer_sends_the_next_stage_in_order_until_the_last() {
    let (s, id) = drafted();
    let (s, first) = started(s);
    let (s, commands) = ev(s, answer(id, first, 0, Ok(12)));
    let (second, stage) = one(&commands);
    assert_eq!(stage, 1);
    assert_eq!(result_of(&s, 0), Some(StageCount::Exact(12)));
    let (s, commands) = ev(s, answer(id, second, 1, Ok(0)));
    let (third, stage) = one(&commands);
    assert_eq!(stage, 2);
    let (s, commands) = ev(s, answer(id, third, 2, Ok(COUNT_CAP as u64)));
    assert!(commands.is_empty(), "the sequence is over");
    assert!(!s.aggregate.counts.is_running());
    assert_eq!(result_of(&s, 1), Some(StageCount::Exact(0)));
    assert_eq!(
        result_of(&s, 2),
        Some(StageCount::AtLeast(COUNT_CAP as u64))
    );
}

#[test]
fn a_second_start_while_running_sends_nothing() {
    let (s, _) = drafted();
    let (s, request) = started(s);
    let (s, again) = counts(s, CountsAction::Start);
    assert!(again.is_empty());
    assert!(s.aggregate.counts.awaits(request));
}

#[test]
fn nothing_is_sent_without_a_pipeline_a_connection_or_a_collection() {
    let (s, _) = browsing();
    assert!(counts(s, CountsAction::Start).1.is_empty(), "no stages");

    let (mut s, _) = drafted();
    s.sidebar.selected = None;
    assert!(counts(s, CountsAction::Start).1.is_empty(), "no collection");

    let (mut s, _) = drafted();
    s.status = ConnStatus::Failed;
    assert!(counts(s, CountsAction::Start).1.is_empty(), "no connection");

    let (s, _) = browsing();
    let s = act(s, A::AddStage("$match".into())).0;
    let s = act(s, A::Edit(0, "{ not json".into())).0;
    let (s, commands) = counts(s, CountsAction::Start);
    assert!(commands.is_empty(), "no stage can be counted");
    assert!(!s.aggregate.counts.is_running());
}

#[test]
fn a_failure_on_one_stage_stays_on_its_card_and_the_next_stage_is_counted() {
    let (s, id) = drafted();
    let (s, first) = started(s);
    let (s, commands) = ev(
        s,
        answer(id, first, 0, Err(CoreError::Database("boom".into()))),
    );
    assert_eq!(one(&commands).1, 1);
    assert!(matches!(
        result_of(&s, 0),
        Some(StageCount::Failed(CoreError::Database(_)))
    ));
    assert!(s.banner.is_none());
    assert_eq!(s.status, ConnStatus::Connected);
}

#[test]
fn a_timeout_is_a_timed_out_card_and_the_sequence_goes_on() {
    let (s, id) = drafted();
    let (s, first) = started(s);
    let timeout = Err(CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into()));
    let (s, commands) = ev(s, answer(id, first, 0, timeout));
    assert_eq!(result_of(&s, 0), Some(StageCount::TimedOut));
    assert_eq!(one(&commands).1, 1);
}

#[test]
fn a_request_the_runtime_could_not_run_fails_that_stage_and_goes_on() {
    let (s, id) = drafted();
    let (s, first) = started(s);
    let (s, commands) = ev(s, failed_event(id, first, CoreError::ReadOnly));
    assert!(matches!(
        result_of(&s, 0),
        Some(StageCount::Failed(CoreError::ReadOnly))
    ));
    assert_eq!(one(&commands).1, 1);
}

#[test]
fn an_answer_with_another_number_stage_or_collection_is_ignored() {
    let (s, id) = drafted();
    let (s, request) = started(s);
    let before = s.aggregate.counts.clone();
    let (s, c) = ev(s, answer(id, request + 9, 0, Ok(1)));
    assert!(c.is_empty());
    assert_eq!(s.aggregate.counts, before);
    let (s, c) = ev(s, answer(id, request, 1, Ok(1)));
    assert!(c.is_empty());
    assert_eq!(s.aggregate.counts, before, "wrong stage");
    let elsewhere = Event::AggregateCounted {
        session: id,
        request,
        ns: Namespace::new("app", "orders"),
        stage_index: 0,
        outcome: Ok(1),
    };
    let (s, c) = ev(s, elsewhere);
    assert!(c.is_empty());
    assert_eq!(s.aggregate.counts, before, "wrong collection");
    let (_, c) = ev(s, answer(Uuid::from_u128(77), request, 0, Ok(1)));
    assert!(c.is_empty(), "wrong session");
}

/// A sequence with stage 0 answered and stage 1 awaited: the state and that request.
fn midway() -> (AppState, Uuid, u64) {
    let (s, id) = drafted();
    let (s, first) = started(s);
    let (s, commands) = ev(s, answer(id, first, 0, Ok(5)));
    (s, id, one(&commands).0)
}

#[test]
fn a_draft_change_ends_the_sequence_keeps_the_results_and_a_late_answer_changes_nothing() {
    let changes = [
        A::Edit(0, "{ \"$match\": { \"a\": 1 } }".into()),
        A::Toggle(1),
        A::MoveUp(2),
        A::Remove(2),
        A::AddStage("$skip".into()),
    ];
    for change in changes {
        let (s, id, second) = midway();
        let (s, _) = act(s, change);
        assert!(!s.aggregate.counts.is_running());
        assert!(s.aggregate.counts.is_out_of_date(&s.aggregate.stages));
        let before = s.aggregate.counts.clone();
        let (after, c) = ev(s, answer(id, second, 1, Ok(9)));
        assert!(c.is_empty(), "no next stage after a late answer");
        assert_eq!(after.aggregate.counts, before);
    }
    let (s, _, _) = midway();
    let (s, _) = act(s, A::Edit(0, "{ \"$match\": { \"a\": 1 } }".into()));
    assert_eq!(result_of(&s, 0), Some(StageCount::Exact(5)), "still shown");
}

#[test]
fn choosing_another_stage_to_preview_does_not_end_the_sequence() {
    let (s, _) = drafted();
    let (s, request) = started(s);
    let (s, _) = act(s, A::Select(1));
    assert!(s.aggregate.counts.awaits(request));
}

#[test]
fn cancel_ends_the_sequence_and_keeps_the_numbers_so_far() {
    let (s, id) = drafted();
    let (s, first) = started(s);
    let (s, commands) = ev(s, answer(id, first, 0, Ok(5)));
    let (second, _) = one(&commands);
    let (s, c) = counts(s, CountsAction::Cancel);
    assert!(c.is_empty());
    assert!(!s.aggregate.counts.is_running());
    assert_eq!(result_of(&s, 0), Some(StageCount::Exact(5)));
    assert!(!s.aggregate.counts.is_out_of_date(&s.aggregate.stages));
    let (_, c) = ev(s, answer(id, second, 1, Ok(2)));
    assert!(c.is_empty());
}

#[test]
fn another_collection_forgets_the_counts() {
    let (s, id) = drafted();
    let (s, first) = started(s);
    let (s, _) = ev(s, answer(id, first, 0, Ok(5)));
    let (s, _) = update(
        s,
        Input::Sidebar(SidebarAction::Open(Namespace::new("app", "orders"))),
    );
    assert!(!s.aggregate.counts.is_running());
    assert_eq!(result_of(&s, 0), None);
}

#[test]
fn a_lost_server_fails_the_session_and_ends_the_sequence() {
    for error in [CoreError::Unreachable, CoreError::TlsHandshake] {
        let (s, id) = drafted();
        let (s, request) = started(s);
        let (s, commands) = ev(s, failed_event(id, request, error));
        assert!(commands.is_empty(), "nothing else is sent to a dead server");
        assert_eq!(s.status, ConnStatus::Failed);
        assert_eq!(s.aggregate.stages.len(), 3, "the draft survives");
        assert!(!s.aggregate.counts.is_running());
    }
}

#[test]
fn a_loss_for_a_request_nobody_awaits_changes_nothing() {
    let (s, id) = drafted();
    let (s, request) = started(s);
    let (s, _) = ev(s, failed_event(id, request + 7, CoreError::Unreachable));
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(s.aggregate.counts.awaits(request));
}

#[test]
fn a_superseded_count_ends_the_sequence_without_a_message() {
    let (s, id) = drafted();
    let (s, request) = started(s);
    let (s, commands) = ev(s, failed_event(id, request, CoreError::Superseded));
    assert!(commands.is_empty());
    assert!(!s.aggregate.counts.is_running());
    assert!(s.banner.is_none());
}

#[test]
fn a_write_stage_is_never_sent_and_says_it_is_not_counted() {
    let (s, id) = drafted();
    let s = act(s, A::Edit(1, "{ \"$out\": \"x\" }".into())).0;
    let (s, first) = started(s);
    let (s, commands) = ev(s, answer(id, first, 0, Ok(3)));
    assert!(commands.is_empty(), "stages 1 and 2 are not sent");
    assert_eq!(
        result_of(&s, 1),
        Some(StageCount::NotCounted(NotCounted::WriteStage))
    );
    assert_eq!(
        result_of(&s, 2),
        Some(StageCount::NotCounted(NotCounted::WriteStage))
    );
    assert!(!s.aggregate.counts.is_running());
}

#[test]
fn disabled_stages_are_skipped_and_show_nothing() {
    let (s, id) = drafted();
    let s = act(s, A::Toggle(1)).0;
    let (s, first) = started(s);
    let (s, commands) = ev(s, answer(id, first, 0, Ok(3)));
    let (_, stage) = one(&commands);
    assert_eq!(stage, 2);
    assert_eq!(result_of(&s, 1), None);
}

#[test]
fn an_ordinary_action_does_not_copy_the_results() {
    let (s, id) = drafted();
    let (s, first) = started(s);
    let (s, _) = ev(s, answer(id, first, 0, Ok(3)));
    let before = s.aggregate.counts.clone();
    let s = act(s, A::Select(0)).0;
    let s = act(s, A::Select(2)).0;
    assert!(before.shares_results_with(&s.aggregate.counts));
}

#[test]
fn switching_to_another_session_ends_the_sequence() {
    let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let (s, _) = drafted();
    let s = AppState {
        open: vec![a],
        active: Some(a),
        ..s
    };
    let view_b = SessionView {
        status: ConnStatus::Connected,
        ..SessionView::default()
    };
    let s = s.opened_with(b, view_b).activated(a);
    let (s, _) = counts(s, CountsAction::Start);
    assert!(s.aggregate.counts.is_running());
    let s = super::switched_to(s, b);
    assert_eq!(s.active, Some(b));
    assert!(!s.parked[&a].aggregate.counts.is_running());
    let again = super::switched_to(s, b);
    assert_eq!(again.active, Some(b), "already in view");
}

#[test]
fn an_answer_for_a_parked_session_lands_in_its_own_draft() {
    let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let draft = AggregateState::default().add_stage("$sort");
    let plan =
        crate::viewmodel::aggregate_counts_plan::count_plan(&draft, &draft.stages).expect("a plan");
    let (ui, send) = draft.counts.clone().begun(plan, &draft.stages, &draft.keys);
    let request = send.expect("sent").request;
    let mut draft_b = draft;
    draft_b.counts = ui;
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
    let (s, c) = ev(s, answer(b, request, 0, Ok(4)));
    assert!(c.is_empty());
    assert_eq!(s.active, Some(a));
    assert_eq!(*s.aggregate, mine, "the active draft is untouched");
    let parked = &s.parked[&b].aggregate;
    assert_eq!(
        parked.counts.result_of(parked.keys[0]),
        Some(&StageCount::Exact(4))
    );
}
