//! Find bar view: the Filter with Find / Clear / Previous / Next and the "More" toggle on one
//! wrapping row, then, behind "More", Sort / Projection / Skip / Limit and the Query options
//! toggle on another, and the Query options section under it.
//! Intent resolution lives in `viewmodel::findbar` (pure, tested).
use super::code_export_entry::{find_blocked, show_copy_code_icon};
use super::findbar_cell::{field_cell, field_row_height};
use super::findbar_more::{MoreOutcome, show_more_fields, show_more_toggle};
use super::findbar_options::{SectionOutcome, show_options_section};
use crate::I18n;
use crate::theme::{filled_button, gate, outlined_field, text_button, tonal_button};
use crate::viewmodel::aggregate_hint_list::HintListUi;
use crate::viewmodel::findbar_more::more_open;
use crate::viewmodel::findbar_options::{more_set_count, options_open, options_set_count};
use crate::viewmodel::{FindBarState, FindIntent, pick_intent, resolve_find_intent};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::COMMAND_TIMEOUT_MESSAGE;
use eframe::egui::{self, vec2};

pub use crate::viewmodel::FindAction;

/// The Filter is never narrower than this beside the buttons; on a narrower row the buttons
/// wrap under it and the Filter takes the whole row.
const FILTER_MIN_WIDTH: f32 = 220.0;
/// The width of the buttons after the Filter until a frame has measured it (about 475 with the
/// icon-only Copy as code button; the Filter keeps one row with them at the 1000-wide window).
const TRAILING_GUESS: f32 = 475.0;
/// Room left between the Filter and the buttons' measured width, so rounding never wraps the
/// last button.
const ROW_SLACK: f32 = 1.0;

const KEY_FILTER: (&str, &str) = ("findbar.filter", "findbar.filter_hint");

/// `has_prev` / `has_next` come from the caller (current skip > 0 / `Page::has_more`). Without
/// an index list for the Query options' hint picker (see [`show_findbar_with`]).
pub fn show_findbar(
    ui: &mut egui::Ui,
    state: &mut FindBarState,
    has_prev: bool,
    has_next: bool,
    i18n: &I18n,
) -> Vec<FindAction> {
    let no_list = HintListUi::default().unavailable();
    show_findbar_with(ui, state, (has_prev, has_next), &no_list, i18n)
}

/// The find bar with the collection's index names for the Query options' hint picker (`hints`):
/// the actions of this frame, the request for the list first when the open section lacks one.
pub fn show_findbar_with(
    ui: &mut egui::Ui,
    state: &mut FindBarState,
    (has_prev, has_next): (bool, bool),
    hints: &HintListUi,
    i18n: &I18n,
) -> Vec<FindAction> {
    *state = state.more_settled().options_settled();
    let more = MoreToggle {
        open: more_open(state),
        hidden: more_set_count(state),
    };
    let blocked = find_blocked(state, i18n);
    let options_off = state
        .options_blocked()
        .then(|| i18n.t("query.options.blocked"));
    let row = query_row(
        ui,
        &mut state.text.filter,
        (has_prev, has_next),
        (more, blocked.as_deref(), options_off.as_deref()),
        i18n,
    );
    let (more_out, section) = match more.open {
        true => {
            let toggle = (options_open(state), options_set_count(&state.options));
            let more_out = show_more_fields(ui, &mut state.text, toggle, i18n);
            (more_out, show_options_section(ui, state, hints, i18n))
        }
        false => (MoreOutcome::default(), SectionOutcome::default()),
    };
    show_error_line(ui, state, options_off.as_deref(), i18n);
    let enter = row.enter || more_out.enter || section.enter;
    let mut actions: Vec<FindAction> = section
        .wants_hints
        .then_some(FindAction::HintListWanted)
        .into_iter()
        .chain(section.retry_hints.then_some(FindAction::HintListRetry))
        .collect();
    apply_section(state, section, row.toggled, more_out.toggled);
    if let Some(intent) = pick_intent(row.button, enter) {
        let (next, action) = resolve_find_intent(state, intent, has_prev, has_next);
        *state = next;
        actions.extend(action);
    }
    actions
}

/// The edits of the Query options form, Reset, and the two disclosure clicks, applied in order.
fn apply_section(state: &mut FindBarState, section: SectionOutcome, more: bool, options: bool) {
    let mut next = std::mem::take(state);
    for edit in section.edits {
        next = next.options_edited(edit);
    }
    if section.reset {
        next = next.options_reset();
    }
    if more {
        next = next.more_toggled();
    }
    if options {
        next = next.options_toggled();
    }
    *state = next;
}

/// The last error of the bar (the options' own timeout said as the person's limit), or why Find
/// is off because of the options.
fn show_error_line(
    ui: &mut egui::Ui,
    state: &FindBarState,
    options_off: Option<&str>,
    i18n: &I18n,
) {
    if let Some(e) = &state.last_error {
        match (state.error_from_options, e) {
            (true, CoreError::Database(text)) if text == COMMAND_TIMEOUT_MESSAGE => {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    i18n.t("query.options.timeout_set"),
                );
            }
            _ => super::error_label::show_error(ui, e, i18n),
        }
    }
    if let Some(reason) = options_off {
        ui.colored_label(ui.visuals().error_fg_color, reason);
    }
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
    let response = described(ui, response, i18n.t("query.verdict.automatic_hint"));
    response
        .changed()
        .then_some(FindAction::SetAutomaticVerdict(checked))
}

