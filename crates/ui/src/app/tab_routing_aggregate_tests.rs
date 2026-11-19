//! The Aggregate draft of each tab: a preview, a count and a run dialog answer reach the draft
//! that sent them, and the one-preview-in-flight rule holds per tab.
use super::support::{act, ev, orders, parked, two_tabs};
use crate::app::state::{AppState, ConnStatus};
use crate::app::tabs::TabId;
use crate::app::{AggregateAction, CountsAction, ExplainAction};
use crate::viewmodel::CollectionTab;
use crate::viewmodel::aggregate_run::CountState;
use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};

use crate::app::reducer::{Input, update};

#[test]
fn a_preview_answer_goes_to_the_draft_that_sent_it_and_releases_only_its_slot() {
    let t = two_tabs();
    let (s, commands) = act(t.s, t.second, Input::Tab(CollectionTab::Aggregate));
    assert!(commands.is_empty());
    let (s, commands) = act(
        s,
        t.second,
        Input::Aggregate(AggregateAction::AddStage("$match".into())),
    );
    let request = match commands.as_slice() {
        [Command::AggregatePreview { request, .. }] => *request,
        other => panic!("{other:?}"),
    };
    assert_eq!(parked(&s, t.second).aggregate.in_flight, Some(request));
    assert_eq!(s.aggregate.in_flight, None);
    let s = ev(
        s,
        Event::AggregatePreviewed {
            session: t.a,
            request,
            outcome: Ok(vec![doc! { "n": 1 }]),
        },
    );
    assert_eq!(
        parked(&s, t.second).aggregate.in_flight,
        None,
        "its slot is free"
    );
    assert!(matches!(
        parked(&s, t.second).aggregate.preview,
        crate::viewmodel::aggregate::PreviewState::Shown(_)
    ));
    assert!(matches!(
        s.aggregate.preview,
        crate::viewmodel::aggregate::PreviewState::Idle
    ));
}

