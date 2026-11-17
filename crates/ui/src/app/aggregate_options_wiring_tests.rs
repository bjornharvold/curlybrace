//! Every operation that sends a pipeline sends the draft's applied options, and none sends
//! anything while the form has a problem.
use super::OptionsAction as O;
use super::tests::{act, connected};
use crate::app::AggregateAction as A;
use crate::app::reducer::{Input, Out, update};
use crate::app::state::AppState;
use crate::app::{CodeAction, CodeSource, CountsAction, ExplainAction};
use crate::viewmodel::aggregate_options::OptionsEdit;
use crate::viewmodel::aggregate_run::RunAvailability;
use crate::viewmodel::aggregate_run::RunDisabled;
use crate::viewmodel::code_export::{CodeBlock, stages_exported};
use curlybrace_core::pipeline::{PipelineOptions, ReadPreferenceMode};
use curlybrace_core::runtime::{Command, EMBEDDED_SESSION, Event};
use curlybrace_core::store::Namespace;

fn people() -> Namespace {
    Namespace::new("app", "people")
}

fn edit(s: AppState, edit: OptionsEdit) -> Out {
    act(s, A::Options(O::Edit(edit)))
}

/// Answers the preview in flight (and any that waited), until none is out.
fn settled(mut s: AppState) -> AppState {
    while let Some(request) = s.aggregate.in_flight {
        let event = Event::AggregatePreviewed {
            session: EMBEDDED_SESSION,
            request,
            outcome: Ok(Vec::new()),
        };
        s = update(s, Input::Event(event)).0;
    }
    s
}

/// `$match`, `$sort`, `$out` drafted and settled.
fn drafted() -> AppState {
    let s = ["$match", "$sort", "$out"]
        .into_iter()
        .fold(connected(), |s, name| act(s, A::AddStage(name.into())).0);
    let s = act(s, A::Edit(2, r#"{ "$out": "dest" }"#.into())).0;
    settled(s)
}

/// `drafted()` with a read preference and a comment applied.
fn optioned() -> AppState {
    let s = edit(
        drafted(),
        OptionsEdit::ReadPreference(Some(ReadPreferenceMode::Nearest)),
    )
    .0;
    let s = settled(s);
    let s = edit(s, OptionsEdit::Comment("report".into())).0;
    settled(act(s, A::Options(O::Apply)).0)
}

fn wanted() -> PipelineOptions {
    PipelineOptions {
        read_preference: Some(ReadPreferenceMode::Nearest),
        comment: Some("report".into()),
        ..PipelineOptions::default()
    }
}

fn counts_of(commands: &[Command]) -> Vec<(u64, usize, PipelineOptions)> {
    commands
        .iter()
        .filter_map(|c| match c {
            Command::AggregateCount {
                request,
                stage_index,
                options,
                ..
            } => Some((*request, *stage_index, options.clone())),
            _ => None,
        })
        .collect()
}

fn counted(request: u64, stage_index: usize) -> Event {
    Event::AggregateCounted {
        session: EMBEDDED_SESSION,
        request,
        ns: people(),
        stage_index,
        outcome: Ok(3),
    }
}

fn explains(commands: &[Command]) -> Vec<PipelineOptions> {
    commands
        .iter()
        .filter_map(|c| match c {
            Command::AggregateExplain { options, .. } => Some(options.clone()),
            _ => None,
        })
        .collect()
}

fn runs(commands: &[Command]) -> Vec<PipelineOptions> {
    commands
        .iter()
        .filter_map(|c| match c {
            Command::AggregateRun { options, .. } => Some(options.clone()),
            _ => None,
        })
        .collect()
}

/// Run clicked, the target typed, Confirm pressed.
fn run_confirmed(s: AppState) -> Out {
    let (s, _) = act(s, A::RunClicked);
    let (s, _) = act(s, A::RunTyped("dest".into()));
    act(s, A::RunConfirmed)
}

#[test]
fn a_count_sequence_sends_the_options_it_began_with() {
    let (s, commands) = act(optioned(), A::Counts(CountsAction::Start));
    let first = counts_of(&commands);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].2, wanted());
    let (_, commands) = update(s, Input::Event(counted(first[0].0, first[0].1)));
    let second = counts_of(&commands);
    assert_eq!(second.len(), 1, "the next stage follows");
    assert_eq!(second[0].2, wanted(), "the sequence keeps its options");
}

