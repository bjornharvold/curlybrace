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
    assert!(
        state.folder_prompt.is_some(),
        "stays open until the handler succeeds"
    );
    assert!(
        state.folder_prompt_confirmed().is_some(),
        "emits once per click"
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
    state.error = Some(CoreError::InvalidFolderName);
    state.close_folder_prompt();
    assert_eq!(state.error, None, "cancel clears the rejection");
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
    state.notice = Some(Notice::AppliesOnReconnect);
    assert_eq!(run_frame(&mut state), None);
    state.notice = Some(Notice::Imported {
        imported: 2,
        skipped: 1,
    });
    assert!(state.folder_prompt.is_some(), "still open");
    state.open_rename_prompt("Work");
    assert_eq!(run_frame(&mut state), None);
    state.close_folder_prompt();
    state.filter = "nothing matches this".into();
    assert_eq!(run_frame(&mut state), None);
}

fn run_frame_with(
    ctx: &egui::Context,
    state: &mut ConnectionsState,
    events: Vec<egui::Event>,
) -> Option<ConnectionAction> {
    let i18n = I18n::english();
    let mut out = None;
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1200.0, 900.0),
        )),
        events,
        ..Default::default()
    };
    let _ = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            out = show_connections(ui, state, &i18n);
        });
    });
    out
}

fn click_at(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn a_click_outside_the_edit_dialog_keeps_it_open_and_the_typed_form() {
    let ctx = egui::Context::default();
    let mut state = ConnectionsState::default();
    state.start_new();
    state.form.name = "typed but not saved".into();
    run_frame_with(&ctx, &mut state, vec![]);
    let outside = egui::pos2(8.0, 8.0);
    run_frame_with(&ctx, &mut state, vec![egui::Event::PointerMoved(outside)]);
    run_frame_with(&ctx, &mut state, vec![click_at(outside, true)]);
    run_frame_with(&ctx, &mut state, vec![click_at(outside, false)]);
    run_frame_with(&ctx, &mut state, vec![]);
    assert!(state.editing, "a backdrop click must not close the dialog");
    assert_eq!(state.form.name, "typed but not saved");
}

#[test]
fn escape_still_closes_the_edit_dialog() {
    let ctx = egui::Context::default();
    let mut state = ConnectionsState::default();
    state.start_new();
    run_frame_with(&ctx, &mut state, vec![]);
    run_frame_with(
        &ctx,
        &mut state,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(!state.editing);
}

/// Every text painted in one frame of the manager.
fn painted_texts(state: &mut ConnectionsState) -> Vec<String> {
    fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    let i18n = I18n::english();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1200.0, 900.0),
        )),
        ..Default::default()
    };
    let output = ctx.run(input, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            show_connections(ui, state, &i18n);
        });
    });
    let mut texts = Vec::new();
    output
        .shapes
        .iter()
        .for_each(|clipped| collect(&clipped.shape, &mut texts));
    texts
}

#[test]
fn the_row_connect_buttons_keep_their_plain_visible_text() {
    let mut state = ConnectionsState::new(vec![in_folder("a", None), in_folder("b", None)]);
    let texts = painted_texts(&mut state);
    assert_eq!(texts.iter().filter(|t| t.as_str() == "Connect").count(), 2);
    assert!(texts.iter().all(|t| !t.starts_with("Connect ")));
}
