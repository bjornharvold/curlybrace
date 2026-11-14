//! The confirmed write run in the reducer: the typed confirmation gates the one command, a
//! read-only connection refuses it, anything that changes what is confirmed closes the dialog,
//! and the answers become the run's status.
use super::AggregateAction as A;
use crate::app::reducer::{Input, Out, update};
use crate::app::state::{AppState, Banner, ConnStatus};
use crate::viewmodel::AggregateState;
use crate::viewmodel::SidebarAction;
use crate::viewmodel::aggregate_run::{RunAvailability, RunDisabled, RunStatus};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, EMBEDDED_SESSION, Event};
use curlybrace_core::store::Namespace;
use uuid::Uuid;

const MATCH: &str = r#"{ "$match": {} }"#;
const OUT: &str = r#"{ "$out": "dest" }"#;

fn people() -> Namespace {
    Namespace::new("app", "people")
}

fn dest() -> Namespace {
    Namespace::new("app", "dest")
}

fn act(s: AppState, action: A) -> Out {
    update(s, Input::Aggregate(action))
}

fn ev(s: AppState, event: Event) -> Out {
    update(s, Input::Event(event))
}

fn draft(texts: &[&str]) -> AggregateState {
    texts
        .iter()
        .enumerate()
        .fold(AggregateState::default(), |d, (i, text)| {
            d.add_stage("$match").edit(i, text)
        })
}

/// A connected embedded session with `app.people` selected and the pipeline `texts` drafted.
fn drafted(texts: &[&str]) -> AppState {
    let mut s = AppState::new(vec![]);
    s.open = vec![EMBEDDED_SESSION];
    s.active = Some(EMBEDDED_SESSION);
    s.status = ConnStatus::Connected;
    s.sidebar = s
        .sidebar
        .with_databases(vec!["app".into()])
        .with_collections("app", vec!["people".into()])
        .toggled("app")
        .selecting(people());
    s.aggregate = Box::new(draft(texts));
    s
}

fn ready() -> AppState {
    drafted(&[MATCH, OUT])
}

/// `ready()` with the dialog open and `dest` typed.
fn confirming() -> AppState {
    let (s, _) = act(ready(), A::RunClicked);
    act(s, A::RunTyped("dest".into())).0
}

/// `confirming()` after Confirm: the run is in flight.
fn running() -> AppState {
    act(confirming(), A::RunConfirmed).0
}

fn ran(run: u64, outcome: Result<Option<u64>, CoreError>) -> Event {
    Event::AggregateRan {
        session: EMBEDDED_SESSION,
        run,
        ns: people(),
        target: dest(),
        outcome,
    }
}

fn run_commands(commands: &[Command]) -> Vec<&Command> {
    commands
        .iter()
        .filter(|c| matches!(c, Command::AggregateRun { .. }))
        .collect()
}

#[test]
fn clicking_run_opens_the_dialog_and_asks_for_the_target_count() {
    let (s, commands) = act(ready(), A::RunClicked);
    let dialog = s.aggregate.run.dialog.clone().expect("dialog");
    assert_eq!(dialog.source, people());
    assert_eq!(dialog.target.collection, "dest");
    assert_eq!(dialog.stage, "$out");
    assert!(matches!(
        commands.as_slice(),
        [Command::EstimatedCount { session, ns }] if *session == EMBEDDED_SESSION && *ns == dest()
    ));
    assert!(run_commands(&commands).is_empty());
}

#[test]
fn the_count_is_asked_in_the_targets_database() {
    let s = drafted(&[
        MATCH,
        r#"{ "$out": { "db": "warehouse", "coll": "dest" } }"#,
    ]);
    let (_, commands) = act(s, A::RunClicked);
    assert!(matches!(
        commands.as_slice(),
        [Command::EstimatedCount { ns, .. }] if *ns == Namespace::new("warehouse", "dest")
    ));
}

#[test]
fn the_count_answer_fills_the_dialog() {
    let (s, _) = act(ready(), A::RunClicked);
    let (s, commands) = ev(
        s,
        Event::EstimatedCount {
            session: EMBEDDED_SESSION,
            ns: dest(),
            count: 12,
        },
    );
    assert!(commands.is_empty());
    assert_eq!(s.aggregate.run.dialog.unwrap().target_count, Some(12));
}

#[test]
fn the_count_of_another_collection_does_not_fill_the_dialog() {
    let (s, _) = act(ready(), A::RunClicked);
    let (s, _) = ev(
        s,
        Event::EstimatedCount {
            session: EMBEDDED_SESSION,
            ns: people(),
            count: 99,
        },
    );
    assert_eq!(s.aggregate.run.dialog.unwrap().target_count, None);
}

