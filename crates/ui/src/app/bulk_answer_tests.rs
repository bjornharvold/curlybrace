//! The answers the bulk flow trusts: stale ids, other collections, failed numbered reads.
use super::bulk_test_support::*;

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
    let applied = super::bulk_actions::count_arrived(s, ns(), request, Some(1));
    assert_eq!(flow(&applied).count, CountState::Pending);
}

#[test]
fn a_count_without_an_answer_from_the_store_is_unavailable() {
    let (s, id) = plain();
    let (s, cmds) = act(s, BulkAction::Open(BulkKind::Delete));
    let (s, _) = ev(s, count_answer(id, count_id(&cmds), None));
    assert_eq!(flow(&s).count, CountState::Unavailable);
}

#[test]
fn bulk_count_timeout_request_failed_marks_count_unavailable() {
    let (s, id) = plain();
    let (s, cmds) = act(s, BulkAction::Open(BulkKind::Delete));
    let (s, _) = ev(s, read_failed(id, "bulk_count", count_id(&cmds)));
    assert_eq!(flow(&s).count, CountState::Unavailable);
    assert!(s.banner.is_none());
}

#[test]
fn a_stale_count_request_failed_is_ignored() {
    let (s, id) = plain();
    let (s, cmds) = act(s, BulkAction::Open(BulkKind::Delete));
    let (s, _) = ev(s, read_failed(id, "bulk_count", count_id(&cmds) + 5));
    assert_eq!(flow(&s).count, CountState::Pending);
}

#[test]
fn a_failed_sample_request_marks_the_preview_failed_without_a_banner() {
    let (s, id) = defining(BulkKind::Delete, config(false, false));
    let (s, cmds) = act(s, BulkAction::Next);
    let (s, out) = ev(s, read_failed(id, "bulk_sample", preview_id(&cmds)));
    assert_eq!(flow(&s).preview, PreviewState::Failed);
    assert_eq!(s.bulk.as_ref().unwrap().preview_request, None);
    assert!(s.banner.is_none() && out.is_empty());
}

#[test]
fn a_stale_sample_request_failed_is_ignored() {
    let (s, id) = defining(BulkKind::Delete, config(false, false));
    let (s, cmds) = act(s, BulkAction::Next);
    let (s, _) = ev(s, read_failed(id, "bulk_sample", preview_id(&cmds) + 5));
    assert_eq!(flow(&s).preview, PreviewState::Pending);
    assert!(s.banner.is_none());
}

#[test]
fn a_bulk_failure_never_reaches_the_index_panel_arm() {
    // The index panel numbers its own reads: a bulk id that equals its counter must not be
    // taken for one of its requests (it would raise a banner through `Failed`).
    let (s, id) = defining(BulkKind::Delete, config(false, false));
    let (mut s, cmds) = act(s, BulkAction::Next);
    let stale = preview_id(&cmds) + 5;
    s.indexes.request = stale;
    let (s, _) = ev(s, read_failed(id, "bulk_sample", stale));
    assert_eq!(flow(&s).preview, PreviewState::Pending);
    assert!(s.banner.is_none());
    assert!(!s.indexes.loading);
}

#[test]
fn a_failed_count_is_settled_by_id_when_the_index_counter_collides() {
    let (s, id) = plain();
    let (mut s, cmds) = act(s, BulkAction::Open(BulkKind::Delete));
    let request = count_id(&cmds);
    s.indexes.request = request + 9;
    let (s, _) = ev(s, read_failed(id, "bulk_count", request));
    assert_eq!(flow(&s).count, CountState::Unavailable);
    assert!(s.banner.is_none());
}
