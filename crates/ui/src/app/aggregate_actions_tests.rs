//! The Aggregate tab in the reducer: every draft change previews the selected stage, answers are
//! matched by request number and session, and failures stay in the preview pane.
use super::AggregateAction as A;
use crate::app::reducer::{Input, update};
use crate::app::session::SessionView;
use crate::app::state::{AppState, ConnStatus};
use crate::viewmodel::AggregateState;
use crate::viewmodel::aggregate::{Mode, Preview, PreviewError, PreviewState};
use bson::{Document, doc};
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::{PREVIEW_LIMIT, PipelineError};
use curlybrace_core::runtime::{Command, EMBEDDED_SESSION, Event};
use curlybrace_core::store::Namespace;
use uuid::Uuid;

fn ns() -> Namespace {
    Namespace::new("app", "people")
}

/// Connected embedded session with `app.people` selected.
fn connected() -> AppState {
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

fn act(s: AppState, action: A) -> (AppState, Vec<Command>) {
    update(s, Input::Aggregate(action))
}

fn previewed(
    s: AppState,
    session: Uuid,
    request: u64,
    outcome: Result<Vec<Document>, CoreError>,
) -> (AppState, Vec<Command>) {
    let event = Event::AggregatePreviewed {
        session,
        request,
        outcome,
    };
    update(s, Input::Event(event))
}

/// The one preview command in `commands`: its session, request and pipeline.
fn the_preview(commands: &[Command]) -> (Uuid, u64, Vec<Document>) {
    match commands {
        [
            Command::AggregatePreview {
                session,
                request,
                ns: sent,
                pipeline,
            },
        ] => {
            assert_eq!(*sent, ns());
            (*session, *request, pipeline.clone())
        }
        other => panic!("expected one AggregatePreview, got {other:?}"),
    }
}

fn limit() -> Document {
    doc! { "$limit": PREVIEW_LIMIT }
}

/// `connected()` with a `$match` and a `$sort` stage, the `$sort` previewed and answered.
fn with_two_stages() -> AppState {
    let (s, _) = act(connected(), A::AddStage("$match".into()));
    let (s, c) = act(s, A::AddStage("$sort".into()));
    let (_, request, _) = the_preview(&c);
    previewed(s, EMBEDDED_SESSION, request, Ok(vec![doc! { "a": 1 }])).0
}

fn shown_docs(s: &AggregateState) -> Option<Vec<Document>> {
    match &s.preview {
        PreviewState::Shown(p) => Some(p.docs.clone()),
        _ => None,
    }
}

#[test]
fn adding_a_stage_previews_it_for_the_active_session() {
    let (s, c) = act(connected(), A::AddStage("$match".into()));
    let (session, request, pipeline) = the_preview(&c);
    assert_eq!(session, EMBEDDED_SESSION);
    assert_eq!(request, s.aggregate.request);
    assert_eq!(pipeline, vec![doc! { "$match": {} }, limit()]);
    assert!(matches!(s.aggregate.preview, PreviewState::Loading { .. }));
}

#[test]
fn each_pipeline_change_previews_the_selected_stage() {
    let cases = [
        A::Remove(0),
        A::MoveUp(1),
        A::MoveDown(0),
        A::Toggle(0),
        A::Edit(0, r#"{ "$match": { "a": 1 } }"#.into()),
        A::Select(0),
        A::Refresh,
    ];
    for action in cases {
        let before = with_two_stages();
        let old = before.aggregate.request;
        let (s, c) = act(before, action.clone());
        let (_, request, pipeline) = the_preview(&c);
        assert!(request > old, "{action:?}");
        assert_eq!(request, s.aggregate.request, "{action:?}");
        assert_eq!(pipeline.last(), Some(&limit()), "{action:?}");
    }
}

#[test]
fn an_edit_sends_the_edited_stage() {
    let (s, c) = act(
        with_two_stages(),
        A::Edit(1, r#"{ "$sort": { "age": -1 } }"#.into()),
    );
    let (_, _, pipeline) = the_preview(&c);
    assert_eq!(
        pipeline,
        vec![
            doc! { "$match": {} },
            doc! { "$sort": { "age": -1 } },
            limit()
        ]
    );
    assert_eq!(s.aggregate.stages[1].text, r#"{ "$sort": { "age": -1 } }"#);
}

#[test]
fn a_change_that_changes_nothing_sends_nothing() {
    let before = (*with_two_stages().aggregate).clone();
    for action in [A::MoveUp(0), A::MoveDown(1), A::Select(1), A::Remove(9)] {
        let (s, c) = act(with_two_stages(), action.clone());
        assert!(c.is_empty(), "{action:?}");
        assert_eq!(*s.aggregate, before, "{action:?}");
    }
}

#[test]
fn a_refused_pipeline_fails_inline_and_sends_nothing() {
    let (s, _) = act(connected(), A::AddStage("$out".into()));
    let (s, c) = act(s, A::AddStage("$match".into()));
    assert!(c.is_empty());
    assert!(matches!(
        s.aggregate.preview,
        PreviewState::Failed {
            error: PreviewError::Pipeline(PipelineError::WriteStageNotLast { .. }),
            ..
        }
    ));
    assert!(s.banner.is_none());
}

#[test]
fn removing_the_last_stage_fails_inline_as_empty() {
    let (s, _) = act(connected(), A::AddStage("$match".into()));
    let (s, c) = act(s, A::Remove(0));
    assert!(c.is_empty());
    assert!(matches!(
        s.aggregate.preview,
        PreviewState::Failed {
            error: PreviewError::Pipeline(PipelineError::Empty),
            ..
        }
    ));
}

#[test]
fn text_mode_edits_the_buffer_only_and_leaving_it_previews_the_parsed_stages() {
    let (s, c) = act(with_two_stages(), A::SetMode(Mode::Text));
    assert!(c.is_empty());
    assert_eq!(s.aggregate.mode, Mode::Text);
    assert!(s.aggregate.text.contains("$sort"));

    let text = r#"[{ "$match": { "a": 2 } }]"#;
    let (s, c) = act(s, A::EditText(text.into()));
    assert!(c.is_empty());
    assert_eq!(s.aggregate.text, text);
    assert_eq!(s.aggregate.stages.len(), 2);

    let (s, c) = act(s, A::SetMode(Mode::Stages));
    let (_, request, pipeline) = the_preview(&c);
    assert_eq!(request, s.aggregate.request);
    assert_eq!(pipeline, vec![doc! { "$match": { "a": 2 } }, limit()]);
    assert_eq!(s.aggregate.mode, Mode::Stages);
}

#[test]
fn leaving_unchanged_text_keeps_the_stages_and_sends_nothing() {
    let (s, _) = act(with_two_stages(), A::Toggle(0));
    let before = s.aggregate.clone();
    let (s, _) = act(s, A::SetMode(Mode::Text));
    let (s, c) = act(s, A::SetMode(Mode::Stages));
    assert!(c.is_empty(), "{c:?}");
    assert_eq!(s.aggregate.stages, before.stages);
    assert_eq!(s.aggregate.selected, before.selected);
    assert_eq!(s.aggregate.mode, Mode::Stages);
}

#[test]
fn invalid_text_stays_in_text_mode_and_sends_nothing() {
    let (s, _) = act(with_two_stages(), A::SetMode(Mode::Text));
    let (s, _) = act(s, A::EditText("[{".into()));
    let (s, c) = act(s, A::SetMode(Mode::Stages));
    assert!(c.is_empty());
    assert_eq!(s.aggregate.mode, Mode::Text);
    assert!(matches!(
        s.aggregate.text_error,
        Some(PipelineError::Syntax { .. })
    ));
    assert_eq!(s.aggregate.stages.len(), 2);
}

#[test]
fn asking_for_the_mode_already_shown_keeps_the_draft() {
    let (s, _) = act(with_two_stages(), A::SetMode(Mode::Text));
    let (s, _) = act(s, A::EditText("typed".into()));
    let (s, c) = act(s, A::SetMode(Mode::Text));
    assert!(c.is_empty());
    assert_eq!(s.aggregate.text, "typed");

    let before = (*with_two_stages().aggregate).clone();
    let (s, c) = act(with_two_stages(), A::SetMode(Mode::Stages));
    assert!(c.is_empty());
    assert_eq!(*s.aggregate, before);
}

#[test]
fn the_answer_is_shown_and_a_stale_one_is_ignored() {
    let (s, c) = act(with_two_stages(), A::Select(0));
    let (_, older, _) = the_preview(&c);
    let (s, c) = act(s, A::Refresh);
    let (_, newer, _) = the_preview(&c);

    let (s, c) = previewed(s, EMBEDDED_SESSION, older, Ok(vec![doc! { "old": 1 }]));
    assert!(c.is_empty());
    assert!(matches!(s.aggregate.preview, PreviewState::Loading { .. }));

    let (s, _) = previewed(s, EMBEDDED_SESSION, newer, Ok(vec![doc! { "new": 1 }]));
    assert_eq!(shown_docs(&s.aggregate), Some(vec![doc! { "new": 1 }]));
}

#[test]
fn an_error_keeps_the_last_preview_and_raises_no_banner() {
    let (s, c) = act(with_two_stages(), A::Refresh);
    let (_, request, _) = the_preview(&c);
    let error = CoreError::Database("timed out".into());
    let (s, c) = previewed(s, EMBEDDED_SESSION, request, Err(error.clone()));
    assert!(c.is_empty());
    assert!(s.banner.is_none());
    let last = Some(Preview {
        docs: vec![doc! { "a": 1 }],
        not_run: Vec::new(),
    });
    assert_eq!(
        s.aggregate.preview,
        PreviewState::Failed {
            error: PreviewError::Core(error),
            last,
        }
    );
}

#[test]
fn a_preview_that_could_not_run_fails_inline_never_as_a_banner() {
    for error in [CoreError::ReadOnly, CoreError::Unreachable] {
        let (s, c) = act(with_two_stages(), A::Refresh);
        let (_, request, _) = the_preview(&c);
        let event = Event::RequestFailed {
            session: EMBEDDED_SESSION,
            context: "aggregate_preview",
            request,
            error: error.clone(),
        };
        let indexes = s.indexes.clone();
        let (s, c) = update(s, Input::Event(event));
        assert!(c.is_empty(), "{error:?}");
        assert!(s.banner.is_none(), "{error:?}");
        assert_eq!(s.status, ConnStatus::Connected, "{error:?}");
        assert_eq!(s.indexes, indexes, "{error:?}");
        assert!(
            matches!(&s.aggregate.preview, PreviewState::Failed { error: PreviewError::Core(e), last: Some(_) } if e == &error),
            "{error:?}"
        );
    }
}

#[test]
fn a_superseded_preview_is_consumed() {
    let (s, c) = act(with_two_stages(), A::Refresh);
    let (_, request, _) = the_preview(&c);
    let before = (*s.aggregate).clone();
    let event = Event::RequestFailed {
        session: EMBEDDED_SESSION,
        context: "aggregate_preview",
        request,
        error: CoreError::Superseded,
    };
    let (s, c) = update(s, Input::Event(event));
    assert!(c.is_empty());
    assert!(s.banner.is_none());
    assert_eq!(*s.aggregate, before);
}

#[test]
fn read_only_and_production_sessions_preview_without_any_confirmation() {
    for (read_only, production) in [(true, false), (false, true), (true, true)] {
        let mut s = connected();
        s.read_only = read_only;
        s.production = production;
        let (s, c) = act(s, A::AddStage("$match".into()));
        let (session, _, _) = the_preview(&c);
        assert_eq!(session, EMBEDDED_SESSION);
        assert!(c.iter().all(|c| !c.is_write()));
        assert!(s.write_warning.is_none());
        assert!(s.banner.is_none());
    }
}

#[test]
fn without_a_collection_or_a_connection_the_draft_changes_but_nothing_is_sent() {
    let mut no_collection = connected();
    no_collection.sidebar.selected = None;
    let mut disconnected = connected();
    disconnected.status = ConnStatus::Disconnected;
    let mut no_session = connected();
    no_session.active = None;
    for s in [no_collection, disconnected, no_session] {
        let (s, c) = act(s, A::AddStage("$match".into()));
        assert!(c.is_empty());
        assert_eq!(s.aggregate.stages.len(), 1);
        assert_eq!(s.aggregate.preview, PreviewState::Idle);
        assert!(s.banner.is_none());
        let (s, c) = act(s, A::Refresh);
        assert!(c.is_empty());
        assert_eq!(s.aggregate.preview, PreviewState::Idle);
    }
}

/// Session `a` active with one `$match` stage; `b` parked, connected, with a `$sort` preview of
/// request `request` in flight.
fn two_sessions() -> (AppState, Uuid, Uuid, u64) {
    let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let (draft_b, plan) = AggregateState::default().add_stage("$sort").begin_preview();
    let request = plan.expect("a $sort stage previews").0;
    let view_b = SessionView {
        status: ConnStatus::Connected,
        aggregate: Box::new(draft_b),
        ..SessionView::default()
    };
    let s = AppState {
        open: vec![a],
        active: Some(a),
        ..connected()
    };
    let s = s.opened_with(b, view_b).activated(a);
    let (s, _) = act(s, A::AddStage("$match".into()));
    (s, a, b, request)
}

#[test]
fn an_answer_for_the_inactive_session_lands_in_its_own_draft() {
    let (s, a, b, request) = two_sessions();
    let mine = (*s.aggregate).clone();
    let (s, c) = previewed(s, b, request, Ok(vec![doc! { "b": 1 }]));
    assert!(c.is_empty());
    assert_eq!(s.active, Some(a));
    assert_eq!(*s.aggregate, mine);
    assert_eq!(
        shown_docs(&s.parked[&b].aggregate),
        Some(vec![doc! { "b": 1 }])
    );

    let s = s.activated(b);
    assert_eq!(s.aggregate.stages.len(), 1);
    assert!(s.aggregate.stages[0].text.contains("$sort"));
    assert_eq!(s.parked[&a].aggregate.stages[0].text, mine.stages[0].text);
}

#[test]
fn an_answer_numbered_like_the_other_session_does_not_cross_over() {
    let (s, a, b, _) = two_sessions();
    let request_a = s.aggregate.request;
    let theirs = (*s.parked[&b].aggregate).clone();
    let (s, _) = previewed(s, a, request_a, Ok(vec![doc! { "a": 1 }]));
    assert_eq!(shown_docs(&s.aggregate), Some(vec![doc! { "a": 1 }]));
    assert_eq!(*s.parked[&b].aggregate, theirs);
}

#[test]
fn switching_to_the_aggregate_tab_with_no_collection_selected_sends_nothing() {
    let mut s = connected();
    s.sidebar.selected = None;
    let (s, c) = update(s, Input::Tab(crate::viewmodel::CollectionTab::Aggregate));
    assert!(c.is_empty());
    assert!(s.banner.is_none());
    assert_eq!(*s.aggregate, AggregateState::default());
}
