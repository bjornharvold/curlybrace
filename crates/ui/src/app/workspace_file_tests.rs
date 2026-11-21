//! The workspace file: what it holds, how it loads and how it saves.
use super::*;
use crate::app::AggregateAction;
use crate::app::persistence::back_up;
use crate::app::reducer::{Input, update};
use crate::app::state::AppState;
use crate::app::tab_actions::MAX_TABS;
use crate::app::tabs::test_support::{act, ev, orders, results, two_tabs, users};
use crate::viewmodel::DocAction;
use bson::doc;
use curlybrace_core::store::Namespace;
use std::cell::RefCell;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

fn tab(connection: Uuid, collection: &str) -> SavedTab {
    SavedTab {
        connection,
        db: "app".into(),
        collection: collection.into(),
        sub_tab: SavedSubTab::Documents,
        view_mode: SavedViewMode::Table,
        find: Some(SavedFind {
            filter: r#"{"a": 1}"#.into(),
            ..SavedFind::default()
        }),
        draft: None,
        text_dropped: false,
        options_dropped: false,
    }
}

fn file_of(tabs: Vec<SavedTab>) -> WorkspaceFile {
    WorkspaceFile {
        tabs,
        ..WorkspaceFile::default()
    }
}

fn known(ids: &[Uuid]) -> BTreeSet<Uuid> {
    ids.iter().copied().collect()
}

fn write(dir: &tempfile::TempDir, text: &[u8]) -> PathBuf {
    let path = dir.path().join("workspace.json");
    std::fs::write(&path, text).unwrap();
    path
}

/// A backup that records the tags it was asked for.
fn recording(tags: &RefCell<Vec<String>>) -> impl Fn(&Path, &str) -> std::io::Result<PathBuf> + '_ {
    move |path, tag| {
        tags.borrow_mut().push(tag.to_string());
        Ok(path.to_path_buf())
    }
}

fn failing(_: &Path, _: &str) -> std::io::Result<PathBuf> {
    Err(std::io::ErrorKind::PermissionDenied.into())
}

#[test]
fn round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let path = dir.path().join("workspace.json");
    let file = WorkspaceFile {
        active: Some(1),
        split: Some(SavedSplit {
            secondary: 0,
            ratio: 0.4,
            orientation: SavedOrientation::Stacked,
            secondary_first: true,
        }),
        ..file_of(vec![tab(id, "users"), tab(id, "orders")])
    };
    save(&path, &PersistGuard::default(), &file).unwrap();
    let loaded = load(&path, &back_up, &known(&[id]));
    assert_eq!(loaded.file, file);
    assert_eq!(loaded.recovery, None);
    assert!(!loaded.needs_rewrite);
}

#[test]
fn missing_file_restores_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.json");
    let loaded = load(&path, &back_up, &BTreeSet::new());
    assert_eq!(loaded.file, WorkspaceFile::default());
    assert_eq!(loaded.recovery, None);
    assert_eq!(loaded.guard, PersistGuard::default());
    assert!(!path.exists(), "loading never creates the file");
}

#[test]
fn corrupt_file_is_backed_up_with_a_notice() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(&dir, b"this is not json");
    let loaded = load(&path, &back_up, &BTreeSet::new());
    assert_eq!(
        loaded.recovery,
        Some(Recovery::Workspace { backed_up: true })
    );
    assert!(loaded.file.tabs.is_empty());
    assert_eq!(loaded.guard, PersistGuard::default());
    let backups: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains("corrupt"))
        .collect();
    assert_eq!(backups.len(), 1, "the original is kept");
}

#[test]
fn a_file_that_is_not_an_object_or_lacks_a_version_is_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    for text in [
        &b"[]"[..],
        br#"{"restore":true,"tabs":[]}"#,
        br#"{"version":0}"#,
    ] {
        let path = write(&dir, text);
        let tags = RefCell::new(Vec::new());
        let loaded = load(&path, &recording(&tags), &BTreeSet::new());
        assert_eq!(*tags.borrow(), ["corrupt"], "{text:?}");
        assert_eq!(
            loaded.recovery,
            Some(Recovery::Workspace { backed_up: true })
        );
    }
}