#[test]
fn a_blocking_apply_ends_a_running_count_sequence() {
    let (s, commands) = act(optioned(), A::Counts(CountsAction::Start));
    let first = counts_of(&commands);
    assert_eq!(first.len(), 1);
    let (s, _) = edit(s, OptionsEdit::MaxTime("abc".into()));
    let (s, _) = act(s, A::Options(O::Apply));
    assert!(!s.aggregate.counts.is_running(), "the sequence ends");
    let (_, commands) = update(s, Input::Event(counted(first[0].0, first[0].1)));
    assert!(counts_of(&commands).is_empty(), "no next stage is sent");
}

#[test]
fn an_explain_carries_the_draft_options() {
    let (_, commands) = act(optioned(), A::Explain(ExplainAction::Run));
    assert_eq!(explains(&commands), [wanted()]);
}

#[test]
fn a_confirmed_run_carries_the_draft_options() {
    let (_, commands) = run_confirmed(optioned());
    assert_eq!(runs(&commands), [wanted()]);
}

#[test]
fn an_options_change_closes_the_run_dialog() {
    let (s, _) = act(optioned(), A::RunClicked);
    assert!(s.aggregate.run.dialog.is_some());
    let (s, _) = edit(s, OptionsEdit::ReadPreference(None));
    assert!(s.aggregate.run.dialog.is_none());
}

#[test]
fn a_blocked_form_refuses_count_explain_and_run() {
    let (s, _) = edit(drafted(), OptionsEdit::Comment("  ".into()));
    let (s, commands) = act(s, A::Counts(CountsAction::Start));
    assert!(commands.is_empty());
    assert!(!s.aggregate.counts.is_running());
    let (s, commands) = act(s, A::Explain(ExplainAction::Run));
    assert!(commands.is_empty());
    assert_eq!(
        s.run_availability(),
        RunAvailability::Disabled(RunDisabled::Options)
    );
    let (s, commands) = act(s, A::RunClicked);
    assert!(commands.is_empty());
    assert!(s.aggregate.run.dialog.is_none());
}

#[test]
fn confirm_re_checks_the_form_before_it_sends() {
    let (s, _) = act(drafted(), A::RunClicked);
    let (s, _) = act(s, A::RunTyped("dest".into()));
    assert!(s.aggregate.run.dialog.is_some());
    let (s, _) = edit(s, OptionsEdit::MaxTime("0".into()));
    let (_, commands) = act(s, A::RunConfirmed);
    assert!(runs(&commands).is_empty());
}

#[test]
fn copy_as_code_is_blocked_by_an_invalid_form() {
    let (s, _) = edit(drafted(), OptionsEdit::Comment("  ".into()));
    assert_eq!(
        stages_exported(&s.aggregate, &s.aggregate.stages),
        Err(CodeBlock::Options)
    );
    let (s, _) = act(s, A::Code(CodeAction::Open(CodeSource::Aggregate)));
    assert!(s.code_export.dialog.is_none(), "the dialog does not open");
}

#[test]
fn the_default_options_send_what_4a_sent() {
    let (_, commands) = act(drafted(), A::Refresh);
    let previewed: Vec<_> = commands
        .iter()
        .filter_map(|c| match c {
            Command::AggregatePreview { options, .. } => Some(options.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(previewed, [PipelineOptions::default()]);
    let (s, commands) = act(drafted(), A::Counts(CountsAction::Start));
    assert!(!counts_of(&commands).is_empty());
    assert!(counts_of(&commands).iter().all(|(_, _, o)| o.is_empty()));
    let (_, commands) = act(s, A::Explain(ExplainAction::Run));
    assert_eq!(explains(&commands), [PipelineOptions::default()]);
    let (_, commands) = run_confirmed(drafted());
    assert_eq!(runs(&commands), [PipelineOptions::default()]);
}
