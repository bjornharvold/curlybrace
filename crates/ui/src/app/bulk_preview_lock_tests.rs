//! A read-only lock that lands after the dialog opened must not strand the preview.
use super::bulk_test_support::*;

fn locked(mut s: AppState, id: Uuid) -> AppState {
    s.conn.connections.iter_mut().for_each(|c| {
        if c.id == id {
            c.read_only = true;
        }
    });
    s
}

#[test]
fn next_in_define_is_refused_when_the_connection_became_read_only() {
    for kind in [BulkKind::Delete, BulkKind::Update] {
        let (s, id) = defining(kind, config(false, false));
        let (s, cmds) = act(locked(s, id), BulkAction::Next);
        assert!(cmds.is_empty(), "{kind:?}: no preview may be sent");
        assert_eq!(s.banner, Some(Banner::ReadOnly));
        assert!(s.bulk.is_none(), "{kind:?}: the flow is over");
    }
}

#[test]
fn next_leaves_an_unrelated_pending_write_alone_when_refused() {
    let (mut s, id) = defining(BulkKind::Delete, config(false, false));
    s.pending = Some(Pending {
        ns: ns(),
        editor: None,
    });
    let (s, _) = act(locked(s, id), BulkAction::Next);
    assert!(s.pending.is_some());
}

#[test]
fn a_read_only_refusal_of_the_preview_closes_the_flow_with_the_banner() {
    let (s, id) = defining(BulkKind::Update, config(false, false));
    let (s, _) = act(s, BulkAction::Next);
    assert_eq!(flow(&s).step, BulkStep::Preview);
    let (s, cmds) = ev(s, failed(id, "bulk_preview", CoreError::ReadOnly));
    assert!(cmds.is_empty());
    assert!(s.bulk.is_none());
    assert_eq!(s.banner, Some(Banner::ReadOnly));
}

#[test]
fn a_late_read_only_refusal_of_a_preview_nobody_waits_for_changes_nothing() {
    let (s, id) = defining(BulkKind::Update, config(false, false));
    let before = s.bulk.clone();
    let (after, _) = ev(s, failed(id, "bulk_preview", CoreError::ReadOnly));
    assert_eq!(after.bulk, before);
    assert!(after.banner.is_none());
}

#[test]
fn a_read_only_refusal_of_a_write_still_takes_the_write_failure_path() {
    for context in ["bulk_update", "bulk_delete"] {
        let (s, id, _) = running_delete();
        let (s, _) = ev(s, failed(id, context, CoreError::ReadOnly));
        assert!(s.bulk.is_none() && s.pending.is_none(), "{context}");
        assert_eq!(s.banner, Some(Banner::ReadOnly));
        // With nothing in flight it is a late answer and changes nothing.
        let (c, id) = confirming(BulkKind::Delete, config(false, false));
        let before = c.bulk.clone();
        let (c, _) = ev(c, failed(id, context, CoreError::ReadOnly));
        assert_eq!(c.bulk, before, "{context}");
    }
}

#[test]
fn another_preview_failure_still_marks_the_preview_failed() {
    let (s, id) = defining(BulkKind::Update, config(false, false));
    let (s, _) = act(s, BulkAction::Next);
    let error = CoreError::Database("x".into());
    let (s, _) = ev(s, failed(id, "bulk_preview", error.clone()));
    assert_eq!(flow(&s).preview, PreviewState::Failed(error));
    assert!(s.bulk.is_some() && s.banner.is_none());
}
