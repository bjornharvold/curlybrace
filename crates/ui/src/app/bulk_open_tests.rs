//! Opening the bulk flow, its edits and its steps.
use super::bulk_test_support::*;

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