/// The global "warn before full scans" box; the tooltip and the accessible description say what
/// the check does and costs. Same shape as [`show_automatic_verdict`].
pub fn show_guard_setting(ui: &mut egui::Ui, guard: bool, i18n: &I18n) -> Option<FindAction> {
    let mut checked = guard;
    let response = ui.checkbox(&mut checked, i18n.t("query.guard.setting"));
    let response = described(ui, response, i18n.t("query.guard.setting_hint"));
    response.changed().then_some(FindAction::SetGuard(checked))
}

/// `text` as the hover text and the accessible description of `response`: the label is kept
/// short, so the full sentence is what a screen reader and a hover give.
pub(super) fn described(ui: &egui::Ui, response: egui::Response, text: String) -> egui::Response {
    let description = text.clone();
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_description(description));
    response.on_hover_text(text)
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
    (more, code_blocked, find_off): (MoreToggle, Option<&str>, Option<&str>),
    i18n: &I18n,
) -> RowOutcome {
    let memory = ui.id().with("findbar_trailing_width");
    let trailing = ui.data(|d| d.get_temp::<f32>(memory));
    // The Filter's label sticks out above its box; the row is as tall as the box plus that
    // overhang mirrored below, so the box is centred in the row, and so are the buttons, which
    // then line up with the box.
    let row_height = field_row_height(ui);
    ui.horizontal_wrapped(|ui| {
        ui.set_row_height(row_height);
        let width = filter_width(ui.available_width(), trailing.unwrap_or(TRAILING_GUESS));
        let enter = field_cell(ui, width, row_height, |ui| {
            sized_field(ui, width, KEY_FILTER, filter, i18n)
        });
        let (button, buttons_width) =
            buttons(ui, (has_prev, has_next), (code_blocked, find_off), i18n);
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

/// Find; off with the reason as its tooltip and accessible description while the options have a
/// problem.
fn find_button(ui: &mut egui::Ui, off: Option<&str>, i18n: &I18n) -> egui::Response {
    let label = i18n.t("findbar.run");
    let Some(reason) = off else {
        return filled_button(ui, label);
    };
    let response = gate(ui, false, |ui| filled_button(ui, label)).on_hover_text(reason);
    let why = reason.to_string();
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_description(why));
    response
}

/// Find, Clear, the `</>` Copy as code icon (off with the parse error as its reason), Previous
/// and Next in the current row; the intent clicked and the width they took with the gap before each.
fn buttons(
    ui: &mut egui::Ui,
    (has_prev, has_next): (bool, bool),
    (code_blocked, find_off): (Option<&str>, Option<&str>),
    i18n: &I18n,
) -> (Option<FindIntent>, f32) {
    let gap = ui.spacing().item_spacing.x;
    let mut intent = None;
    let mut width = 0.0;
    let run = find_button(ui, find_off, i18n);
    let clear = tonal_button(ui, i18n.t("findbar.clear"));
    let code = show_copy_code_icon(ui, code_blocked, i18n);
    let prev = gate(ui, has_prev, |ui| text_button(ui, i18n.t("findbar.prev")));
    let next = gate(ui, has_next, |ui| text_button(ui, i18n.t("findbar.next")));
    for (response, clicked) in [
        (run, FindIntent::RunButton),
        (clear, FindIntent::ClearButton),
        (code, FindIntent::CodeButton),
        (prev, FindIntent::PrevButton),
        (next, FindIntent::NextButton),
    ] {
        width += response.rect.width() + gap;
        let usable = match clicked {
            FindIntent::CodeButton => code_blocked.is_none(),
            FindIntent::RunButton => find_off.is_none(),
            _ => true,
        };
        if response.clicked() && usable {
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

    fn run_frame(state: &mut FindBarState, prev: bool, next: bool) -> Vec<FindAction> {
        let ctx = egui::Context::default();
        let i18n = I18n::english();
        let mut out = Vec::new();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                out = show_findbar(ui, state, prev, next, &i18n);
            });
        });
        out
    }

    #[test]
    fn renders_default_and_error_state_without_panicking() {
        assert_eq!(
            run_frame(&mut FindBarState::default(), false, false),
            Vec::new()
        );
        let mut s = FindBarState {
            text: QueryText {
                filter: "{ \"имя\": \"a b.c\" }".into(),
                limit: "10".into(),
                ..Default::default()
            },
            last_error: Some(CoreError::InvalidJson("filter: x".into())),
            more: Some(true),
            ..Default::default()
        };
        let before = s.clone();
        assert_eq!(run_frame(&mut s, true, true), Vec::new());
        assert_eq!(s, before, "an idle frame must not change the state");
    }

    #[test]
    fn more_opened_by_a_value_stays_open_when_the_last_value_is_deleted() {
        let mut s = FindBarState {
            text: QueryText {
                limit: "10".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        run_frame(&mut s, false, false);
        assert_eq!(s.more, Some(true), "shown open, so the opening sticks");
        s.text.limit.clear();
        run_frame(&mut s, false, false);
        assert!(more_open(&s), "deleting the last value does not close it");
        let mut empty = FindBarState::default();
        run_frame(&mut empty, false, false);
        assert_eq!(empty.more, None, "nothing shown open, nothing decided");
    }
}
