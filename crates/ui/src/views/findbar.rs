//! Find bar view: filter/sort/projection/skip/limit inputs and paging controls.
//! Intent resolution lives in `viewmodel::findbar` (pure, tested).
use crate::I18n;
use crate::theme::{filled_button, gate, outlined_field, text_button, tonal_button};
use crate::viewmodel::{FindBarState, FindIntent, pick_intent, resolve_find_intent};
use curlybrace_core::query::QueryText;
use eframe::egui::{self, vec2};

pub use crate::viewmodel::FindAction;

/// Width of the Skip and Limit fields: room for the floating label and a few digits.
const NUMBER_FIELD_WIDTH: f32 = 96.0;

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
        super::error_label::show_error(ui, e, i18n);
    }
    let button = buttons(ui, has_prev, has_next, i18n);
    let intent = pick_intent(button, enter)?;
    let (next, action) = resolve_find_intent(state, intent, has_prev, has_next);
    *state = next;
    action
}

/// The global "check index use automatically" box. The label is the box's accessible name; the
/// action carries the new setting when it was toggled this frame.
pub fn show_automatic_verdict(
    ui: &mut egui::Ui,
    automatic: bool,
    i18n: &I18n,
) -> Option<FindAction> {
    let mut checked = automatic;
    let response = ui.checkbox(&mut checked, i18n.t("query.verdict.automatic"));
    response
        .changed()
        .then_some(FindAction::SetAutomaticVerdict(checked))
}

/// An outlined single-line box whose label floats on the border (and is its accessible name);
/// true when Enter was pressed in it.
fn text_row(ui: &mut egui::Ui, keys: (&str, &str), value: &mut String, i18n: &I18n) -> bool {
    let response = outlined_field(ui, &i18n.t(keys.0), value, &i18n.t(keys.1));
    pressed_enter(ui, &response)
}

fn pressed_enter(ui: &egui::Ui, response: &egui::Response) -> bool {
    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

fn number_row(ui: &mut egui::Ui, text: &mut QueryText, i18n: &I18n) -> bool {
    ui.horizontal_top(|ui| {
        let mut enter = false;
        for (key, value) in [
            ("findbar.skip", &mut text.skip),
            ("findbar.limit", &mut text.limit),
        ] {
            let response = ui
                .allocate_ui(vec2(NUMBER_FIELD_WIDTH, 0.0), |ui| {
                    outlined_field(ui, &i18n.t(key), value, "")
                })
                .inner;
            enter |= pressed_enter(ui, &response);
        }
        enter
    })
    .inner
}

fn buttons(ui: &mut egui::Ui, has_prev: bool, has_next: bool, i18n: &I18n) -> Option<FindIntent> {
    ui.horizontal(|ui| {
        let mut intent = None;
        if filled_button(ui, i18n.t("findbar.run")).clicked() {
            intent = Some(FindIntent::RunButton);
        }
        if tonal_button(ui, i18n.t("findbar.clear")).clicked() {
            intent = Some(FindIntent::ClearButton);
        }
        let prev = gate(ui, has_prev, |ui| text_button(ui, i18n.t("findbar.prev")));
        if prev.clicked() {
            intent = Some(FindIntent::PrevButton);
        }
        let next = gate(ui, has_next, |ui| text_button(ui, i18n.t("findbar.next")));
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
    use curlybrace_core::error::CoreError;

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
