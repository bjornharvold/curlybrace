//! Save, open, rename and delete of saved pipelines through the reducer, with a real `AppState`
//! and a `pipelines.json` in a tempdir.
use super::AggregateAction as A;
use crate::app::reducer::{Input, update};
use crate::app::state::{AppState, ConnStatus};
use crate::viewmodel::aggregate_saved::SavedDialog;
use curlybrace_core::runtime::Command;
use curlybrace_core::saved_pipeline::SavedPipelines;
use curlybrace_core::store::Namespace;
use std::os::unix::fs::PermissionsExt;
use uuid::Uuid;

pub(super) fn ns() -> Namespace {
    Namespace::new("app", "people")
}

/// A connected session on connection `conn`, `app.people` selected, saving into `dir`.
pub(super) fn session_in(dir: &tempfile::TempDir, conn: Uuid) -> AppState {
    let mut s = AppState::new(vec![]).with_pipelines_path(Ok(dir.path().join("pipelines.json")));
    s.open = vec![conn];
    s.active = Some(conn);
    s.status = ConnStatus::Connected;
    s.sidebar = s
        .sidebar
        .with_databases(vec!["app".into()])
        .with_collections("app", vec!["people".into()])
        .toggled("app")
        .selecting(ns());
    s
}

pub(super) fn act(s: AppState, action: A) -> (AppState, Vec<Command>) {
    update(s, Input::Aggregate(action))
}

pub(super) fn run(s: AppState, actions: Vec<A>) -> AppState {
    actions.into_iter().fold(s, |s, a| act(s, a).0)
}

pub(super) fn saved_as(s: AppState, name: &str) -> AppState {
    run(
        s,
        vec![
            A::SaveClicked,
            A::SaveNameEdited(name.into()),
            A::SaveConfirmed,
        ],
    )
}

pub(super) fn drafted(s: AppState) -> AppState {
    run(
        s,
        vec![A::AddStage("$match".into()), A::AddStage("$sort".into())],
    )
}

pub(super) fn only_entry(
    s: &AppState,
    conn: Uuid,
) -> curlybrace_core::saved_pipeline::SavedPipeline {
    s.pipelines.list(conn)[0].clone()
}

#[test]
fn save_opens_a_dialog_and_cancel_closes_it_without_touching_the_draft() {
    let dir = tempfile::tempdir().unwrap();
    let s = drafted(session_in(&dir, Uuid::new_v4()));
    let before = s.aggregate.stages.clone();
    let (s, commands) = act(s, A::SaveClicked);
    assert!(commands.is_empty());
    assert!(matches!(
        s.aggregate.saved.dialog,
        Some(SavedDialog::Save {
            replace_prompt: false,
            ..
        })
    ));
    let s = act(s, A::DialogCancelled).0;
    assert_eq!(s.aggregate.saved.dialog, None);
    assert_eq!(s.aggregate.stages, before);
}

#[test]
fn saving_stores_the_draft_on_disk_and_marks_it_opened_and_unmodified() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = saved_as(drafted(session_in(&dir, conn)), "  Totals ");
    assert_eq!(s.aggregate.saved.dialog, None);
    let entry = only_entry(&s, conn);
    assert_eq!(entry.name, "Totals");
    assert_eq!(entry.ns_hint.as_deref(), Some("app.people"));
    assert_eq!(entry.stages.len(), 2);
    assert_eq!(s.aggregate.saved.opened, Some((entry.id, "Totals".into())));
    assert!(!s.aggregate.is_modified());
    assert!(dir.path().join("pipelines.json").exists());
}

#[test]
fn a_saved_pipeline_opens_on_a_fresh_state_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = saved_as(drafted(session_in(&dir, conn)), "totals");
    let id = only_entry(&s, conn).id;
    let fresh = session_in(&dir, conn);
    assert_eq!(fresh.pipelines.list(conn).len(), 1);
    let (fresh, commands) = act(fresh, A::OpenMenuPicked(id));
    assert!(commands.is_empty(), "opening runs nothing by itself");
    assert_eq!(fresh.aggregate.stages.len(), 2);
    assert_eq!(fresh.aggregate.saved.dialog, None);
    assert!(!fresh.aggregate.is_modified());
}

