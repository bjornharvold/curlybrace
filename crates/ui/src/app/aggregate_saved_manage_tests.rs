//! Open, rename and delete of saved pipelines through the reducer (see `tests` for the helpers).
use super::AggregateAction as A;
use super::tests::{act, drafted, only_entry, run, saved_as, session_in};
use crate::viewmodel::aggregate_saved::SavedDialog;
use uuid::Uuid;

#[test]
fn open_asks_first_when_the_draft_is_modified_and_cancel_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = saved_as(drafted(session_in(&dir, conn)), "totals");
    let id = only_entry(&s, conn).id;
    let s = act(s, A::AddStage("$limit".into())).0;
    let draft = s.aggregate.stages.clone();
    let s = act(s, A::OpenMenuPicked(id)).0;
    assert_eq!(
        s.aggregate.saved.dialog,
        Some(SavedDialog::ConfirmOpen { id })
    );
    assert_eq!(s.aggregate.stages, draft);
    let s = act(s, A::DialogCancelled).0;
    assert_eq!(s.aggregate.stages, draft);
    assert_eq!(s.aggregate.saved.dialog, None);
    let s = run(s, vec![A::OpenMenuPicked(id), A::OpenConfirmed]);
    assert_eq!(s.aggregate.stages.len(), 2);
    assert!(!s.aggregate.is_modified());
    assert_eq!(s.aggregate.saved.dialog, None);
}

#[test]
fn opening_an_unmodified_draft_or_an_empty_one_does_not_ask() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = saved_as(drafted(session_in(&dir, conn)), "totals");
    let id = only_entry(&s, conn).id;
    let s = act(s, A::OpenMenuPicked(id)).0;
    assert_eq!(s.aggregate.saved.dialog, None);
    let empty = act(session_in(&dir, conn), A::OpenMenuPicked(id)).0;
    assert_eq!(empty.aggregate.stages.len(), 2);
}

#[test]
fn opening_clears_a_waiting_edit_and_leaves_the_preview_idle() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = saved_as(drafted(session_in(&dir, conn)), "totals");
    let id = only_entry(&s, conn).id;
    let (s, _) = act(s, A::OpenMenuPicked(id));
    assert_eq!(
        s.aggregate.preview,
        crate::viewmodel::aggregate::PreviewState::Idle
    );
    assert!(s.aggregate_view.pending.is_none());
}

#[test]
fn rename_validates_persists_and_updates_the_opened_name() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = saved_as(drafted(session_in(&dir, conn)), "a");
    let id = only_entry(&s, conn).id;
    let s = saved_as(run(s, vec![A::AddStage("$limit".into())]), "b");
    let a = s.pipelines.find_by_name(conn, "a").unwrap().id;
    let s = run(
        s,
        vec![
            A::RenameStarted(a),
            A::RenameEdited("B".into()),
            A::RenameConfirmed,
        ],
    );
    assert_eq!(
        s.aggregate.saved.error,
        Some("aggregate.saved.error.name_taken")
    );
    assert!(matches!(
        s.aggregate.saved.dialog,
        Some(SavedDialog::Rename { .. })
    ));
    let s = run(s, vec![A::RenameEdited("c".into()), A::RenameConfirmed]);
    assert_eq!(s.aggregate.saved.dialog, None);
    assert_eq!(s.pipelines.find(conn, a).unwrap().name, "c");
    let s = run(
        s,
        vec![
            A::RenameStarted(id),
            A::RenameEdited("renamed".into()),
            A::RenameConfirmed,
        ],
    );
    assert_eq!(s.pipelines.find(conn, id).unwrap().name, "renamed");
    // "b" is the opened one and was not renamed
    assert_eq!(s.aggregate.saved.opened.as_ref().unwrap().1, "b");
    let b = s.aggregate.saved.opened.as_ref().unwrap().0;
    let s = run(
        s,
        vec![
            A::RenameStarted(b),
            A::RenameEdited("b2".into()),
            A::RenameConfirmed,
        ],
    );
    assert_eq!(s.aggregate.saved.opened.as_ref().unwrap().1, "b2");
    assert!(!s.aggregate.is_modified());
}

#[test]
fn delete_asks_and_deleting_the_opened_pipeline_keeps_the_draft() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = saved_as(drafted(session_in(&dir, conn)), "totals");
    let id = only_entry(&s, conn).id;
    let s = act(s, A::DeleteAsked(id)).0;
    assert_eq!(
        s.aggregate.saved.dialog,
        Some(SavedDialog::ConfirmDelete { id })
    );
    assert_eq!(s.pipelines.list(conn).len(), 1, "asking deletes nothing");
    let s = act(s, A::DialogCancelled).0;
    assert_eq!(s.pipelines.list(conn).len(), 1);
    let s = run(s, vec![A::DeleteAsked(id), A::DeleteConfirmed]);
    assert!(s.pipelines.list(conn).is_empty());
    assert_eq!(s.aggregate.saved.opened, None);
    assert_eq!(s.aggregate.stages.len(), 2);
    assert_eq!(s.aggregate.saved.dialog, None);
    let again = session_in(&dir, conn);
    assert!(again.pipelines.list(conn).is_empty(), "the file follows");
}