#[test]
fn a_read_pipeline_has_no_run() {
    let s = drafted(&[MATCH]);
    assert_eq!(s.run_availability(), RunAvailability::Hidden);
    let (s, commands) = act(s, A::RunClicked);
    assert!(s.aggregate.run.dialog.is_none());
    assert!(commands.is_empty());
}

#[test]
fn run_on_a_read_only_connection_shows_the_banner_and_sends_nothing() {
    let mut s = ready();
    s.read_only = true;
    assert_eq!(
        s.run_availability(),
        RunAvailability::Disabled(RunDisabled::ReadOnly)
    );
    let (s, commands) = act(s, A::RunClicked);
    assert_eq!(s.banner, Some(Banner::ReadOnly));
    assert!(s.aggregate.run.dialog.is_none());
    assert!(commands.is_empty());
}

#[test]
fn a_connection_locked_after_the_dialog_opened_is_refused() {
    let mut s = confirming();
    s.read_only = true;
    let (s, commands) = act(s, A::RunConfirmed);
    assert_eq!(s.banner, Some(Banner::ReadOnly));
    assert!(s.aggregate.run.dialog.is_none());
    assert!(s.pending.is_none());
    assert!(commands.is_empty());
}

#[test]
fn typing_edits_the_confirmation_text() {
    let (s, _) = act(ready(), A::RunClicked);
    let (s, commands) = act(s, A::RunTyped("de".into()));
    assert_eq!(s.aggregate.run.dialog.unwrap().typed, "de");
    assert!(commands.is_empty());
    // Without a dialog the typing goes nowhere.
    let (s, _) = act(ready(), A::RunTyped("dest".into()));
    assert!(s.aggregate.run.dialog.is_none());
}

#[test]
fn a_typed_name_that_does_not_match_sends_nothing() {
    for typed in ["", "des", "Dest", "dest2", "de st"] {
        let (s, _) = act(ready(), A::RunClicked);
        let (s, _) = act(s, A::RunTyped(typed.into()));
        let (s, commands) = act(s, A::RunConfirmed);
        assert!(commands.is_empty(), "{typed:?}");
        assert!(s.aggregate.run.dialog.is_some(), "{typed:?}");
        assert_eq!(s.aggregate.run.status, RunStatus::Idle);
        assert!(s.pending.is_none());
    }
}

#[test]
fn confirming_without_a_dialog_sends_nothing() {
    let (s, commands) = act(ready(), A::RunConfirmed);
    assert!(commands.is_empty());
    assert!(s.pending.is_none());
}

#[test]
fn the_matching_name_sends_exactly_one_run() {
    let (s, commands) = act(confirming(), A::RunConfirmed);
    let pipeline = vec![bson::doc! { "$match": {} }, bson::doc! { "$out": "dest" }];
    assert!(matches!(
        commands.as_slice(),
        [Command::AggregateRun { session, ns, pipeline: sent, run: 1 }]
            if *session == EMBEDDED_SESSION && *ns == people() && *sent == pipeline
    ));
    assert_eq!(s.aggregate.run.status, RunStatus::Running);
    assert!(s.aggregate.run.dialog.is_none());
    assert!(s.pending.is_some());
    assert!(s.write_warning.is_none());
}

#[test]
fn the_typed_name_is_trimmed() {
    let (s, _) = act(ready(), A::RunClicked);
    let (s, _) = act(s, A::RunTyped("  dest\n".into()));
    let (_, commands) = act(s, A::RunConfirmed);
    assert_eq!(run_commands(&commands).len(), 1);
}

#[test]
fn a_production_connection_confirms_through_the_dialog_alone() {
    let mut s = ready();
    s.production = true;
    let (s, _) = act(s, A::RunClicked);
    let (s, _) = act(s, A::RunTyped("dest".into()));
    let (s, commands) = act(s, A::RunConfirmed);
    assert_eq!(run_commands(&commands).len(), 1);
    assert!(s.write_warning.is_none(), "no second warning");
}

#[test]
fn production_still_needs_the_typed_name() {
    let mut s = ready();
    s.production = true;
    let (s, _) = act(s, A::RunClicked);
    let (s, commands) = act(s, A::RunConfirmed);
    assert!(commands.is_empty());
    assert!(s.write_warning.is_none());
}

#[test]
fn a_second_confirm_while_the_run_is_in_flight_sends_nothing() {
    let s = running();
    let (s, commands) = act(s, A::RunConfirmed);
    assert!(commands.is_empty());
    let (_, commands) = act(s, A::RunClicked);
    assert!(
        commands.is_empty(),
        "Run is disabled while one is in flight"
    );
}

