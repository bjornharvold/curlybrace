//! The sampling settings (databases the usage sampler skips) and the history cleanup that goes
//! with deleting a saved connection. Every file lives in a tempdir: nothing here may touch the
//! real config directory.
use super::persistence::{PersistError, back_up};
use super::sampling_settings::{SamplingSettings, default_sampling_path, load, save, saved};
use super::*;
use crate::viewmodel::IndexesAction;
use curlybrace_core::runtime::Command;
use curlybrace_core::secrets::MemorySecretStore;
use std::collections::BTreeSet;
use std::path::Path;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

type Commands = UnboundedReceiver<Command>;

fn names(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn settings(items: &[&str]) -> SamplingSettings {
    items
        .iter()
        .fold(SamplingSettings::default(), |s, name| s.excluding(name))
}

/// An app whose runtime is two channels the test holds, with every file in `dir`.
fn app_in(
    dir: &tempfile::TempDir,
    connections: Vec<ConnectionConfig>,
) -> (CurlybraceApp, UnboundedSender<Event>, Commands) {
    let (tx, commands) = tokio::sync::mpsc::unbounded_channel();
    let (events, rx) = tokio::sync::mpsc::unbounded_channel();
    let app = CurlybraceApp::new(
        RuntimeHandle { tx, rx },
        I18n::english(),
        connections,
        Arc::new(MemorySecretStore::default()),
    )
    .with_config_path(Ok(dir.path().join("connections.json")))
    .with_sampling_path(Ok(dir.path().join("sampling.json")));
    (app, events, commands)
}

fn drain(commands: &mut Commands) -> Vec<Command> {
    std::iter::from_fn(|| commands.try_recv().ok()).collect()
}

fn sent_sets(commands: Vec<Command>) -> Vec<BTreeSet<String>> {
    commands
        .into_iter()
        .filter_map(|c| match c {
            Command::SetSamplingExcluded { databases } => Some(databases),
            _ => None,
        })
        .collect()
}

fn purged(commands: Vec<Command>) -> Vec<uuid::Uuid> {
    commands
        .into_iter()
        .filter_map(|c| match c {
            Command::PurgeHistory { connection } => Some(connection),
            _ => None,
        })
        .collect()
}

fn exclude(app: &mut CurlybraceApp, name: &str) {
    app.dispatch(Input::Indexes(IndexesAction::ExcludeDatabase(name.into())));
}

#[test]
fn settings_round_trip_through_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub").join("sampling.json");
    let before = settings(&["logs", "my.db", "a b", "ünï"]);
    save(&path, &before).unwrap();
    let started = load(&path, &back_up);
    assert_eq!(started.settings, before);
    assert_eq!(started.recovery, None);
    assert!(started.guard.check().is_ok());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "private, like connections.json");
    }
    // A missing file is the defaults, with nothing to report.
    let missing = load(&dir.path().join("none.json"), &back_up);
    assert_eq!(missing.settings, SamplingSettings::default());
    assert_eq!(missing.recovery, None);
}

#[test]
fn a_corrupt_settings_file_is_backed_up_and_defaults_apply() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sampling.json");
    std::fs::write(&path, "{ not json").unwrap();
    let started = load(&path, &back_up);
    assert_eq!(started.settings, SamplingSettings::default());
    assert_eq!(
        started.recovery,
        Some(persistence::Recovery::Sampling { backed_up: true })
    );
    assert!(started.guard.check().is_ok());
    let backups: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().contains("sampling.json.corrupt-"))
        .collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(std::fs::read_to_string(&backups[0]).unwrap(), "{ not json");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");

    // A backup that cannot be made turns saving off, so the original is never overwritten.
    let failing = |_: &Path, _: &str| Err(std::io::ErrorKind::PermissionDenied.into());
    let blocked = load(&path, &failing);
    assert_eq!(
        blocked.recovery,
        Some(persistence::Recovery::Sampling { backed_up: false })
    );
    assert_eq!(blocked.guard.check(), Err(PersistError::Blocked));

    // Valid JSON of the wrong shape is just as unusable.
    std::fs::write(&path, r#"{"excluded_dbs": [1, 2]}"#).unwrap();
    assert!(load(&path, &back_up).recovery.is_some());
}

