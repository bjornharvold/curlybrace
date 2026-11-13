//! Find bar view: the Filter with Find / Clear / Previous / Next and the "More" toggle on one
//! wrapping row, then, behind "More", Sort / Projection / Skip / Limit on another.
//! Intent resolution lives in `viewmodel::findbar` (pure, tested).
use super::findbar_more::{show_more_fields, show_more_toggle};
use crate::I18n;
use crate::theme::{filled_button, gate, outlined_field, text_button, tonal_button};
use crate::viewmodel::findbar_more::{hidden_count, more_open};
use crate::viewmodel::{FindBarState, FindIntent, pick_intent, resolve_find_intent};
use eframe::egui::{self, vec2};

pub use crate::viewmodel::FindAction;

/// The Filter is never narrower than this beside the buttons; on a narrower row the buttons
/// wrap under it and the Filter takes the whole row.
const FILTER_MIN_WIDTH: f32 = 240.0;
/// The width of the buttons after the Filter until a frame has measured it.
const TRAILING_GUESS: f32 = 440.0;
/// Room left between the Filter and the buttons' measured width, so rounding never wraps the
/// last button.
const ROW_SLACK: f32 = 1.0;

const KEY_FILTER: (&str, &str) = ("findbar.filter", "findbar.filter_hint");

/// `has_prev` / `has_next` come from the caller (current skip > 0 / `Page::has_more`).
pub fn show_findbar(
    ui: &mut egui::Ui,
    state: &mut FindBarState,
    has_prev: bool,
    has_next: bool,
    i18n: &I18n,
) -> Option<FindAction> {
    let more = MoreToggle {
        open: more_open(state),
        hidden: hidden_count(&state.text),
    };
    let row = query_row(ui, &mut state.text.filter, (has_prev, has_next), more, i18n);
    let more_enter = more.open && show_more_fields(ui, &mut state.text, i18n);
    if let Some(e) = &state.last_error {
        super::error_label::show_error(ui, e, i18n);
    }
    if row.toggled {
        *state = state.more_toggled();
    }
    let intent = pick_intent(row.button, row.enter || more_enter)?;
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

/// The global "warn before queries that would scan a collection" box; the tooltip says what the
/// check costs. Same shape as [`show_automatic_verdict`].
pub fn show_guard_setting(ui: &mut egui::Ui, guard: bool, i18n: &I18n) -> Option<FindAction> {
    let mut checked = guard;
    let response = ui
        .checkbox(&mut checked, i18n.t("query.guard.setting"))
        .on_hover_text(i18n.t("query.guard.setting_hint"));
    response.changed().then_some(FindAction::SetGuard(checked))
}

/// What the "More" toggle shows: whether the fields behind it are open and how many hold a
/// value.
#[derive(Clone, Copy)]
struct MoreToggle {
    open: bool,
    hidden: usize,
}

/// What the person did on the Filter row this frame.
struct RowOutcome {
    enter: bool,
    button: Option<FindIntent>,
    toggled: bool,
}

/// The Filter, as wide as the row allows beside the buttons, then Find / Clear / Previous /
/// Next and the "More" toggle; the row wraps on a narrow window. The buttons' width is measured
/// each frame (a change re-runs the frame unseen), so the Filter's width needs no guessing.
fn query_row(
    ui: &mut egui::Ui,
    filter: &mut String,
    (has_prev, has_next): (bool, bool),
    more: MoreToggle,
    i18n: &I18n,
) -> RowOutcome {
    let memory = ui.id().with("findbar_trailing_width");
    let trailing = ui.data(|d| d.get_temp::<f32>(memory));
    ui.horizontal_wrapped(|ui| {
        let width = filter_width(ui.available_width(), trailing.unwrap_or(TRAILING_GUESS));
        let enter = sized_field(ui, width, KEY_FILTER, filter, i18n);
        let (button, buttons_width) = buttons(ui, has_prev, has_next, i18n);
        let toggle = show_more_toggle(ui, more.open, more.hidden, i18n);
        let measured = buttons_width + toggle.rect.width() + ui.spacing().item_spacing.x;
        if trailing.is_none_or(|known| (known - measured).abs() > 0.5) {
            ui.data_mut(|d| d.insert_temp(memory, measured));
            ui.ctx().request_discard("find bar buttons measured");
        }
        RowOutcome {
            enter,
            button,
            toggled: toggle.clicked(),
        }
    })
    .inner
}

/// The Filter's width: what the buttons leave, or the whole row when that is too narrow.
fn filter_width(available: f32, trailing: f32) -> f32 {
    let beside = available - trailing - ROW_SLACK;
    if beside >= FILTER_MIN_WIDTH {
        beside
    } else {
        available
    }
}

/// An outlined single-line box `width` wide whose label floats on the border (and is its
/// accessible name); true when Enter was pressed in it.
pub(super) fn sized_field(
    ui: &mut egui::Ui,
    width: f32,
    (label, hint): (&str, &str),
    value: &mut String,
    i18n: &I18n,
) -> bool {
    let hint = match hint.is_empty() {
        true => String::new(),
        false => i18n.t(hint),
    };
    let response = ui
        .allocate_ui(vec2(width, 0.0), |ui| {
            outlined_field(ui, &i18n.t(label), value, &hint)
        })
        .inner;
    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

/// Find, Clear, Previous and Next in the current row; the intent clicked and the width they
/// took with the gap before each.
fn buttons(
    ui: &mut egui::Ui,
    has_prev: bool,
    has_next: bool,
    i18n: &I18n,
) -> (Option<FindIntent>, f32) {
    let gap = ui.spacing().item_spacing.x;
    let mut intent = None;
    let mut width = 0.0;
    let run = filled_button(ui, i18n.t("findbar.run"));
    let clear = tonal_button(ui, i18n.t("findbar.clear"));
    let prev = gate(ui, has_prev, |ui| text_button(ui, i18n.t("findbar.prev")));
    let next = gate(ui, has_next, |ui| text_button(ui, i18n.t("findbar.next")));
    for (response, clicked) in [
        (run, FindIntent::RunButton),
        (clear, FindIntent::ClearButton),
        (prev, FindIntent::PrevButton),
        (next, FindIntent::NextButton),
    ] {
        width += response.rect.width() + gap;
        if response.clicked() {
            intent = Some(clicked);
        }
    }
    (intent, width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use curlybrace_core::error::CoreError;
    use curlybrace_core::query::QueryText;

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
            more: None,
        };
        let before = s.clone();
        assert_eq!(run_frame(&mut s, true, true), None);
        assert_eq!(s, before, "an idle frame must not change the state");
    }
}