#[test]
fn unreadable_file_is_backed_up_with_a_notice() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(&dir, b"{}");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    let tags = RefCell::new(Vec::new());
    let loaded = load(&path, &recording(&tags), &BTreeSet::new());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(*tags.borrow(), ["unreadable"]);
    assert_eq!(
        loaded.recovery,
        Some(Recovery::Workspace { backed_up: true })
    );
}

#[test]
fn failed_backup_blocks_saving() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(&dir, b"junk");
    let loaded = load(&path, &failing, &BTreeSet::new());
    assert_eq!(
        loaded.recovery,
        Some(Recovery::Workspace { backed_up: false })
    );
    assert_eq!(
        save(&path, &loaded.guard, &WorkspaceFile::default()),
        Err(PersistError::Blocked)
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"junk", "left as it was");
}

#[test]
fn newer_version_is_left_alone_and_saving_is_off() {
    let dir = tempfile::tempdir().unwrap();
    let text = br#"{"version":2,"restore":true,"tabs":[],"future":1}"#;
    let path = write(&dir, text);
    let tags = RefCell::new(Vec::new());
    let loaded = load(&path, &recording(&tags), &BTreeSet::new());
    assert_eq!(loaded.recovery, Some(Recovery::WorkspaceNewer));
    assert!(tags.borrow().is_empty(), "no backup of a file left alone");
    assert_eq!(
        save(&path, &loaded.guard, &WorkspaceFile::default()),
        Err(PersistError::Blocked)
    );
    assert_eq!(std::fs::read(&path).unwrap(), text);
}

#[test]
fn unknown_connection_and_bad_namespace_entries_are_skipped_and_counted() {
    let dir = tempfile::tempdir().unwrap();
    let (id, gone) = (Uuid::new_v4(), Uuid::new_v4());
    let bad = SavedTab {
        collection: "has$dollar".into(),
        ..tab(id, "x")
    };
    let file = WorkspaceFile {
        active: Some(3),
        ..file_of(vec![
            tab(gone, "users"),
            bad,
            tab(id, "orders"),
            tab(id, "users"),
        ])
    };
    let path = dir.path().join("workspace.json");
    save(&path, &PersistGuard::default(), &file).unwrap();
    let tags = RefCell::new(Vec::new());
    let loaded = load(&path, &recording(&tags), &known(&[id]));
    let names: Vec<&str> = loaded
        .file
        .tabs
        .iter()
        .map(|t| t.collection.as_str())
        .collect();
    assert_eq!(names, ["orders", "users"]);
    assert_eq!(
        loaded.file.active,
        Some(1),
        "the active tab follows its entry"
    );
    assert_eq!(
        loaded.recovery,
        Some(Recovery::WorkspaceSkipped {
            count: 2,
            backed_up: true
        })
    );
    assert_eq!(*tags.borrow(), ["skipped"]);
    assert!(loaded.needs_rewrite);
}

#[test]
fn a_skip_with_a_failed_backup_blocks_saving() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.json");
    save(
        &path,
        &PersistGuard::default(),
        &file_of(vec![tab(Uuid::new_v4(), "users")]),
    )
    .unwrap();
    let loaded = load(&path, &failing, &BTreeSet::new());
    assert_eq!(
        loaded.recovery,
        Some(Recovery::WorkspaceSkipped {
            count: 1,
            backed_up: false
        })
    );
    assert_eq!(loaded.guard, PersistGuard::blocked());
}

#[test]
fn over_8_mib_is_refused_as_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    let padding = " ".repeat(MAX_FILE_BYTES);
    let text = format!(r#"{{"version":1,"restore":true,"tabs":[]}}{padding}"#);
    let path = write(&dir, text.as_bytes());
    let tags = RefCell::new(Vec::new());
    let loaded = load(&path, &recording(&tags), &BTreeSet::new());
    assert_eq!(*tags.borrow(), ["corrupt"]);
    assert_eq!(
        loaded.recovery,
        Some(Recovery::Workspace { backed_up: true })
    );
}