#[test]
fn a_corrupt_file_at_launch_shows_the_recovery_banner_and_sends_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("sampling.json"), "][").unwrap();
    let (app, _events, mut commands) = app_in(&dir, Vec::new());
    assert_eq!(
        app.state().banner,
        Some(Banner::Recovery(persistence::Recovery::Sampling {
            backed_up: true
        }))
    );
    assert_eq!(sent_sets(drain(&mut commands)), vec![BTreeSet::new()]);
}

#[test]
fn excluding_a_database_sends_set_sampling_excluded() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _events, mut commands) = app_in(&dir, Vec::new());
    drain(&mut commands);
    exclude(&mut app, "logs");
    exclude(&mut app, "staging");
    assert_eq!(
        sent_sets(drain(&mut commands)),
        vec![names(&["logs"]), names(&["logs", "staging"])],
        "the full set goes each time"
    );
    assert_eq!(app.state().sampling, settings(&["logs", "staging"]));
    // The change is on disk before the next launch reads it.
    let again = load(&dir.path().join("sampling.json"), &back_up);
    assert_eq!(again.settings, settings(&["logs", "staging"]));
    // Excluding what is already excluded changes nothing and sends nothing.
    exclude(&mut app, "logs");
    assert!(drain(&mut commands).is_empty());
}

#[test]
fn including_a_database_again_sends_the_smaller_set() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _events, mut commands) = app_in(&dir, Vec::new());
    exclude(&mut app, "logs");
    exclude(&mut app, "staging");
    drain(&mut commands);
    app.dispatch(Input::Indexes(IndexesAction::IncludeDatabase(
        "logs".into(),
    )));
    assert_eq!(sent_sets(drain(&mut commands)), vec![names(&["staging"])]);
    let again = load(&dir.path().join("sampling.json"), &back_up);
    assert_eq!(again.settings, settings(&["staging"]));
    // Including a database that is not excluded is a no-op.
    app.dispatch(Input::Indexes(IndexesAction::IncludeDatabase(
        "never-was".into(),
    )));
    assert!(drain(&mut commands).is_empty());
}

#[test]
fn the_exclusion_set_is_sent_once_at_startup() {
    let dir = tempfile::tempdir().unwrap();
    save(
        &dir.path().join("sampling.json"),
        &settings(&["a b", "x.y"]),
    )
    .unwrap();
    let (mut app, _events, mut commands) = app_in(&dir, Vec::new());
    assert_eq!(
        sent_sets(drain(&mut commands)),
        vec![names(&["a b", "x.y"])],
        "before anything else could be sent"
    );
    let ctx = egui::Context::default();
    for _ in 0..3 {
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.run_frame(ctx));
    }
    assert!(
        sent_sets(drain(&mut commands)).is_empty(),
        "frames do not repeat it"
    );
}

#[test]
fn a_failed_save_is_a_banner_but_the_change_still_applies() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
    let (_events, rx) = tokio::sync::mpsc::unbounded_channel();
    let blocker = dir.path().join("file");
    std::fs::write(&blocker, b"x").unwrap();
    let mut app = CurlybraceApp::new(
        RuntimeHandle { tx, rx },
        I18n::english(),
        Vec::new(),
        Arc::new(MemorySecretStore::default()),
    )
    .with_sampling_path(Ok(dir.path().join("sampling.json")));
    // The file read fine at launch; from now on its folder cannot be created.
    app.sampling_path = Ok(blocker.join("sampling.json"));
    drain(&mut commands);
    exclude(&mut app, "logs");
    assert!(matches!(
        app.state().banner,
        Some(Banner::Persist(PersistError::Write(_)))
    ));
    assert_eq!(sent_sets(drain(&mut commands)), vec![names(&["logs"])]);
}

