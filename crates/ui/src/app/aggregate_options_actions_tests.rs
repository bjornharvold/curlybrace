//! The Options section in the reducer: a pick applies at once and previews with the new options,
//! typed text waits for Apply, an invalid form sends nothing and keeps the last good preview.
use super::OptionsAction as O;
use crate::app::AggregateAction as A;
use crate::app::reducer::{Input, Out, update};
use crate::app::state::{AppState, ConnStatus};
use crate::viewmodel::aggregate::{PreviewError, PreviewState};
use crate::viewmodel::aggregate_options::{OptionsEdit, Tri};
use bson::{Document, doc};
use curlybrace_core::pipeline::{PipelineOptions, ReadPreferenceMode};
use curlybrace_core::runtime::{Command, EMBEDDED_SESSION, Event};
use curlybrace_core::store::Namespace;

fn ns() -> Namespace {
    Namespace::new("app", "people")
}

/// Connected embedded session with `app.people` selected and a `$match` stage drafted.
pub(super) fn connected() -> AppState {
    let mut s = AppState::new(vec![]);
    s.open = vec![EMBEDDED_SESSION];
    s.active = Some(EMBEDDED_SESSION);
    s.status = ConnStatus::Connected;
    s.sidebar = s
        .sidebar
        .with_databases(vec!["app".into()])
        .with_collections("app", vec!["people".into()])
        .toggled("app")
        .selecting(ns());
    s
}

pub(super) fn act(s: AppState, action: A) -> Out {
    update(s, Input::Aggregate(action))
}

fn opt(s: AppState, action: O) -> Out {
    act(s, A::Options(action))
}

fn edit(s: AppState, edit: OptionsEdit) -> Out {
    opt(s, O::Edit(edit))
}

/// The preview commands in `commands`: request and the options they carry.
fn previews(commands: &[Command]) -> Vec<(u64, PipelineOptions)> {
    commands
        .iter()
        .filter_map(|c| match c {
            Command::AggregatePreview {
                request, options, ..
            } => Some((*request, options.clone())),
            _ => None,
        })
        .collect()
}

fn drafted() -> AppState {
    let (s, commands) = act(connected(), A::AddStage("$match".into()));
    let request = previews(&commands)[0].0;
    answered(s, request).0
}

fn answered(s: AppState, request: u64) -> Out {
    let event = Event::AggregatePreviewed {
        session: EMBEDDED_SESSION,
        request,
        outcome: Ok(vec![doc! { "a": 1 }] as Vec<Document>),
    };
    update(s, Input::Event(event))
}

#[test]
fn a_discrete_edit_previews_with_the_new_options() {
    let (s, commands) = edit(
        drafted(),
        OptionsEdit::ReadPreference(Some(ReadPreferenceMode::Nearest)),
    );
    let sent = previews(&commands);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1.read_preference, Some(ReadPreferenceMode::Nearest));
    assert_eq!(s.aggregate.options.set_count(), 1);
}

#[test]
fn a_typed_edit_sends_nothing_until_apply() {
    let (s, commands) = edit(drafted(), OptionsEdit::Comment("nightly".into()));
    assert!(commands.is_empty());
    assert!(s.aggregate.options.is_empty(), "not applied yet");
    let (s, commands) = opt(s, O::Apply);
    let sent = previews(&commands);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1.comment.as_deref(), Some("nightly"));
    assert_eq!(s.aggregate.options.comment.as_deref(), Some("nightly"));
}

#[test]
fn apply_with_an_invalid_form_sends_nothing_and_shows_the_block() {
    let (s, _) = edit(drafted(), OptionsEdit::Comment("   ".into()));
    let (s, commands) = opt(s, O::Apply);
    assert!(commands.is_empty());
    assert!(matches!(
        s.aggregate.preview,
        PreviewState::Failed {
            error: PreviewError::Options,
            last: Some(_)
        }
    ));
}