#[test]
fn the_file_may_hold_more_tabs_than_the_open_limit() {
    // Saved tabs that wait (for a connection or for room) are not cut to what can be open.
    let dir = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let tabs: Vec<SavedTab> = (0..MAX_TABS + 5)
        .map(|n| tab(id, &format!("c{n}")))
        .collect();
    let path = dir.path().join("workspace.json");
    save(&path, &PersistGuard::default(), &file_of(tabs)).unwrap();
    let loaded = load(&path, &back_up, &known(&[id]));
    assert_eq!(loaded.file.tabs.len(), MAX_TABS + 5);
    assert_eq!(loaded.recovery, None);
}

#[test]
fn entries_past_the_most_the_file_may_hold_are_counted_with_a_backup() {
    let dir = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let tabs: Vec<SavedTab> = (0..MAX_SAVED_TABS + 5)
        .map(|n| tab(id, &format!("c{n}")))
        .collect();
    let path = dir.path().join("workspace.json");
    std::fs::write(&path, serde_json::to_vec(&file_of(tabs)).unwrap()).unwrap();
    let tags = RefCell::new(Vec::new());
    let loaded = load(&path, &recording(&tags), &known(&[id]));
    assert_eq!(loaded.file.tabs.len(), MAX_SAVED_TABS);
    assert_eq!(
        loaded.recovery,
        Some(Recovery::WorkspaceOverflow {
            count: 5,
            backed_up: true
        })
    );
    assert_eq!(*tags.borrow(), ["skipped"]);
}

#[test]
fn invalid_entries_do_not_eat_into_the_bound_and_both_notices_are_given() {
    let dir = tempfile::tempdir().unwrap();
    let (id, gone) = (Uuid::new_v4(), Uuid::new_v4());
    let mut tabs: Vec<SavedTab> = (0..3).map(|_| tab(gone, "x")).collect();
    tabs.extend((0..MAX_SAVED_TABS + 2).map(|n| tab(id, &format!("c{n}"))));
    let path = dir.path().join("workspace.json");
    std::fs::write(&path, serde_json::to_vec(&file_of(tabs)).unwrap()).unwrap();
    let loaded = load(&path, &back_up, &known(&[id]));
    assert_eq!(
        loaded.file.tabs.len(),
        MAX_SAVED_TABS,
        "all the valid ones that fit"
    );
    assert_eq!(
        loaded.recovery,
        Some(Recovery::WorkspaceSkipped {
            count: 3,
            backed_up: true
        })
    );
    assert_eq!(
        loaded.also,
        vec![Recovery::WorkspaceOverflow {
            count: 2,
            backed_up: true
        }]
    );
}

#[test]
fn one_tabs_bad_options_cost_that_tab_its_options_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let id = "00000000-0000-0000-0000-000000000001";
    let text = format!(
        r#"{{"version":1,"restore":true,"tabs":[
        {{"connection":"{id}","db":"a","collection":"bad","sub_tab":"documents","view_mode":"table",
          "find":{{"filter":"{{\"keep\": 1}}","sort":"","projection":"","skip":"","limit":"",
                   "options":{{"maxTimeMS":0}}}}}},
        {{"connection":"{id}","db":"a","collection":"good","sub_tab":"documents","view_mode":"table",
          "find":{{"filter":"{{}}","sort":"","projection":"","skip":"","limit":"",
                   "options":{{"comment":"fine"}}}}}}]}}"#
    );
    let path = write(&dir, text.as_bytes());
    let id: Uuid = id.parse().unwrap();
    let tags = RefCell::new(Vec::new());
    let loaded = load(&path, &recording(&tags), &known(&[id]));
    assert_eq!(
        loaded.recovery,
        Some(Recovery::WorkspaceRepaired {
            count: 1,
            backed_up: true
        }),
        "the person is told, and the original was kept first"
    );
    assert_eq!(*tags.borrow(), ["repaired"]);
    assert!(loaded.file.tabs[0].options_dropped, "that tab is flagged");
    assert!(!loaded.file.tabs[1].options_dropped);
    assert_eq!(loaded.file.tabs.len(), 2);
    let bad = loaded.file.tabs[0].find.as_ref().unwrap();
    assert_eq!(bad.filter, r#"{"keep": 1}"#, "the tab and its text stay");
    assert!(bad.options.is_empty(), "its options do not");
    let good = loaded.file.tabs[1].find.as_ref().unwrap();
    assert_eq!(good.options.comment.as_deref(), Some("fine"));
    assert!(loaded.needs_rewrite, "the first write cleans the file");
}

