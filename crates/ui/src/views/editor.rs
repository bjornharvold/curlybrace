//! Modal JSON editor for creating and editing a document.
//! Validation and state transitions live in `viewmodel::editor`.
use crate::I18n;
use crate::theme::typography::dialog_title;
use crate::theme::{filled_button, gate, modal_frame, text_button};
use crate::viewmodel::{EditorAction, EditorIntent, EditorState, dismissal_action, resolve_editor};
use eframe::egui;

const EDITOR_ROWS: usize = 22;

pub fn show_editor(
    ctx: &egui::Context,
    state: &mut EditorState,
    connection: &str,
    i18n: &I18n,
) -> Option<EditorAction> {
    show_editor_with(ctx, state, connection, i18n, true)
}

/// Like [`show_editor`]; `save_enabled == false` greys out Save (a mutation is in flight).
pub fn show_editor_with(
    ctx: &egui::Context,
    state: &mut EditorState,
    connection: &str,
    i18n: &I18n,
    save_enabled: bool,
) -> Option<EditorAction> {
    let title_key = if state.original_id.is_some() {
        "editor.title_edit"
    } else {
        "editor.title_new"
    };
    let mut intent = None;
    let modal = egui::Modal::new(egui::Id::new("document_editor"))
        .frame(modal_frame(ctx))
        .show(ctx, |ui| {
            let title = ui.label(dialog_title(ui, i18n.t(title_key)));
            ui.label(i18n.t_with("editor.writing_to", &[("connection", connection)]));
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut state.text)
                        .code_editor()
                        .desired_rows(EDITOR_ROWS)
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(title.id);
            });
            if let Some(e) = &state.error {
                super::error_label::show_error(ui, e, i18n);
            }
            intent = buttons(ui, i18n, save_enabled);
        });
    let Some(intent) = intent else {
        return modal
            .should_close()
            .then(|| dismissal_action(state))
            .flatten();
    };
    let (next, action) = resolve_editor(state, intent);
    *state = next;
    action
}

fn buttons(ui: &mut egui::Ui, i18n: &I18n, save_enabled: bool) -> Option<EditorIntent> {
    ui.horizontal(|ui| {
        let mut intent = None;
        let save = gate(ui, save_enabled, |ui| {
            filled_button(ui, i18n.t("editor.save"))
        });
        if save.clicked() {
            intent = Some(EditorIntent::Save);
        }
        if text_button(ui, i18n.t("editor.cancel")).clicked() {
            intent = Some(EditorIntent::Cancel);
        }
        intent
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use curlybrace_core::test_support::mock_objects::exotic_doc;

    fn frames(state: &mut EditorState) -> Vec<EditorAction> {
        let ctx = egui::Context::default();
        let i18n = I18n::english();
        let mut actions = Vec::new();
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                actions.extend(show_editor(ctx, state, "test", &i18n));
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

    /// Every filled rectangle in `shape` (a frame with a shadow is a `Shape::Vec`).
    fn collect_rects(shape: &egui::Shape, out: &mut Vec<(egui::Color32, egui::CornerRadius)>) {
        match shape {
            egui::Shape::Rect(r) => out.push((r.fill, r.corner_radius)),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect_rects(s, out)),
            _ => {}
        }
    }

    #[test]
    fn the_editor_sits_on_the_themed_modal_frame() {
        use crate::theme::shape::{EXTRA_LARGE, radius};
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let (i18n, mut state) = (I18n::english(), EditorState::for_new());
        let mut output = None;
        for _ in 0..8 {
            output = Some(ctx.run(egui::RawInput::default(), |ctx| {
                show_editor(ctx, &mut state, "test", &i18n);
            }));
        }
        let high = crate::theme::current(&ctx).surface_container_high;
        let mut rects = Vec::new();
        for clipped in &output.expect("ran").shapes {
            collect_rects(&clipped.shape, &mut rects);
        }
        let framed = rects
            .iter()
            .any(|(fill, corners)| *fill == high && *corners == radius(EXTRA_LARGE));
        assert!(framed, "no modal frame among {rects:?}");
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
