//! The opt-out: turning "Reopen tabs at launch" off erases what was saved.
use crate::app::pipelines_wiring_tests::app_in;
use crate::app::reducer::Input;
use crate::app::tools_state::ToolsAction;
use crate::app::workspace_file::WorkspaceFile;
use crate::viewmodel::SidebarAction;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::runtime::Event;
use curlybrace_core::store::Namespace;
use eframe::egui;
use std::time::Instant;

const SENTINEL: &str = "SAVED_QUERY_TEXT_91c2";

/// A connected app with one tab whose typed filter has been written to the file.
fn written() -> (
    crate::app::CurlybraceApp,
    tempfile::TempDir,
    ConnectionConfig,
) {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let (app, events, _commands) = app_in(&dir, vec![cfg.clone()]);
    let mut app = app.with_workspace_path(Ok(dir.path().join("workspace.json")));
    app.dispatch(Input::Connect(cfg.id));
    let epoch = app.state().epoch_counter;
    events
        .send(Event::Connected {
            session: cfg.id,
            epoch,
            topology: None,
        })
        .unwrap();
    app.pump();
    app.dispatch(Input::Sidebar(SidebarAction::Open(Namespace::new(
        "app", "users",
    ))));
    app.state_mut().find.text.filter = format!(r#"{{"name": "{SENTINEL}"}}"#);
    app.save_workspace_now();
    let text = std::fs::read_to_string(dir.path().join("workspace.json")).unwrap();
    assert!(text.contains(SENTINEL), "the text was saved: {text}");
    std::mem::forget(events);
    (app, dir, cfg)
}

fn step(app: &mut crate::app::CurlybraceApp) {
    app.save_workspace_if_changed(&egui::Context::default(), Instant::now());
}

#[test]
fn turning_restore_off_erases_saved_text_immediately() {
    let (mut app, dir, _) = written();
    app.dispatch(Input::Tools(ToolsAction::SetRestore(false)));
    step(&mut app);
    let text = std::fs::read_to_string(dir.path().join("workspace.json")).unwrap();
    assert!(!text.contains(SENTINEL), "the text is gone: {text}");
    let file: WorkspaceFile = serde_json::from_str(&text).unwrap();
    assert_eq!(file, WorkspaceFile::without_tabs(false));
    // Typing afterwards saves nothing.
    app.state_mut().find.text.filter = format!(r#"{{"x": "{SENTINEL}"}}"#);
    app.save_workspace_now();
    let text = std::fs::read_to_string(dir.path().join("workspace.json")).unwrap();
    assert!(!text.contains(SENTINEL), "{text}");
}

#[test]
fn turning_restore_off_clears_dormant_tabs() {
    let (app, dir, cfg) = written();
    drop(app);
    // A new launch holds the tab, waiting for its connection.
    let (second, _events, _commands) = app_in(&dir, vec![cfg.clone()]);
    let mut second = second.with_workspace_path(Ok(dir.path().join("workspace.json")));
    assert_eq!(second.state().restore.dormant[&cfg.id].len(), 1);
    second.dispatch(Input::Tools(ToolsAction::SetRestore(false)));
    assert!(second.state().restore.dormant.is_empty());
    assert_eq!(second.state().restore.active_hint, None);
    assert!(!second.state().restore.restore);
    step(&mut second);
    let text = std::fs::read_to_string(dir.path().join("workspace.json")).unwrap();
    assert!(
        !text.contains(SENTINEL),
        "the waiting tabs' text is gone too: {text}"
    );
}

#[test]
fn restore_off_file_restores_nothing_next_launch() {
    let (mut app, dir, cfg) = written();
    app.dispatch(Input::Tools(ToolsAction::SetRestore(false)));
    step(&mut app);
    drop(app);
    let (next, _events, _commands) = app_in(&dir, vec![cfg]);
    let next = next.with_workspace_path(Ok(dir.path().join("workspace.json")));
    assert!(next.state().restore.dormant.is_empty());
    assert!(!next.state().restore.restore);
}

#[test]
fn turning_restore_back_on_saves_the_open_tabs_again() {
    let (mut app, dir, _) = written();
    app.dispatch(Input::Tools(ToolsAction::SetRestore(false)));
    step(&mut app);
    app.dispatch(Input::Tools(ToolsAction::SetRestore(true)));
    step(&mut app);
    let text = std::fs::read_to_string(dir.path().join("workspace.json")).unwrap();
    assert!(
        text.contains(SENTINEL) && text.contains(r#""restore": true"#),
        "{text}"
    );
}