#[test]
fn a_blocked_settings_file_is_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sampling.json");
    std::fs::write(&path, "{ not json").unwrap();
    let guard = persistence::PersistGuard::blocked();
    assert_eq!(saved(&path, &guard, &settings(&["logs"])), Ok(()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
}

#[test]
fn a_new_app_never_writes_to_the_real_config_directory() {
    let (tx, mut commands) = tokio::sync::mpsc::unbounded_channel();
    let (_events, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = CurlybraceApp::new(
        RuntimeHandle { tx, rx },
        I18n::english(),
        Vec::new(),
        Arc::new(MemorySecretStore::default()),
    );
    exclude(&mut app, "logs");
    // No path was given, so the edit is applied but nothing is saved anywhere.
    assert_eq!(
        app.state().banner,
        Some(Banner::Persist(PersistError::ConfigDirUnknown))
    );
    assert_eq!(sent_sets(drain(&mut commands)), vec![names(&["logs"])]);
    // The shipped location sits beside connections.json.
    let shipped = default_sampling_path();
    let connections = persistence::default_connections_path();
    assert_eq!(
        shipped.map(|p| p.parent().map(Path::to_path_buf)),
        connections.map(|p| p.parent().map(Path::to_path_buf))
    );
}

#[test]
fn database_names_are_kept_verbatim_and_empty_names_are_rejected() {
    let kept = ["my.db", "with space", "ünï-ç☃", "UPPER", "a--b", "x$y"];
    let all = kept
        .iter()
        .fold(SamplingSettings::default(), |s, n| s.excluding(n));
    assert_eq!(all.excluded_dbs, names(&kept));
    // Only the ends are trimmed; the interior stays exactly as typed.
    let trimmed = SamplingSettings::default().excluding("  two  words \t");
    assert_eq!(trimmed.excluded_dbs, names(&["two  words"]));
    // Exact equality: case matters.
    assert_ne!(
        SamplingSettings::default().excluding("Logs"),
        SamplingSettings::default().excluding("logs")
    );
    // Nothing, or only whitespace, is not a name.
    for empty in ["", " ", "\t\n  "] {
        assert_eq!(
            SamplingSettings::default().excluding(empty),
            SamplingSettings::default(),
            "{empty:?}"
        );
    }
    // The built-in skip list is accepted like any other name.
    assert_eq!(
        SamplingSettings::default().excluding("admin").excluded_dbs,
        names(&["admin"])
    );
    // And the names survive the file unchanged.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sampling.json");
    save(&path, &all).unwrap();
    assert_eq!(load(&path, &back_up).settings, all);
}

#[test]
fn rejected_input_changes_and_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _events, mut commands) = app_in(&dir, Vec::new());
    drain(&mut commands);
    exclude(&mut app, "   ");
    exclude(&mut app, "");
    assert!(drain(&mut commands).is_empty());
    assert_eq!(app.state().sampling, SamplingSettings::default());
    assert_eq!(app.state().banner, None);
}

fn saved_connections(n: usize) -> Vec<ConnectionConfig> {
    (0..n)
        .map(|i| ConnectionConfig::new(&format!("c{i}"), "mongodb://h"))
        .collect()
}

#[test]
fn deleting_a_connection_sends_purge_history_for_its_id() {
    let dir = tempfile::tempdir().unwrap();
    let list = saved_connections(2);
    let (gone, kept) = (list[0].id, list[1].id);
    let (mut app, _events, mut commands) = app_in(&dir, list);
    drain(&mut commands);
    app.dispatch_connection(ConnectionAction::Delete(gone));
    assert!(app.state().connection(gone).is_none());
    assert_eq!(purged(drain(&mut commands)), vec![gone]);
    // Deleting it again, or one that never existed, purges nothing more.
    app.dispatch_connection(ConnectionAction::Delete(gone));
    app.dispatch_connection(ConnectionAction::Delete(uuid::Uuid::new_v4()));
    assert!(purged(drain(&mut commands)).is_empty());
    assert!(app.state().connection(kept).is_some());
}

#[test]
fn deleting_an_open_connection_closes_it_before_its_history_goes() {
    let dir = tempfile::tempdir().unwrap();
    let list = saved_connections(1);
    let id = list[0].id;
    let (mut app, _events, mut commands) = app_in(&dir, list);
    app.dispatch(Input::Connect(id));
    drain(&mut commands);
    app.dispatch_connection(ConnectionAction::Delete(id));
    let sent = drain(&mut commands);
    let order: Vec<&str> = sent
        .iter()
        .map(|c| match c {
            Command::Disconnect { .. } => "disconnect",
            Command::PurgeHistory { .. } => "purge",
            _ => "other",
        })
        .collect();
    assert_eq!(order, ["disconnect", "purge"], "the sampler stops first");
}