#[test]
fn a_split_of_a_tab_with_itself_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let file = WorkspaceFile {
        active: Some(0),
        split: Some(SavedSplit {
            secondary: 0,
            ratio: 0.5,
            orientation: SavedOrientation::Stacked,
            secondary_first: false,
        }),
        ..file_of(vec![tab(id, "a"), tab(id, "b")])
    };
    let path = dir.path().join("workspace.json");
    save(&path, &PersistGuard::default(), &file).unwrap();
    let loaded = load(&path, &back_up, &known(&[id]));
    assert_eq!(loaded.file.split, None);
    let waiting = crate::app::workspace_restore::RestoreState::of_file(
        &loaded.file,
        crate::app::workspace_app::WorkspaceSync::default(),
    );
    assert_eq!(waiting.split_hint, None);
    let again = WorkspaceFile {
        split: file.split,
        ..loaded.file
    };
    let waiting = crate::app::workspace_restore::RestoreState::of_file(
        &again,
        crate::app::workspace_app::WorkspaceSync::default(),
    );
    assert_eq!(
        waiting.split_hint, None,
        "also for a file that was not loaded"
    );
}

#[test]
fn a_draft_with_too_many_stages_is_saved_without_text_and_flagged() {
    let many = SavedDraftText {
        stages: (0..=MAX_STAGES_PER_DRAFT)
            .map(|_| SavedStage {
                text: "{}".into(),
                enabled: true,
            })
            .collect(),
        ..SavedDraftText::default()
    };
    let t = SavedTab {
        draft: Some(many),
        ..tab(Uuid::new_v4(), "users")
    }
    .bounded();
    assert!(t.draft.is_none() && t.text_dropped);
}

#[test]
fn a_pipe_where_the_file_should_be_does_not_block_the_launch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.json");
    let made = std::process::Command::new("mkfifo").arg(&path).status();
    if !made.is_ok_and(|s| s.success()) {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let known = BTreeSet::new();
    std::thread::spawn(move || {
        let tags = RefCell::new(Vec::new());
        let loaded = load(&path, &recording(&tags), &known);
        let _ = tx.send((loaded.recovery, loaded.guard, tags.borrow().len()));
    });
    let answer = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the launch was blocked by a pipe");
    assert_eq!(answer.0, Some(Recovery::Workspace { backed_up: false }));
    assert_eq!(answer.1, PersistGuard::blocked(), "nothing replaces it");
    assert_eq!(answer.2, 0, "and it was not opened for a backup either");
}

#[test]
fn a_backup_never_reads_more_than_its_limit() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(&dir, &[b'x'; 100]);
    let copy = crate::app::persistence::back_up_limited(&path, "corrupt", Some(10)).unwrap();
    assert_eq!(std::fs::read(copy).unwrap().len(), 10);
}

#[test]
fn the_options_are_saved_and_an_older_file_without_them_loads() {
    use curlybrace_core::pipeline_options::PipelineOptions;
    let id = Uuid::new_v4();
    let options = PipelineOptions {
        comment: Some("why".into()),
        max_time_ms: Some(250),
        allow_disk_use: Some(true),
        ..PipelineOptions::default()
    };
    let t = SavedTab {
        find: Some(SavedFind {
            options: options.clone(),
            ..SavedFind::default()
        }),
        draft: Some(SavedDraftText {
            options: options.clone(),
            text_mode: true,
            ..SavedDraftText::default()
        }),
        ..tab(id, "users")
    };
    let text = serde_json::to_string(&t).unwrap();
    let back: SavedTab = serde_json::from_str(&text).unwrap();
    assert_eq!(back, t);
    let old = r#"{"connection":"00000000-0000-0000-0000-000000000001","db":"a","collection":"b",
        "sub_tab":"documents","view_mode":"table","find":{"filter":"{}","sort":"","projection":"",
        "skip":"","limit":""},"draft":{"stages":[]}}"#;
    let loaded: SavedTab = serde_json::from_str(old).unwrap();
    assert!(loaded.find.unwrap().options.is_empty() && !loaded.draft.unwrap().text_mode);
}