#[test]
fn another_write_in_flight_blocks_the_run() {
    let mut s = confirming();
    s.pending = Some(crate::app::state::Pending {
        ns: people(),
        editor: None,
    });
    let (s, commands) = act(s, A::RunConfirmed);
    assert!(commands.is_empty());
    assert_eq!(s.aggregate.run.status, RunStatus::Idle);
}

#[test]
fn a_dialog_for_another_collection_runs_nothing() {
    let mut forced = confirming();
    // The collection switch itself closes the dialog; force the mismatch by selecting directly.
    forced.sidebar = forced.sidebar.selecting(Namespace::new("app", "orders"));
    let (forced, commands) = act(forced, A::RunConfirmed);
    assert!(commands.is_empty());
    assert!(forced.pending.is_none());
}

#[test]
fn a_pipeline_changed_behind_the_dialog_runs_nothing() {
    let mut s = confirming();
    // Past `edit` (which closes the dialog): the stages changed without the dialog noticing.
    let mut changed = (*s.aggregate).clone();
    changed.stages[1].text = r#"{ "$out": "elsewhere" }"#.into();
    s.aggregate = Box::new(changed);
    assert!(s.aggregate.run.dialog.is_some());
    let (_, commands) = act(s, A::RunConfirmed);
    assert!(commands.is_empty());
}

#[test]
fn dismissing_closes_the_dialog() {
    let (s, _) = act(confirming(), A::RunDismissed);
    assert!(s.aggregate.run.dialog.is_none());
    assert_eq!(s.aggregate.run.status, RunStatus::Idle);
}

#[test]
fn editing_the_pipeline_closes_the_dialog() {
    let (s, _) = act(
        confirming(),
        A::Edit(0, r#"{ "$match": { "a": 1 } }"#.into()),
    );
    assert!(s.aggregate.run.dialog.is_none());
}

#[test]
fn switching_collection_closes_the_dialog() {
    let (s, _) = update(
        confirming(),
        Input::Sidebar(SidebarAction::Open(Namespace::new("app", "orders"))),
    );
    assert!(s.aggregate.run.dialog.is_none());
}

fn two_sessions() -> (AppState, Uuid, Uuid) {
    let a = ConnectionConfig::new("a", "mongodb://localhost");
    let b = ConnectionConfig::new("b", "mongodb://localhost");
    let (ida, idb) = (a.id, b.id);
    let mut s = AppState::new(vec![a, b]);
    for id in [ida, idb] {
        s = update(s, Input::Connect(id)).0;
        let epoch = s.epoch_counter;
        s = ev(
            s,
            Event::Connected {
                session: id,
                epoch,
                topology: None,
            },
        )
        .0;
    }
    (s, ida, idb)
}

#[test]
fn switching_session_closes_the_dialog() {
    let (s, ida, idb) = two_sessions();
    let s = update(s, Input::Activate(ida)).0;
    let s = update(s, Input::Sidebar(SidebarAction::Open(people()))).0;
    let s = AppState {
        aggregate: Box::new(draft(&[MATCH, OUT])),
        ..s
    };
    let (s, _) = act(s, A::RunClicked);
    assert!(s.aggregate.run.dialog.is_some());
    let s = update(s, Input::Activate(idb)).0;
    let s = update(s, Input::Activate(ida)).0;
    assert!(s.aggregate.run.dialog.is_none());
}

#[test]
fn losing_the_connection_closes_the_dialog() {
    let (s, _) = ev(
        confirming(),
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "list_collections",
            error: CoreError::Unreachable,
        },
    );
    assert!(s.aggregate.run.dialog.is_none());
}

#[test]
fn success_shows_the_target_and_its_count() {
    let (s, commands) = ev(running(), ran(1, Ok(Some(7))));
    assert_eq!(
        s.aggregate.run.status,
        RunStatus::Done {
            target: dest(),
            count: Some(7)
        }
    );
    assert!(s.pending.is_none());
    assert!(s.banner.is_none());
    // `dest` is new to the sidebar: the lists are read again.
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, Command::ListDatabases { .. }))
    );
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, Command::ListCollections { db, .. } if db == "app"))
    );
}

#[test]
fn a_success_without_a_count_is_still_a_success() {
    let (s, _) = ev(running(), ran(1, Ok(None)));
    assert_eq!(
        s.aggregate.run.status,
        RunStatus::Done {
            target: dest(),
            count: None
        }
    );
}

