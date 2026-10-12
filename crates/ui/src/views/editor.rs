//! Modal JSON editor for creating and editing a document.
//! Validation and state transitions live in `viewmodel::editor`.
use crate::I18n;
use crate::viewmodel::{EditorAction, EditorIntent, EditorState, resolve_editor};
use eframe::egui;

const EDITOR_ROWS: usize = 22;

pub fn show_editor(
    ctx: &egui::Context,
    state: &mut EditorState,
    i18n: &I18n,
) -> Option<EditorAction> {
    show_editor_with(ctx, state, i18n, true)
}

/// Like [`show_editor`]; `save_enabled == false` greys out Save (a mutation is in flight).
pub fn show_editor_with(
    ctx: &egui::Context,
    state: &mut EditorState,
    i18n: &I18n,
    save_enabled: bool,
) -> Option<EditorAction> {
    let title_key = if state.original_id.is_some() {
        "editor.title_edit"
    } else {
        "editor.title_new"
    };
    let mut intent = None;
    let modal = egui::Modal::new(egui::Id::new("document_editor")).show(ctx, |ui| {
        ui.heading(i18n.t(title_key));
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut state.text)
                    .code_editor()
                    .desired_rows(EDITOR_ROWS)
                    .desired_width(f32::INFINITY),
            );
        });
        if let Some(e) = &state.error {
            super::error_label::show_error(ui, e, i18n);
        }
        intent = buttons(ui, i18n, save_enabled);
    });
    let intent = intent.or_else(|| modal.should_close().then_some(EditorIntent::Cancel))?;
    let (next, action) = resolve_editor(state, intent);
    *state = next;
    action
}

fn buttons(ui: &mut egui::Ui, i18n: &I18n, save_enabled: bool) -> Option<EditorIntent> {
    ui.horizontal(|ui| {
        let mut intent = None;
        let save = ui.add_enabled(save_enabled, egui::Button::new(i18n.t("editor.save")));
        if save.clicked() {
            intent = Some(EditorIntent::Save);
        }
        if ui.button(i18n.t("editor.cancel")).clicked() {
            intent = Some(EditorIntent::Cancel);
        }
        intent
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongostudio_core::test_support::mock_objects::exotic_doc;

    fn frames(state: &mut EditorState) -> Vec<EditorAction> {
        let ctx = egui::Context::default();
        let i18n = I18n::english();
        let mut actions = Vec::new();
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                actions.extend(show_editor(ctx, state, &i18n));
            });
        }
        actions
    }

    #[test]
    fn open_editor_renders_without_action() {
        for mut state in [EditorState::for_new(), EditorState::for_edit(&exotic_doc())] {
            let before = state.clone();
            assert!(frames(&mut state).is_empty());
            assert_eq!(state, before);
        }
    }

    #[test]
    fn editor_with_error_renders_it() {
        let mut state = EditorState::for_new();
        state.text = "{ nope".into();
        let (mut state, _) = resolve_editor(&state, EditorIntent::Save);
        assert!(state.error.is_some());
        assert!(frames(&mut state).is_empty());
    }
}