#[test]
fn a_tab_over_256_kib_in_the_file_loads_without_text_and_flagged() {
    let dir = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let big = SavedTab {
        find: Some(SavedFind {
            filter: "x".repeat(MAX_TEXT_PER_TAB + 1),
            ..SavedFind::default()
        }),
        ..tab(id, "users")
    };
    let path = dir.path().join("workspace.json");
    save(&path, &PersistGuard::default(), &file_of(vec![big])).unwrap();
    let loaded = load(&path, &back_up, &known(&[id]));
    assert_eq!(loaded.file.tabs[0].find, None);
    assert!(loaded.file.tabs[0].text_dropped);
}

#[test]
fn a_tab_over_256_kib_is_saved_without_text_and_flagged() {
    let t = two_tabs();
    let mut s = t.s;
    s.find.text.filter = "x".repeat(MAX_TEXT_PER_TAB + 1);
    let file = snapshot(&s);
    let first = &file.tabs[0];
    assert_eq!(first.find, None);
    assert!(first.text_dropped);
    let under = &file.tabs[1];
    assert!(!under.text_dropped);
    let mut s = s;
    s.find.text.filter = "x".repeat(MAX_TEXT_PER_TAB);
    let first_again = snapshot(&s).tabs.remove(0);
    assert!(first_again.find.is_some() && !first_again.text_dropped);
}

