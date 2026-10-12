//! Find bar view: filter/sort/projection/skip/limit inputs and paging controls.
//! Intent resolution lives in `viewmodel::findbar` (pure, tested).
use crate::I18n;
use crate::viewmodel::{FindBarState, FindIntent, pick_intent, resolve_find_intent};
use eframe::egui;
use mongostudio_core::query::QueryText;

pub use crate::viewmodel::FindAction;

const NUMBER_FIELD_WIDTH: f32 = 70.0;

const KEY_FILTER: (&str, &str) = ("findbar.filter", "findbar.filter_hint");
const KEY_SORT: (&str, &str) = ("findbar.sort", "findbar.sort_hint");
const KEY_PROJECTION: (&str, &str) = ("findbar.projection", "findbar.projection_hint");

/// `has_prev` / `has_next` come from the caller (current skip > 0 / `Page::has_more`).
pub fn show_findbar(
    ui: &mut egui::Ui,
    state: &mut FindBarState,
    has_prev: bool,
    has_next: bool,
    i18n: &I18n,
) -> Option<FindAction> {
    let mut enter = false;
    enter |= text_row(ui, KEY_FILTER, &mut state.text.filter, i18n);
    enter |= text_row(ui, KEY_SORT, &mut state.text.sort, i18n);
    enter |= text_row(ui, KEY_PROJECTION, &mut state.text.projection, i18n);
    enter |= number_row(ui, &mut state.text, i18n);
    if let Some(e) = &state.last_error {
        ui.colored_label(ui.visuals().error_fg_color, i18n.error(e));
    }
    let button = buttons(ui, has_prev, has_next, i18n);
    let intent = pick_intent(button, enter)?;
    let (next, action) = resolve_find_intent(state, intent, has_prev, has_next);
    *state = next;
    action
}

/// A labelled single-line box; true when Enter was pressed in it.
fn text_row(ui: &mut egui::Ui, keys: (&str, &str), value: &mut String, i18n: &I18n) -> bool {
    ui.horizontal(|ui| {
        ui.label(i18n.t(keys.0));
        let edit = egui::TextEdit::singleline(value)
            .hint_text(i18n.t(keys.1))
            .desired_width(f32::INFINITY);
        let response = ui.add(edit);
        response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
    })
    .inner
}

fn number_row(ui: &mut egui::Ui, text: &mut QueryText, i18n: &I18n) -> bool {
    ui.horizontal(|ui| {
        let mut enter = false;
        for (key, value) in [
            ("findbar.skip", &mut text.skip),
            ("findbar.limit", &mut text.limit),
        ] {
            ui.label(i18n.t(key));
            let edit = egui::TextEdit::singleline(value).desired_width(NUMBER_FIELD_WIDTH);
            let response = ui.add(edit);
            enter |= response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        }
        enter
    })
    .inner
}

fn buttons(ui: &mut egui::Ui, has_prev: bool, has_next: bool, i18n: &I18n) -> Option<FindIntent> {
    ui.horizontal(|ui| {
        let mut intent = None;
        if ui.button(i18n.t("findbar.run")).clicked() {
            intent = Some(FindIntent::RunButton);
        }
        if ui.button(i18n.t("findbar.clear")).clicked() {
            intent = Some(FindIntent::ClearButton);
        }
        let prev = ui.add_enabled(has_prev, egui::Button::new(i18n.t("findbar.prev")));
        if prev.clicked() {
            intent = Some(FindIntent::PrevButton);
        }
        let next = ui.add_enabled(has_next, egui::Button::new(i18n.t("findbar.next")));
        if next.clicked() {
            intent = Some(FindIntent::NextButton);
        }
        intent
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongostudio_core::error::CoreError;

    fn run_frame(state: &mut FindBarState, prev: bool, next: bool) -> Option<FindAction> {
        let ctx = egui::Context::default();
        let i18n = I18n::english();
        let mut out = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                out = show_findbar(ui, state, prev, next, &i18n);
            });
        });
        out
    }

    #[test]
    fn renders_default_and_error_state_without_panicking() {
        assert_eq!(run_frame(&mut FindBarState::default(), false, false), None);
        let mut s = FindBarState {
            text: QueryText {
                filter: "{ \"имя\": \"a b.c\" }".into(),
                limit: "10".into(),
                ..Default::default()
            },
            last_error: Some(CoreError::InvalidJson("filter: x".into())),
        };
        let before = s.clone();
        assert_eq!(run_frame(&mut s, true, true), None);
        assert_eq!(s, before, "an idle frame must not change the state");
    }
}