#[test]
fn name_problems_show_inline_and_keep_the_dialog_open() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let cases = [
        ("   ", "aggregate.saved.error.empty_name"),
        (&"x".repeat(81), "aggregate.saved.error.name_too_long"),
        ("two\nlines", "aggregate.saved.error.invalid_name"),
    ];
    for (name, key) in cases {
        let s = saved_as(drafted(session_in(&dir, conn)), name);
        assert_eq!(s.aggregate.saved.error, Some(key));
        assert!(s.aggregate.saved.dialog.is_some());
        assert!(s.banner.is_none());
        assert!(s.pipelines.list(conn).is_empty());
    }
}

#[test]
fn a_taken_name_asks_to_replace_and_replacing_keeps_the_id() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = saved_as(drafted(session_in(&dir, conn)), "Totals");
    let id = only_entry(&s, conn).id;
    let s = run(s, vec![A::AddStage("$limit".into()), A::SaveClicked]);
    let s = run(
        s,
        vec![A::SaveNameEdited("totals".into()), A::SaveConfirmed],
    );
    assert!(matches!(
        s.aggregate.saved.dialog,
        Some(SavedDialog::Save {
            replace_prompt: true,
            ..
        })
    ));
    assert_eq!(
        s.pipelines.list(conn)[0].stages.len(),
        2,
        "not replaced yet"
    );
    let s = act(s, A::SaveConfirmed).0;
    assert_eq!(s.aggregate.saved.dialog, None);
    assert_eq!(s.pipelines.list(conn).len(), 1);
    let entry = only_entry(&s, conn);
    assert_eq!(entry.id, id);
    assert_eq!(entry.stages.len(), 3);
    assert_eq!(entry.name, "Totals", "the stored spelling stays");
    assert_eq!(s.aggregate.saved.opened, Some((id, "Totals".into())));
}

#[test]
fn editing_the_name_withdraws_the_replace_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = saved_as(drafted(session_in(&dir, conn)), "a");
    let s = run(s, vec![A::SaveClicked, A::SaveConfirmed]);
    assert!(matches!(
        s.aggregate.saved.dialog,
        Some(SavedDialog::Save {
            replace_prompt: true,
            ..
        })
    ));
    let s = act(s, A::SaveNameEdited("b".into())).0;
    assert!(matches!(
        s.aggregate.saved.dialog,
        Some(SavedDialog::Save {
            replace_prompt: false,
            ..
        })
    ));
}

#[test]
fn a_failed_write_keeps_the_dialog_the_draft_and_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("readonly");
    std::fs::create_dir(&folder).unwrap();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::write(folder.join("probe"), "x").is_ok() {
        return; // permissions not enforced (root)
    }
    let conn = Uuid::new_v4();
    let s = drafted(session_in(&dir, conn)).with_pipelines_path(Ok(folder.join("pipelines.json")));
    let stages = s.aggregate.stages.clone();
    let s = saved_as(s, "totals");
    assert!(matches!(
        s.aggregate.saved.dialog,
        Some(SavedDialog::Save { .. })
    ));
    assert_eq!(
        s.aggregate.saved.error,
        Some("aggregate.saved.write_failed_save")
    );
    assert_eq!(s.aggregate.stages, stages);
    assert!(s.pipelines.list(conn).is_empty());
    assert_eq!(s.aggregate.saved.opened, None);
    assert!(s.banner.is_none(), "the failure is inline, not a banner");
}

#[test]
fn a_blocked_store_refuses_inline() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let mut s = drafted(session_in(&dir, conn));
    s.pipelines_guard = crate::app::persistence::PersistGuard::blocked();
    let s = saved_as(s, "totals");
    assert_eq!(s.aggregate.saved.error, Some("aggregate.saved.blocked"));
    assert!(s.pipelines.list(conn).is_empty());
}