#[test]
fn a_target_the_sidebar_lists_does_not_refresh_it() {
    let mut s = running();
    s.sidebar = s
        .sidebar
        .with_collections("app", vec!["people".into(), "dest".into()]);
    let (s, commands) = ev(s, ran(1, Ok(Some(7))));
    assert!(commands.is_empty());
    assert!(matches!(s.aggregate.run.status, RunStatus::Done { .. }));
}

#[test]
fn a_failed_run_shows_the_error_inline_and_does_not_refresh_the_sidebar() {
    let error = CoreError::Database("boom".into());
    let (s, commands) = ev(running(), ran(1, Err(error.clone())));
    assert_eq!(s.aggregate.run.status, RunStatus::Failed(error));
    assert!(s.pending.is_none());
    assert!(s.banner.is_none());
    assert!(commands.is_empty());
}

#[test]
fn a_timeout_is_an_unknown_outcome_not_a_failure() {
    let (s, _) = ev(running(), ran(1, Err(CoreError::WriteOutcomeUnknown)));
    assert_eq!(s.aggregate.run.status, RunStatus::Unknown);
    assert!(s.pending.is_none());
}

#[test]
fn an_interrupted_run_is_an_unknown_outcome() {
    let (s, _) = ev(
        running(),
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "aggregate_run",
            error: CoreError::WriteOutcomeUnknown,
        },
    );
    assert_eq!(s.aggregate.run.status, RunStatus::Unknown);
    assert!(s.pending.is_none());
    assert!(s.banner.is_none());
}

#[test]
fn a_lost_server_is_an_unknown_outcome_and_fails_the_session() {
    let (s, _) = ev(
        running(),
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "aggregate_run",
            error: CoreError::Unreachable,
        },
    );
    assert_eq!(s.aggregate.run.status, RunStatus::Unknown);
    assert_eq!(s.status, ConnStatus::Failed);
}

#[test]
fn a_lost_server_with_an_editor_open_is_still_unknown() {
    let mut s = running();
    s.editor = Some(crate::viewmodel::EditorState::for_new());
    let (s, _) = ev(
        s,
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "aggregate_run",
            error: CoreError::Unreachable,
        },
    );
    assert_eq!(s.aggregate.run.status, RunStatus::Unknown);
    assert!(s.pending.is_none());
}

#[test]
fn a_command_refused_as_read_only_shows_the_banner() {
    let (s, _) = ev(
        running(),
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "aggregate_run",
            error: CoreError::ReadOnly,
        },
    );
    assert_eq!(s.banner, Some(Banner::ReadOnly));
    assert_eq!(s.aggregate.run.status, RunStatus::Idle);
    assert!(s.pending.is_none());
}

#[test]
fn another_failure_of_the_command_is_shown_with_the_run() {
    let error = CoreError::Database("nope".into());
    let (s, _) = ev(
        running(),
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "aggregate_run",
            error: error.clone(),
        },
    );
    assert_eq!(s.aggregate.run.status, RunStatus::Failed(error));
}

#[test]
fn a_stale_answer_changes_nothing() {
    let (after, commands) = ev(running(), ran(7, Ok(Some(1))));
    assert_eq!(after.aggregate.run.status, RunStatus::Running);
    assert!(after.pending.is_some());
    assert!(commands.is_empty());
    // An answer after the run was settled does not overwrite it either.
    let (done, _) = ev(after, ran(1, Ok(Some(1))));
    let settled = done.aggregate.run.status.clone();
    let (again, _) = ev(done, ran(1, Err(CoreError::WriteOutcomeUnknown)));
    assert_eq!(again.aggregate.run.status, settled);
}

#[test]
fn an_answer_with_no_run_in_flight_is_dropped() {
    let (s, commands) = ev(ready(), ran(1, Ok(Some(1))));
    assert_eq!(s.aggregate.run.status, RunStatus::Idle);
    assert!(commands.is_empty());
}

#[test]
fn a_run_in_flight_when_the_connection_resets_is_unknown() {
    let s = running();
    let draft = (*s.aggregate).clone().on_connection_reset();
    assert_eq!(draft.run.status, RunStatus::Unknown);
}

const MERGE: &str = r#"{ "$merge": { "into": "dest" } }"#;

fn merging() -> AppState {
    let (s, _) = act(drafted(&[MATCH, MERGE]), A::RunClicked);
    let (s, _) = act(s, A::RunTyped("dest".into()));
    act(s, A::RunConfirmed).0
}