#[test]
fn a_refused_delete_purges_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let list = saved_connections(1);
    let id = list[0].id;
    let (app, _events, mut commands) = app_in(&dir, list);
    let mut app = app.with_persist_guard(persistence::PersistGuard::blocked());
    drain(&mut commands);
    app.dispatch_connection(ConnectionAction::Delete(id));
    assert!(app.state().connection(id).is_some(), "the guard refused");
    assert!(purged(drain(&mut commands)).is_empty());
}

/// Deleting a folder moves its connections to the top level; it removes none, so there is no
/// history to purge. Whatever a folder delete does remove is purged once each.
#[test]
fn deleting_a_folder_purges_every_connection_in_it_exactly_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut list = saved_connections(3);
    list[0].folder = Some("work".into());
    list[1].folder = Some("work".into());
    let ids: Vec<_> = list.iter().map(|c| c.id).collect();
    let (mut app, _events, mut commands) = app_in(&dir, list);
    drain(&mut commands);
    app.dispatch_connection(ConnectionAction::DeleteFolder("work".into()));
    let removed: Vec<_> = ids
        .iter()
        .copied()
        .filter(|id| app.state().connection(*id).is_none())
        .collect();
    let mut sent = purged(drain(&mut commands));
    sent.sort();
    let mut expected = removed;
    expected.sort();
    assert_eq!(sent, expected, "one purge per removed connection");
    assert!(
        expected.is_empty(),
        "a folder delete keeps its connections, so their history stays"
    );
    assert!(ids.iter().all(|id| app.state().connection(*id).is_some()));
}

#[test]
fn the_ids_removed_between_two_lists_are_purged_once_each() {
    let list = saved_connections(3);
    let before: Vec<_> = list.iter().map(|c| c.id).collect();
    let state = AppState::new(vec![list[1].clone()]);
    let commands = connection_ops::purge_removed(&before, &state);
    let mut sent = purged(commands);
    sent.sort();
    let mut expected = vec![list[0].id, list[2].id];
    expected.sort();
    assert_eq!(sent, expected);
    // A duplicated id in the list before is still purged once.
    let doubled = [before[0], before[0]];
    let none = AppState::new(Vec::new());
    assert_eq!(
        purged(connection_ops::purge_removed(&doubled, &none)),
        vec![before[0]]
    );
}

/// Answers an import request with a fixed file.
struct ImportsFrom(PathBuf);

impl FilePicker for ImportsFrom {
    fn save_file(&self, _: &str, _: &str) -> Option<PathBuf> {
        None
    }
    fn open_file(&self, _: &str) -> Option<PathBuf> {
        Some(self.0.clone())
    }
}

#[test]
fn importing_or_disconnecting_never_purges() {
    let dir = tempfile::tempdir().unwrap();
    let list = saved_connections(1);
    let id = list[0].id;
    // A file that imports one new connection and one that overwrites nothing of the saved list.
    let file = dir.path().join("in.json");
    let incoming = vec![ConnectionConfig::new("imported", "mongodb://i")];
    std::fs::write(&file, serde_json::to_vec(&incoming).unwrap()).unwrap();
    let (app, _events, mut commands) = app_in(&dir, list);
    let mut app = app.with_file_picker(Arc::new(ImportsFrom(file)), Arc::new(|| {}));
    app.dispatch(Input::Connect(id));
    drain(&mut commands);

    app.dispatch_connection(ConnectionAction::StartImport);
    let start = std::time::Instant::now();
    while app.state().conn.connections.len() < 2 {
        app.pump();
        assert!(start.elapsed().as_secs() < 5, "the import never finished");
        std::thread::yield_now();
    }
    app.dispatch(Input::Disconnect(id));
    let sent = drain(&mut commands);
    assert!(purged(sent.clone()).is_empty(), "{sent:?}");
    assert!(
        sent.iter().any(|c| matches!(c, Command::Disconnect { .. })),
        "the disconnect itself was sent"
    );
    assert_eq!(app.state().conn.connections.len(), 2);
}