#[test]
fn the_limits_map_to_their_own_messages() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = drafted(session_in(&dir, conn));
    let big = s.aggregate.clone().edit(0, &"x".repeat(1_000_001));
    let s = AppState {
        aggregate: Box::new(big),
        ..s
    };
    let s = saved_as(s, "big");
    assert_eq!(
        s.aggregate.saved.error,
        Some("aggregate.saved.error.too_large")
    );

    let dir = tempfile::tempdir().unwrap();
    let mut s = drafted(session_in(&dir, conn));
    s.pipelines = (0..200).fold(SavedPipelines::default(), |p, i| {
        p.saved(conn, &format!("p{i}"), None, Vec::new(), 1, false)
            .unwrap()
    });
    let s = saved_as(s, "one more");
    assert_eq!(
        s.aggregate.saved.error,
        Some("aggregate.saved.error.too_many")
    );
}

#[test]
fn a_file_changed_by_another_window_refuses_inline_and_reloads() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let first = drafted(session_in(&dir, conn));
    let second = drafted(session_in(&dir, conn));
    let first = saved_as(first, "other");
    assert_eq!(first.aggregate.saved.error, None);
    let draft = second.aggregate.stages.clone();
    let s = saved_as(second, "totals");
    assert_eq!(
        s.aggregate.saved.error,
        Some("aggregate.saved.error.changed_on_disk")
    );
    assert!(matches!(
        s.aggregate.saved.dialog,
        Some(SavedDialog::Save { .. })
    ));
    assert_eq!(s.aggregate.stages, draft);
    assert_eq!(s.pipelines.list(conn).len(), 1);
    assert_eq!(s.pipelines.list(conn)[0].name, "other");
    assert!(s.banner.is_none());
    let s = act(s, A::SaveConfirmed).0;
    assert_eq!(s.aggregate.saved.error, None);
    assert_eq!(s.pipelines.list(conn).len(), 2);
}

#[test]
fn a_file_of_a_newer_version_refuses_every_save_and_stays_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let newer = r#"{"version": 2, "connections": {}}"#;
    std::fs::write(dir.path().join("pipelines.json"), newer).unwrap();
    let s = saved_as(drafted(session_in(&dir, conn)), "totals");
    assert_eq!(
        s.aggregate.saved.error,
        Some("aggregate.saved.error.unsupported")
    );
    assert!(s.pipelines.list(conn).is_empty());
    let text = std::fs::read_to_string(dir.path().join("pipelines.json")).unwrap();
    assert_eq!(text, newer);
}

#[test]
fn text_that_does_not_parse_cannot_be_saved() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = run(
        drafted(session_in(&dir, conn)),
        vec![
            A::SetMode(crate::viewmodel::aggregate::Mode::Text),
            A::EditText("[{".into()),
        ],
    );
    let s = saved_as(s, "broken");
    assert_eq!(
        s.aggregate.saved.error,
        Some("aggregate.saved.error.invalid_text")
    );
    assert!(s.pipelines.list(conn).is_empty());
}

fn selecting(mut s: AppState, collection: &str) -> AppState {
    s.sidebar = s
        .sidebar
        .with_collections("app", vec!["people".into(), collection.into()])
        .selecting(Namespace::new("app", collection));
    s
}

fn hint_of(s: &AppState, conn: Uuid, name: &str) -> Option<String> {
    s.pipelines.find_by_name(conn, name)?.ns_hint.clone()
}

#[test]
fn a_new_pipeline_is_hinted_with_the_selected_collection() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = saved_as(drafted(session_in(&dir, conn)), "first");
    let s = saved_as(selecting(s, "orders"), "second");
    assert_eq!(hint_of(&s, conn, "first").as_deref(), Some("app.people"));
    assert_eq!(hint_of(&s, conn, "second").as_deref(), Some("app.orders"));
}

#[test]
fn re_saving_a_pipeline_keeps_its_own_collection_hint() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let s = saved_as(drafted(session_in(&dir, conn)), "Totals");
    // The sidebar moved on to another collection; the pipeline was saved from app.people.
    let s = selecting(s, "orders");
    let s = run(
        s,
        vec![
            A::AddStage("$limit".into()),
            A::SaveClicked,
            A::SaveConfirmed,
            A::SaveConfirmed,
        ],
    );
    assert_eq!(s.pipelines.list(conn).len(), 1);
    assert_eq!(s.pipelines.list(conn)[0].stages.len(), 3, "it was replaced");
    assert_eq!(hint_of(&s, conn, "Totals").as_deref(), Some("app.people"));
}
