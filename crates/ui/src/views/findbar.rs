//! Find bar view: filter/sort/projection/skip/limit inputs and paging controls.
use crate::I18n;
use crate::viewmodel::{FindBarState, submit_find};
use eframe::egui;
use mongostudio_core::query::QueryText;
use mongostudio_core::store::Query;

const NUMBER_FIELD_WIDTH: f32 = 70.0;

/// What the user asked for; the caller performs it.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)] // contract: Run carries the Query by value
pub enum FindAction {
    Run(Query),
    Prev,
    Next,
    Clear,
}

/// `has_prev` / `has_next` come from the caller (current skip > 0 / `Page::has_more`).
pub fn show_findbar(
    ui: &mut egui::Ui,
    state: &mut FindBarState,
    has_prev: bool,
    has_next: bool,
    i18n: &I18n,
) -> Option<FindAction> {
    let mut submit = false;
    submit |= text_row(ui, "findbar.filter", &mut state.text.filter, i18n);
    submit |= text_row(ui, "findbar.sort", &mut state.text.sort, i18n);
    submit |= text_row(ui, "findbar.projection", &mut state.text.projection, i18n);
    submit |= number_row(ui, &mut state.text, i18n);
    if let Some(e) = &state.last_error {
        ui.colored_label(ui.visuals().error_fg_color, i18n.error(e));
    }
    let mut action = buttons(ui, has_prev, has_next, &mut submit, i18n);
    if submit {
        action = run_query(state);
    } else if action == Some(FindAction::Clear) {
        *state = FindBarState::default();
    }
    action
}

/// A labelled single-line box; true when Enter was pressed in it.
fn text_row(ui: &mut egui::Ui, key: &str, value: &mut String, i18n: &I18n) -> bool {
    ui.horizontal(|ui| {
        ui.label(i18n.t(key));
        let edit = egui::TextEdit::singleline(value)
            .hint_text(i18n.t(&format!("{key}_hint")))
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

fn buttons(
    ui: &mut egui::Ui,
    has_prev: bool,
    has_next: bool,
    submit: &mut bool,
    i18n: &I18n,
) -> Option<FindAction> {
    ui.horizontal(|ui| {
        let mut action = None;
        if ui.button(i18n.t("findbar.run")).clicked() {
            *submit = true;
        }
        if ui.button(i18n.t("findbar.clear")).clicked() {
            action = Some(FindAction::Clear);
        }
        let prev = ui.add_enabled(has_prev, egui::Button::new(i18n.t("findbar.prev")));
        if prev.clicked() {
            action = Some(FindAction::Prev);
        }
        let next = ui.add_enabled(has_next, egui::Button::new(i18n.t("findbar.next")));
        if next.clicked() {
            action = Some(FindAction::Next);
        }
        action
    })
    .inner
}

/// Parses the bar. On error the state keeps its text and shows the error; no action is emitted.
fn run_query(state: &mut FindBarState) -> Option<FindAction> {
    match submit_find(state) {
        Ok(query) => {
            *state = state.cleared_error();
            Some(FindAction::Run(query))
        }
        Err(failed) => {
            *state = failed;
            None
        }
    }
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

    #[test]
    fn run_query_failure_keeps_text_and_emits_nothing() {
        let mut s = FindBarState {
            text: QueryText {
                filter: "{oops".into(),
                ..Default::default()
            },
            last_error: None,
        };
        let text = s.text.clone();
        assert_eq!(run_query(&mut s), None);
        assert_eq!(s.text, text);
        assert!(s.last_error.is_some());
    }

    #[test]
    fn run_query_success_clears_error_and_emits_query() {
        let mut s = FindBarState {
            last_error: Some(CoreError::InvalidJson("old".into())),
            ..Default::default()
        };
        assert!(matches!(run_query(&mut s), Some(FindAction::Run(_))));
        assert_eq!(s.last_error, None);
    }
}