/// A session holding one saved pipeline, in a folder that can no longer be written to: the
/// next write fails. Gives the state, the pipeline and the folder (restored afterwards by the
/// caller through `restore`); `None` where permissions are not enforced, as for root.
fn unwritable(dir: &tempfile::TempDir, conn: Uuid) -> Option<(crate::app::state::AppState, Uuid)> {
    use std::os::unix::fs::PermissionsExt;
    let s = saved_as(drafted(session_in(dir, conn)), "totals");
    let id = only_entry(&s, conn).id;
    let mode = |m| std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(m));
    mode(0o555).unwrap();
    if std::fs::write(dir.path().join("probe"), "x").is_ok() {
        mode(0o755).unwrap();
        return None;
    }
    Some((s, id))
}

fn restore(dir: &tempfile::TempDir) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn a_failed_save_says_the_pipeline_is_still_open() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let Some((s, _)) = unwritable(&dir, conn) else {
        return;
    };
    let s = run(s, vec![A::SaveClicked, A::SaveNameEdited("more".into())]);
    let s = act(s, A::SaveConfirmed).0;
    restore(&dir);
    assert_eq!(
        s.aggregate.saved.error,
        Some("aggregate.saved.write_failed_save")
    );
}

#[test]
fn a_failed_rename_says_the_pipeline_was_not_renamed() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let Some((s, id)) = unwritable(&dir, conn) else {
        return;
    };
    let s = run(
        s,
        vec![
            A::RenameStarted(id),
            A::RenameEdited("renamed".into()),
            A::RenameConfirmed,
        ],
    );
    restore(&dir);
    assert_eq!(
        s.aggregate.saved.error,
        Some("aggregate.saved.write_failed_rename")
    );
    assert_eq!(s.pipelines.list(conn)[0].name, "totals");
}

#[test]
fn a_failed_delete_says_the_pipeline_was_not_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let Some((s, id)) = unwritable(&dir, conn) else {
        return;
    };
    let s = run(s, vec![A::DeleteAsked(id), A::DeleteConfirmed]);
    restore(&dir);
    assert_eq!(
        s.aggregate.saved.error,
        Some("aggregate.saved.write_failed_delete")
    );
    assert_eq!(s.pipelines.list(conn).len(), 1);
    assert!(matches!(
        s.aggregate.saved.dialog,
        Some(SavedDialog::ConfirmDelete { .. })
    ));
}

#[test]
fn deleting_another_pipeline_keeps_the_opened_mark() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = saved_as(drafted(session_in(&dir, conn)), "a");
    let a = only_entry(&s, conn).id;
    let s = saved_as(s, "b");
    let s = run(s, vec![A::DeleteAsked(a), A::DeleteConfirmed]);
    assert_eq!(s.pipelines.list(conn).len(), 1);
    assert!(s.aggregate.saved.opened.is_some());
}

#[test]
fn two_sessions_on_two_connections_never_see_each_others_pipelines() {
    let dir = tempfile::tempdir().unwrap();
    let (one, two) = (Uuid::new_v4(), Uuid::new_v4());
    let mut s = drafted(session_in(&dir, one));
    s.open = vec![one, two];
    let s = saved_as(s, "mine");
    assert_eq!(s.pipelines.list(one).len(), 1);
    let id = only_entry(&s, one).id;
    // the same actions aimed at the other session find nothing
    let (s, _) = s.with_session(two, |s| act(s, A::OpenMenuPicked(id)));
    assert!(s.pipelines.list(two).is_empty());
    assert_eq!(s.active, Some(one));
    let parked = s.parked.get(&two).map(|v| v.aggregate.stages.len());
    assert_eq!(parked.unwrap_or(0), 0, "the other session opened nothing");
}

#[test]
fn the_dialog_and_the_opened_mark_survive_a_lost_connection() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = saved_as(drafted(session_in(&dir, conn)), "totals");
    let s = act(s, A::SaveClicked).0;
    let s = s.failed(curlybrace_core::error::CoreError::Unreachable);
    assert!(matches!(
        s.aggregate.saved.dialog,
        Some(SavedDialog::Save { .. })
    ));
    assert!(s.aggregate.saved.opened.is_some());
}

#[test]
fn without_a_session_nothing_happens() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session_in(&dir, Uuid::new_v4());
    s.active = None;
    let (s, commands) = act(s, A::SaveClicked);
    assert!(commands.is_empty());
    assert_eq!(s.aggregate.saved.dialog, None);
}
