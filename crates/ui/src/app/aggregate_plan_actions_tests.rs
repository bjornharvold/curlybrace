//! The Plan tab's follow-ups in the reducer: Try an index and Clear hint set the hint through the
//! options path and explain again, Create index opens the index builder pre-filled, and nothing
//! of it goes through while an explain runs, the form is blocked or the connection is read-only.
use crate::app::index_write_tests::{browsing, shown, with_flags};
use crate::app::reducer::{Input, Out, update};
use crate::app::state::{AppState, Banner, ConnStatus};
use crate::app::write_guard::{WriteKind, WriteWarningAction};
use crate::app::{AggregateAction as A, ExplainAction, OptionsAction};
use crate::viewmodel::aggregate_options::OptionsEdit;
use crate::viewmodel::{AggregateState, IndexesAction};
use bson::Document;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::ejson::editor_json_to_doc;
use curlybrace_core::explain::ExplainVerbosity;
use curlybrace_core::pipeline_options::HintOption;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Explain, Namespace};
use curlybrace_core::test_support::mock_objects::mock_connection;
use uuid::Uuid;

fn people() -> Namespace {
    Namespace::new("app", "people")
}

fn act(s: AppState, action: A) -> Out {
    update(s, Input::Aggregate(action))
}

fn request_of(commands: &[Command]) -> u64 {
    commands
        .iter()
        .find_map(|c| match c {
            Command::AggregateExplain { request, .. } => Some(*request),
            _ => None,
        })
        .expect("an explain command")
}

fn collscan() -> Document {
    editor_json_to_doc(include_str!(
        "../../../core/tests/fixtures/explain_aggregate/stages_collscan.json"
    ))
    .expect("fixture parses")
}

fn explained(id: Uuid, request: u64) -> Event {
    Event::AggregateExplained {
        session: id,
        request,
        ns: people(),
        verbosity: ExplainVerbosity::QueryPlanner,
        outcome: Ok(Explain {
            document: collscan(),
            plan_only: true,
        }),
    }
}

/// A connected session whose draft is `$match` + `$sort` (or `first` alone) with a collection scan
/// plan in its Plan tab.
fn with_plan_of(cfg: ConnectionConfig, stages: &[(&str, &str)]) -> (AppState, Uuid) {
    let (mut s, id) = browsing(cfg);
    let draft = stages
        .iter()
        .fold(AggregateState::default(), |draft, (name, text)| {
            let at = draft.stages.len();
            draft.add_stage(name).edit(at, text)
        });
    s.aggregate = Box::new(draft);
    let (s, commands) = act(s, A::Explain(ExplainAction::Run));
    let request = request_of(&commands);
    let (s, _) = update(s, Input::Event(explained(id, request)));
    assert!(s.aggregate.explain.done().is_some(), "the plan arrived");
    (s, id)
}

