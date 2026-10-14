//! State and frame tests for the manager list and the folder prompt.
use super::connections::{
    ConnectionAction, ConnectionsState, FolderPrompt, Notice, show_connections,
};
use crate::I18n;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use eframe::egui;

fn in_folder(name: &str, folder: Option<&str>) -> ConnectionConfig {
    let mut cfg = ConnectionConfig::new(name, "mongodb://h:27017");
    cfg.folder = folder.map(str::to_owned);
    cfg
}

fn run_frame(state: &mut ConnectionsState) -> Option<ConnectionAction> {
    let ctx = egui::Context::default();
    let i18n = I18n::english();
    let mut out = None;
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            out = show_connections(ui, state, &i18n);
        });
    });
    out
}

#[test]
fn the_move_prompt_is_prefilled_and_confirming_emits_once() {
    let cfg = in_folder("a", Some("Work"));
    let mut state = ConnectionsState::new(vec![cfg.clone()]);
    state.open_move_prompt(cfg.id);
    assert_eq!(
        state.folder_prompt,
        Some(FolderPrompt::Move {
            id: cfg.id,
            input: "Work".into()
        })
    );
    assert_eq!(
        state.folder_prompt_confirmed(),
        Some(ConnectionAction::MoveToFolder {
            id: cfg.id,
            folder: "Work".into()
        })
    );
    assert_eq!(state.folder_prompt, None);
    assert_eq!(
        state.folder_prompt_confirmed(),
        None,
        "emitted exactly once"
    );
}

#[test]
fn the_rename_prompt_emits_the_typed_name_and_cancel_closes_it() {
    let mut state = ConnectionsState::default();
    state.open_rename_prompt("Work");
    if let Some(FolderPrompt::Rename { input, .. }) = state.folder_prompt.as_mut() {
        *input = "Play".into();
    }
    assert_eq!(
        state.folder_prompt_confirmed(),
        Some(ConnectionAction::RenameFolder {
            from: "Work".into(),
            to: "Play".into()
        })
    );
    state.open_rename_prompt("Work");
    state.close_folder_prompt();
    assert_eq!(state.folder_prompt, None);
}

#[test]
fn folder_names_are_distinct_and_sorted() {
    let state = ConnectionsState::new(vec![
        in_folder("a", Some("b")),
        in_folder("c", None),
        in_folder("d", Some("a")),
        in_folder("e", Some("b")),
    ]);
    assert_eq!(state.folder_names(), vec!["a", "b"]);
}

#[test]
fn editing_the_filter_or_selecting_clears_the_notice() {
    let cfg = in_folder("a", None);
    let mut state = ConnectionsState::new(vec![cfg.clone()]);
    state.notice = Some(Notice::Exported { count: 1 });
    state.filter_edited();
    assert_eq!(state.notice, None);
    state.notice = Some(Notice::Exported { count: 1 });
    state.select(&cfg);
    assert_eq!(state.notice, None);
}

#[test]
fn the_screen_renders_a_notice_and_both_prompts_without_panicking() {
    let cfg = in_folder("a", Some("Work"));
    let mut state = ConnectionsState::new(vec![cfg.clone(), in_folder("b", None)]);
    state.notice = Some(Notice::Imported {
        imported: 2,
        skipped: 1,
    });
    state.error = Some(CoreError::InvalidUri("x".into()));
    state.open_move_prompt(cfg.id);
    assert_eq!(run_frame(&mut state), None);
    assert!(state.folder_prompt.is_some(), "still open");
    state.open_rename_prompt("Work");
    assert_eq!(run_frame(&mut state), None);
    state.close_folder_prompt();
    state.filter = "nothing matches this".into();
    assert_eq!(run_frame(&mut state), None);
}