#[test]
fn the_one_preview_in_flight_rule_holds_per_tab() {
    let t = two_tabs();
    let add = || Input::Aggregate(AggregateAction::AddStage("$match".into()));
    let (s, c1) = update(t.s, Input::Tab(CollectionTab::Aggregate));
    assert!(c1.is_empty());
    let (s, c1) = update(s, add());
    let (s, _) = act(s, t.second, Input::Tab(CollectionTab::Aggregate));
    let (s, c2) = act(s, t.second, add());
    let (r1, r2) = match (c1.as_slice(), c2.as_slice()) {
        (
            [Command::AggregatePreview { request: r1, .. }],
            [Command::AggregatePreview { request: r2, .. }],
        ) => (*r1, *r2),
        other => panic!("each draft sends its own preview: {other:?}"),
    };
    assert_ne!(r1, r2);
    // A second edit of tab two while its preview is out is queued, not sent.
    let (s, c3) = act(
        s,
        t.second,
        Input::Aggregate(AggregateAction::Edit(0, r#"{"$match": {"a": 1}}"#.into())),
    );
    assert!(c3.is_empty(), "one in flight per draft: {c3:?}");
    assert!(parked(&s, t.second).aggregate.queued.is_some());
    // Tab one's answer frees tab one's slot only.
    let s = ev(
        s,
        Event::AggregatePreviewed {
            session: t.a,
            request: r1,
            outcome: Ok(vec![]),
        },
    );
    assert_eq!(s.aggregate.in_flight, None);
    assert_eq!(parked(&s, t.second).aggregate.in_flight, Some(r2));
    assert!(parked(&s, t.second).aggregate.queued.is_some());
}

#[test]
fn a_per_stage_count_answer_goes_to_its_tab() {
    let t = two_tabs();
    let (s, _) = act(t.s, t.second, Input::Tab(CollectionTab::Aggregate));
    let (s, _) = act(
        s,
        t.second,
        Input::Aggregate(AggregateAction::AddStage("$match".into())),
    );
    let (s, commands) = act(
        s,
        t.second,
        Input::Aggregate(AggregateAction::Counts(CountsAction::Start)),
    );
    let (request, stage_index) = commands
        .iter()
        .find_map(|c| match c {
            Command::AggregateCount {
                request,
                stage_index,
                ..
            } => Some((*request, *stage_index)),
            _ => None,
        })
        .expect("a count request");
    assert!(parked(&s, t.second).aggregate.counts.is_running());
    let s = ev(
        s,
        Event::AggregateCounted {
            session: t.a,
            request,
            ns: orders(),
            stage_index,
            outcome: Ok(5),
        },
    );
    assert!(
        !parked(&s, t.second).aggregate.counts.is_running(),
        "tab two's count ended"
    );
    assert!(!s.aggregate.counts.is_running());
}

#[test]
fn a_run_dialog_count_answer_goes_to_its_tab() {
    let t = two_tabs();
    let (s, _) = act(t.s, t.second, Input::Tab(CollectionTab::Aggregate));
    let (s, _) = act(
        s,
        t.second,
        Input::Aggregate(AggregateAction::AddStage("$match".into())),
    );
    let (s, _) = act(
        s,
        t.second,
        Input::Aggregate(AggregateAction::AddStage("$out".into())),
    );
    let (s, _) = act(
        s,
        t.second,
        Input::Aggregate(AggregateAction::Edit(1, r#"{ "$out": "dest" }"#.into())),
    );
    let (s, commands) = act(s, t.second, Input::Aggregate(AggregateAction::RunClicked));
    let (request, ns) = commands
        .iter()
        .find_map(|c| match c {
            Command::EstimatedCount { request, ns, .. } => Some((*request, ns.clone())),
            _ => None,
        })
        .expect("the dialog asks for the target's count");
    let s = ev(
        s,
        Event::EstimatedCount {
            session: t.a,
            ns,
            count: 12,
            request,
        },
    );
    let dialog = parked(&s, t.second)
        .aggregate
        .run
        .dialog
        .as_ref()
        .expect("tab two's dialog");
    assert_eq!(dialog.count, CountState::Known(12));
}

/// A lost server is told apart from a mere failure by looking at the draft the request belongs to:
/// for a parked tab's request that is the parked tab's draft, not the current tab's.
fn lost_in_parked_tab(
    context: &'static str,
    send: impl FnOnce(AppState, TabId) -> (AppState, u64),
) {
    let t = two_tabs();
    let (s, request) = send(t.s, t.second);
    let (s, _) = update(
        s,
        Input::Event(Event::RequestFailed {
            session: t.a,
            context,
            request,
            error: CoreError::Unreachable,
        }),
    );
    assert_eq!(
        s.status,
        ConnStatus::Failed,
        "{context}: the session is lost"
    );
}

#[test]
fn a_lost_server_on_a_parked_tabs_preview_fails_the_root() {
    lost_in_parked_tab("aggregate_preview", |s, tab| {
        let (s, _) = act(s, tab, Input::Tab(CollectionTab::Aggregate));
        let (s, commands) = act(
            s,
            tab,
            Input::Aggregate(AggregateAction::AddStage("$match".into())),
        );
        let request = match commands.as_slice() {
            [Command::AggregatePreview { request, .. }] => *request,
            other => panic!("{other:?}"),
        };
        (s, request)
    });
}

#[test]
fn a_lost_server_on_a_parked_tabs_explain_fails_the_root() {
    lost_in_parked_tab("aggregate_explain", |s, tab| {
        let (s, _) = act(s, tab, Input::Tab(CollectionTab::Aggregate));
        let (s, _) = act(
            s,
            tab,
            Input::Aggregate(AggregateAction::AddStage("$match".into())),
        );
        let (s, commands) = act(
            s,
            tab,
            Input::Aggregate(AggregateAction::Explain(ExplainAction::Run)),
        );
        let request = commands
            .iter()
            .find_map(|c| match c {
                Command::AggregateExplain { request, .. } => Some(*request),
                _ => None,
            })
            .expect("an explain request");
        (s, request)
    });
}