const MATCH: (&str, &str) = ("$match", r#"{ "$match": { "status": "A" } }"#);
const SORT: (&str, &str) = ("$sort", r#"{ "$sort": { "total": -1 } }"#);

fn with_plan() -> (AppState, Uuid) {
    with_plan_of(mock_connection(), &[MATCH, SORT])
}

fn try_index(s: AppState, name: &str) -> Out {
    act(s, A::Explain(ExplainAction::TryIndex(name.into())))
}

fn hint_of(commands: &[Command]) -> Vec<Option<HintOption>> {
    commands
        .iter()
        .filter_map(|c| match c {
            Command::AggregatePreview { options, .. }
            | Command::AggregateExplain { options, .. } => Some(options.hint.clone()),
            _ => None,
        })
        .collect()
}

fn kinds(commands: &[Command]) -> Vec<&'static str> {
    commands
        .iter()
        .map(|c| match c {
            Command::AggregatePreview { .. } => "preview",
            Command::AggregateExplain { .. } => "explain",
            _ => "other",
        })
        .collect()
}

#[test]
fn try_index_sets_the_hint_and_sends_preview_then_explain() {
    let (s, _) = with_plan();
    let (s, commands) = try_index(s, "status_1");
    assert_eq!(kinds(&commands), ["preview", "explain"]);
    let named = Some(HintOption::Name("status_1".into()));
    assert_eq!(hint_of(&commands), [named.clone(), named.clone()]);
    assert_eq!(s.aggregate.options.hint, named);
    assert_eq!(s.aggregate.options_ui.form.hint_text, "status_1");
    assert!(s.aggregate.explain.is_loading());
}

#[test]
fn try_index_keeps_the_verbosity_selected() {
    let (s, _) = with_plan();
    let (s, _) = act(
        s,
        A::Explain(ExplainAction::SetVerbosity(
            ExplainVerbosity::ExecutionStats,
        )),
    );
    let (_, commands) = try_index(s, "status_1");
    let verbosity = commands.iter().find_map(|c| match c {
        Command::AggregateExplain { verbosity, .. } => Some(*verbosity),
        _ => None,
    });
    assert_eq!(verbosity, Some(ExplainVerbosity::ExecutionStats));
}

#[test]
fn the_same_index_again_only_re_explains() {
    let (s, id) = with_plan();
    let (s, commands) = try_index(s, "status_1");
    let (s, _) = update(s, Input::Event(explained(id, request_of(&commands))));
    let (_, again) = try_index(s, "status_1");
    assert_eq!(kinds(&again), ["explain"]);
}

#[test]
fn clear_hint_sends_both_without_a_hint() {
    let (s, id) = with_plan();
    let (s, commands) = try_index(s, "status_1");
    let (s, _) = update(s, Input::Event(explained(id, request_of(&commands))));
    // The preview of the first change must be answered: one preview is out at a time.
    let preview = commands
        .iter()
        .find_map(|c| match c {
            Command::AggregatePreview { request, .. } => Some(*request),
            _ => None,
        })
        .expect("a preview");
    let answered = Event::AggregatePreviewed {
        session: id,
        request: preview,
        outcome: Ok(Vec::<Document>::new()),
    };
    let (s, _) = update(s, Input::Event(answered));
    let (s, commands) = act(s, A::Explain(ExplainAction::ClearHint));
    assert_eq!(kinds(&commands), ["preview", "explain"]);
    assert_eq!(hint_of(&commands), [None, None]);
    assert_eq!(s.aggregate.options.hint, None);
    assert_eq!(s.aggregate.options_ui.form.hint_text, "");
}

#[test]
fn try_index_does_nothing_while_an_explain_runs() {
    let (s, _) = with_plan();
    let (s, _) = act(s, A::Explain(ExplainAction::Run));
    assert!(s.aggregate.explain.is_loading());
    let (s, commands) = try_index(s, "status_1");
    assert!(commands.is_empty());
    assert_eq!(s.aggregate.options.hint, None);
}

#[test]
fn try_index_does_nothing_while_the_form_is_blocked() {
    let (s, _) = with_plan();
    let (s, _) = act(
        s,
        A::Options(OptionsAction::Edit(OptionsEdit::MaxTime("0".into()))),
    );
    assert!(s.aggregate.options_blocked());
    let (s, commands) = try_index(s, "status_1");
    assert!(commands.is_empty());
    assert_eq!(s.aggregate.options.hint, None);
}

#[test]
fn try_index_and_clear_hint_do_nothing_while_disconnected() {
    let (s, _) = with_plan();
    let s = AppState {
        status: ConnStatus::Disconnected,
        ..s
    };
    let (s, commands) = try_index(s, "status_1");
    assert!(commands.is_empty());
    assert_eq!(s.aggregate.options.hint, None);
    let (_, commands) = act(s, A::Explain(ExplainAction::ClearHint));
    assert!(commands.is_empty());
}

fn create_index(s: AppState) -> Out {
    act(s, A::Explain(ExplainAction::CreateIndex))
}

#[test]
fn create_index_opens_the_builder_pre_filled() {
    let (s, id) = with_plan();
    let (s, commands) = create_index(s);
    let builder = s.indexes.builder.as_ref().expect("the builder is open");
    assert_eq!(builder.ns, people());
    let fields: Vec<&str> = builder.keys.iter().map(|k| k.field.as_str()).collect();
    assert_eq!(fields, ["status", "total"]);
    assert_eq!(builder.name, "status_1_total_-1");
    assert_eq!(
        shown(&commands),
        shown(&[Command::EstimatedCount {
            session: id,
            ns: people()
        }])
    );
    // Nothing is built without the dialog.
    assert!(s.pending.is_none());
}

#[test]
fn create_index_is_refused_on_a_read_only_connection() {
    let (s, _) = with_plan_of(with_flags(false, true), &[MATCH, SORT]);
    let (s, commands) = create_index(s);
    assert!(commands.is_empty());
    assert!(s.indexes.builder.is_none());
    assert_eq!(s.banner, Some(Banner::ReadOnly));
}

#[test]
fn create_index_on_production_opens_and_create_is_guarded() {
    let (s, _) = with_plan_of(with_flags(true, false), &[MATCH, SORT]);
    let (s, _) = create_index(s);
    assert!(s.indexes.builder.is_some());
    let (s, commands) = update(s, Input::Indexes(IndexesAction::Create));
    assert!(commands.is_empty(), "nothing is sent before the answer");
    let parked = s.write_warning.as_ref().expect("the write is parked");
    assert_eq!(parked.kind, WriteKind::CreateIndex);
    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert_eq!(commands.len(), 1);
    assert!(s.pending.is_some());
}

#[test]
fn create_index_without_a_suggestion_does_nothing() {
    let group = ("$group", r#"{ "$group": { "_id": "$status" } }"#);
    let (s, _) = with_plan_of(mock_connection(), &[group]);
    let (s, commands) = create_index(s);
    assert!(commands.is_empty());
    assert!(s.indexes.builder.is_none());
}

/// A session whose hint picker already holds a list, with an index write pending on `ns`.
fn listed_and_writing(ns: &Namespace) -> AppState {
    let (mut s, _) = with_plan();
    let draft = std::mem::take(&mut *s.aggregate);
    *s.aggregate = draft.hint_edited(|list| list.from_names(vec!["a_1".into()]));
    s.pending = Some(crate::app::state::Pending {
        ns: ns.clone(),
        editor: None,
    });
    s
}

#[test]
fn a_created_index_resets_the_hint_list_of_its_collection() {
    let s = listed_and_writing(&people());
    assert!(!s.aggregate.options_ui.hint.wants_list());
    let reset = crate::app::index_write_actions::write_finished(s, &people());
    assert!(reset.aggregate.options_ui.hint.wants_list());
}

#[test]
fn an_index_written_in_another_collection_leaves_the_hint_list_alone() {
    let other = Namespace::new("app", "orders");
    let s = listed_and_writing(&other);
    let untouched = crate::app::index_write_actions::write_finished(s, &other);
    assert!(!untouched.aggregate.options_ui.hint.wants_list());
}