#[test]
fn snapshot_reads_the_text_of_every_tab_where_it_lives() {
    let t = two_tabs();
    let (s, _) = act(
        t.s,
        t.second,
        Input::Aggregate(AggregateAction::AddStage("$match".into())),
    );
    let mut s = s;
    s.find.text.filter = r#"{"name":"ann"}"#.into();
    let file = snapshot(&s);
    assert_eq!(file.tabs.len(), 2);
    assert_eq!(file.tabs[0].collection, "users");
    assert_eq!(
        file.tabs[0].find.as_ref().map(|f| f.filter.as_str()),
        Some(r#"{"name":"ann"}"#)
    );
    assert_eq!(file.tabs[1].collection, "orders");
    assert_eq!(file.tabs[1].draft.as_ref().map(|d| d.stages.len()), Some(1));
    assert_eq!(file.active, Some(0));
}

#[test]
fn snapshot_holds_no_page_editor_or_uri() {
    let t = two_tabs();
    let a = t.a;
    let mut s = t.s;
    s.conn.connections[0].uri = "mongodb://user:hunter2pw@db.internal.example:27017".into();
    s.conn.connections[0].username = Some("operator-name".into());
    let page = crate::app::tabs::test_support::page(1);
    let mut docs = page;
    docs.docs = vec![doc! { "secret_field": "needle-document" }];
    let first_find = t.first_find;
    let s = ev(
        s,
        curlybrace_core::runtime::Event::Results {
            session: a,
            ns: users(),
            query: Default::default(),
            page: docs,
            request: first_find,
        },
    );
    let (mut s, _) = update(s, Input::Doc(DocAction::Insert));
    s.editor.as_mut().expect("an editor").text = r#"{"needle-editor": 1}"#.into();
    assert!(s.page.is_some(), "the page is in the state");
    let bytes = serde_json::to_string(&snapshot(&s)).unwrap();
    for needle in [
        "needle-document",
        "needle-editor",
        "hunter2pw",
        "db.internal.example",
        "operator-name",
        "mongodb://",
    ] {
        assert!(!bytes.contains(needle), "{needle} leaked: {bytes}");
    }
    assert!(bytes.contains(&a.to_string()), "the connection id is saved");
}

#[test]
fn snapshot_keeps_the_saved_tabs_of_connections_that_have_not_connected() {
    let t = two_tabs();
    let other = Uuid::new_v4();
    let mut s = t.s;
    s.restore.dormant.insert(other, vec![tab(other, "later")]);
    s.restore.active_hint = Some((other, 0));
    let file = snapshot(&s);
    let names: Vec<&str> = file.tabs.iter().map(|t| t.collection.as_str()).collect();
    assert_eq!(names, ["users", "orders", "later"]);
    assert_eq!(file.active, Some(0), "the tab in front wins");
}

#[test]
fn snapshot_keeps_the_saved_active_tab_while_its_connection_is_not_up() {
    let mut s = AppState::default();
    let other = Uuid::new_v4();
    s.restore
        .dormant
        .insert(other, vec![tab(other, "a"), tab(other, "b")]);
    s.restore.active_hint = Some((other, 1));
    assert_eq!(snapshot(&s).active, Some(1));
}

#[test]
fn saved_file_is_0600() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.json");
    save(&path, &PersistGuard::default(), &WorkspaceFile::default()).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn opted_out_file_has_no_tabs() {
    let t = two_tabs();
    let mut s = t.s;
    s.restore.restore = false;
    let file = snapshot(&s);
    assert_eq!(file, WorkspaceFile::without_tabs(false));
    let text = serde_json::to_string(&file).unwrap();
    assert!(
        !text.contains("users") && !text.contains("orders"),
        "{text}"
    );
    // A file that says off but still lists tabs restores none of them.
    let dir = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4();
    let path = dir.path().join("workspace.json");
    let odd = WorkspaceFile {
        restore: false,
        ..file_of(vec![tab(id, "users")])
    };
    save(&path, &PersistGuard::default(), &odd).unwrap();
    let loaded = load(&path, &back_up, &known(&[id]));
    assert!(!loaded.file.restore && loaded.file.tabs.is_empty());
}

#[test]
fn a_tab_without_a_collection_or_a_saved_connection_is_not_saved() {
    let t = two_tabs();
    let (s, _) = act(
        t.s,
        t.second,
        Input::Sidebar(crate::viewmodel::SidebarAction::Open(Namespace::new(
            "app", "x",
        ))),
    );
    let mut s = s;
    s.conn.connections.clear();
    assert!(
        snapshot(&s).tabs.is_empty(),
        "the embedded session has no id to save"
    );
    let _ = orders();
    let _ = results;
}

#[test]
fn only_the_tabs_that_are_kept_count_as_repaired() {
    let dir = tempfile::tempdir().unwrap();
    let (kept, gone) = (
        "00000000-0000-0000-0000-000000000001",
        "00000000-0000-0000-0000-000000000002",
    );
    let tab_json = |id: &str, name: &str| {
        format!(
            r#"{{"connection":"{id}","db":"a","collection":"{name}","sub_tab":"documents","view_mode":"table",
              "find":{{"filter":"{{}}","sort":"","projection":"","skip":"","limit":"","options":{{"maxTimeMS":0}}}}}}"#
        )
    };
    let text = format!(
        r#"{{"version":1,"restore":true,"tabs":[{},{}]}}"#,
        tab_json(kept, "one"),
        tab_json(gone, "two")
    );
    let path = write(&dir, text.as_bytes());
    let id: Uuid = kept.parse().unwrap();
    let loaded = load(&path, &back_up, &known(&[id]));
    assert_eq!(loaded.file.tabs.len(), 1);
    assert_eq!(
        loaded.recovery,
        Some(Recovery::WorkspaceSkipped {
            count: 1,
            backed_up: true
        })
    );
    assert_eq!(
        loaded.also,
        vec![Recovery::WorkspaceRepaired {
            count: 1,
            backed_up: true
        }],
        "one repaired tab was kept, not two"
    );
}

#[path = "workspace_cut_tests.rs"]
mod cut;