#[test]
fn a_failed_merge_says_the_target_may_hold_part_of_the_result() {
    let error = CoreError::Database("boom".into());
    let (done, commands) = ev(merging(), ran(1, Err(error.clone())));
    assert_eq!(
        done.aggregate.run.status,
        RunStatus::FailedPartial(error.clone())
    );
    assert!(commands.is_empty());
    let (via_command, _) = ev(
        merging(),
        Event::Failed {
            session: EMBEDDED_SESSION,
            context: "aggregate_run",
            error: error.clone(),
        },
    );
    assert_eq!(
        via_command.aggregate.run.status,
        RunStatus::FailedPartial(error)
    );
}

#[test]
fn an_unknown_outcome_refreshes_the_sidebar_like_a_success() {
    let (s, commands) = ev(running(), ran(1, Err(CoreError::WriteOutcomeUnknown)));
    assert_eq!(s.aggregate.run.status, RunStatus::Unknown);
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, Command::ListDatabases { .. }))
    );
}

#[test]
fn a_run_onto_the_open_collection_fetches_the_documents_again() {
    let onto_self = |outcome| {
        let s = drafted(&[MATCH, r#"{ "$out": "people" }"#]);
        let (s, _) = act(s, A::RunClicked);
        let (s, _) = act(s, A::RunTyped("people".into()));
        let (s, _) = act(s, A::RunConfirmed);
        let event = Event::AggregateRan {
            session: EMBEDDED_SESSION,
            run: 1,
            ns: people(),
            target: people(),
            outcome,
        };
        ev(s, event).1
    };
    for outcome in [Ok(Some(3)), Err(CoreError::WriteOutcomeUnknown)] {
        let commands = onto_self(outcome);
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, Command::Find { ns, .. } if *ns == people())),
            "{commands:?}"
        );
    }
    // Another target leaves the open documents alone; a failure does too.
    let (_, commands) = ev(running(), ran(1, Ok(Some(1))));
    assert!(!commands.iter().any(|c| matches!(c, Command::Find { .. })));
    let commands = onto_self(Err(CoreError::Database("x".into())));
    assert!(commands.is_empty());
}

#[test]
fn a_target_in_another_database_needs_the_qualified_name() {
    let open = || {
        let s = drafted(&[
            MATCH,
            r#"{ "$out": { "db": "warehouse", "coll": "dest" } }"#,
        ]);
        act(s, A::RunClicked).0
    };
    let (collection_only, _) = act(open(), A::RunTyped("dest".into()));
    let (_, commands) = act(collection_only, A::RunConfirmed);
    assert!(run_commands(&commands).is_empty());
    let (qualified, _) = act(open(), A::RunTyped("warehouse.dest".into()));
    let (_, commands) = act(qualified, A::RunConfirmed);
    assert_eq!(run_commands(&commands).len(), 1);
}

#[test]
fn a_late_answer_of_an_earlier_run_does_not_settle_the_next_run() {
    let (s, _) = ev(running(), ran(1, Ok(Some(1))));
    let (s, _) = act(s, A::RunClicked);
    let (s, _) = act(s, A::RunTyped("dest".into()));
    let (s, _) = act(s, A::RunConfirmed);
    assert_eq!(s.aggregate.run.run, 2);
    let (s, _) = ev(s, ran(1, Err(CoreError::WriteOutcomeUnknown)));
    assert_eq!(s.aggregate.run.status, RunStatus::Running);
    assert!(s.pending.is_some());
}

#[test]
fn a_target_in_a_server_database_cannot_be_run() {
    let s = drafted(&[MATCH, r#"{ "$out": { "db": "config", "coll": "x" } }"#]);
    assert!(matches!(
        s.run_availability(),
        RunAvailability::Disabled(RunDisabled::Invalid(_))
    ));
    let (s, commands) = act(s, A::RunClicked);
    assert!(s.aggregate.run.dialog.is_none());
    assert!(commands.is_empty());
}

#[test]
fn run_numbers_do_not_repeat() {
    let (s, _) = ev(running(), ran(1, Ok(Some(1))));
    let (s, _) = act(s, A::RunClicked);
    let (s, _) = act(s, A::RunTyped("dest".into()));
    let (_, commands) = act(s, A::RunConfirmed);
    assert!(matches!(
        commands.as_slice(),
        [Command::AggregateRun { run: 2, .. }]
    ));
}

#[test]
fn run_actions_inside_a_batch_apply_in_order() {
    let (s, commands) = act(
        ready(),
        A::Batch(vec![
            A::RunClicked,
            A::RunTyped("dest".into()),
            A::RunConfirmed,
        ]),
    );
    assert_eq!(run_commands(&commands).len(), 1);
    assert_eq!(s.aggregate.run.status, RunStatus::Running);
}