#[test]
fn an_invalid_form_drops_the_queued_preview() {
    // One preview in flight, a newer one queued behind it.
    let (s, commands) = act(connected(), A::AddStage("$match".into()));
    let first = previews(&commands)[0].0;
    let (s, commands) = act(s, A::AddStage("$sort".into()));
    assert!(commands.is_empty(), "the slot is taken");
    assert!(s.aggregate.queued.is_some());
    let (s, _) = edit(s, OptionsEdit::Comment("  ".into()));
    let (s, commands) = opt(s, O::Apply);
    assert!(commands.is_empty());
    assert!(s.aggregate.queued.is_none());
    let (s, commands) = answered(s, first);
    assert!(
        commands.is_empty(),
        "the old options are not sent behind the block"
    );
    assert!(matches!(
        s.aggregate.preview,
        PreviewState::Failed {
            error: PreviewError::Options,
            ..
        }
    ));
}

#[test]
fn a_stage_edit_while_the_form_is_invalid_sends_nothing() {
    let (s, _) = edit(drafted(), OptionsEdit::MaxTime("0".into()));
    let (s, commands) = act(s, A::AddStage("$sort".into()));
    assert!(commands.is_empty());
    assert!(matches!(
        s.aggregate.preview,
        PreviewState::Failed {
            error: PreviewError::Options,
            ..
        }
    ));
}

#[test]
fn fixing_the_form_previews_again() {
    let (s, _) = edit(drafted(), OptionsEdit::Comment("  ".into()));
    let (s, _) = opt(s, O::Apply);
    // The fix leaves the options as they were applied: still a new preview.
    let (s, _) = edit(s, OptionsEdit::Comment(String::new()));
    let (_, commands) = opt(s, O::Apply);
    let sent = previews(&commands);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1, PipelineOptions::default());
}

#[test]
fn reset_previews_without_options() {
    let (s, _) = edit(drafted(), OptionsEdit::AllowDiskUse(Tri::On));
    let request = s.aggregate.in_flight.expect("sent");
    let (s, _) = answered(s, request);
    let (_, commands) = opt(s, O::Reset);
    let sent = previews(&commands);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].1.is_empty());
}

#[test]
fn toggle_sends_nothing() {
    let before = drafted();
    let request = before.aggregate.request;
    let (s, commands) = opt(before, O::Toggle);
    assert!(commands.is_empty());
    assert!(s.aggregate.options_ui.open);
    assert_eq!(s.aggregate.request, request);
}

#[test]
fn options_work_on_a_read_only_session() {
    let mut s = drafted();
    s.read_only = true;
    let (s, commands) = edit(
        s,
        OptionsEdit::ReadPreference(Some(ReadPreferenceMode::Secondary)),
    );
    assert_eq!(previews(&commands).len(), 1);
    assert!(s.banner.is_none(), "no read-only refusal for a read");
}

#[test]
fn a_production_preview_with_options_needs_no_confirmation() {
    let mut s = drafted();
    s.production = true;
    let (s, commands) = edit(s, OptionsEdit::AllowDiskUse(Tri::On));
    assert_eq!(previews(&commands).len(), 1);
    assert!(s.write_warning.is_none());
    assert!(s.banner.is_none());
}

#[test]
fn options_actions_in_a_batch_send_one_preview() {
    let batch = A::Batch(vec![
        A::Options(O::Edit(OptionsEdit::MaxTime("5000".into()))),
        A::Options(O::Apply),
        A::Options(O::Edit(OptionsEdit::Comment("c".into()))),
        A::Options(O::Apply),
    ]);
    let (s, commands) = act(drafted(), batch);
    let sent = previews(&commands);
    assert_eq!(sent.len(), 1, "{commands:?}");
    assert_eq!(sent[0].1.max_time_ms, Some(5000));
    assert_eq!(sent[0].1.comment.as_deref(), Some("c"));
    assert_eq!(s.aggregate.options.set_count(), 2);
}

#[test]
fn applying_equal_options_sends_nothing() {
    let (_, commands) = opt(drafted(), O::Apply);
    assert!(commands.is_empty());
}
