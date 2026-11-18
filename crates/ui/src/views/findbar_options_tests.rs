//! The Query options section of the find bar by accessible name and role: the toggle and its
//! count, the section under the More row, what typing and Enter do, a problem under its field with
//! Find off for it, the hint picker and the bounded height.
use super::*;
use crate::I18n;
use crate::theme;
use crate::viewmodel::aggregate_hint_list::HintListUi;
use crate::viewmodel::aggregate_options::OptionsEdit;
use crate::viewmodel::{FindAction, FindBarState};
use crate::views::findbar::show_findbar_with;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::COMMAND_TIMEOUT_MESSAGE;
use eframe::egui;
use eframe::egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

struct Frame {
    state: FindBarState,
    hints: HintListUi,
    seen: Vec<FindAction>,
    /// The y of a label drawn right after the bar: where the page below would start.
    below: f32,
}

const WIDE: (f32, f32) = (1000.0, 700.0);
const SMALL: (f32, f32) = (800.0, 500.0);

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn open(state: FindBarState, hints: HintListUi, size: (f32, f32)) -> Harness<'static, Frame> {
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .build_ui_state(
            move |ui, frame: &mut Frame| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                let actions = show_findbar_with(
                    ui,
                    &mut frame.state,
                    (false, false),
                    &frame.hints,
                    &I18n::english(),
                );
                frame.seen.extend(actions);
                frame.below = ui.label("below").rect.top();
            },
            Frame {
                state,
                hints,
                seen: Vec::new(),
                below: 0.0,
            },
        );
    harness.run_steps(4);
    harness
}

fn more_open() -> FindBarState {
    FindBarState {
        more: Some(true),
        ..FindBarState::default()
    }
}

fn with_options(edits: Vec<OptionsEdit>) -> FindBarState {
    edits
        .into_iter()
        .fold(more_open(), |state, edit| state.options_edited(edit))
}

fn shown(state: FindBarState) -> FindBarState {
    FindBarState {
        options_open: Some(true),
        ..state
    }
}

fn loaded(names: &[&str]) -> HintListUi {
    HintListUi::default().from_names(names.iter().map(|n| n.to_string()).collect())
}

fn asked() -> HintListUi {
    HintListUi::default().requested().0
}

#[test]
fn the_toggle_reads_query_options_then_the_count() {
    let harness = open(more_open(), asked(), WIDE);
    harness.get_by_role_and_label(Role::Button, &t("query.options.toggle"));
    let one = with_options(vec![OptionsEdit::Comment("why".into())]);
    let harness = open(one, asked(), WIDE);
    let label = I18n::english().t_with("query.options.toggle_count", &[("n", "1")]);
    harness.get_by_role_and_label(Role::Button, &label);
}

#[test]
fn the_toggle_reports_its_expanded_state() {
    let label = t("query.options.toggle");
    let closed = open(more_open(), asked(), WIDE);
    let node = closed.get_by_role_and_label(Role::Button, &label);
    assert_eq!(node.accesskit_node().toggled(), Some(Toggled::False));
    let opened = open(shown(more_open()), asked(), WIDE);
    let node = opened.get_by_role_and_label(Role::Button, &label);
    assert_eq!(node.accesskit_node().toggled(), Some(Toggled::True));
}

#[test]
fn clicking_the_toggle_opens_the_section() {
    let mut harness = open(more_open(), asked(), WIDE);
    harness
        .get_by_role_and_label(Role::Button, &t("query.options.toggle"))
        .click();
    harness.run_steps(3);
    assert_eq!(harness.state().state.options_open, Some(true));
    harness.get_by_role_and_label(Role::CheckBox, &t("aggregate.options.collation.enabled"));
}

#[test]
fn a_closed_section_draws_nothing_and_asks_for_nothing() {
    let harness = open(more_open(), HintListUi::default(), WIDE);
    assert!(
        harness
            .query_by_label(&t("aggregate.options.allow_disk_use.label"))
            .is_none()
    );
    assert!(harness.state().seen.is_empty());
}

#[test]
fn the_section_sits_under_the_more_row() {
    let harness = open(shown(more_open()), asked(), WIDE);
    let y = |role, key: &str| {
        harness
            .get_by_role_and_label(role, &t(key))
            .accesskit_node()
            .raw_bounds()
            .expect("laid out")
            .y0
    };
    let limit = y(Role::TextInput, "findbar.limit");
    let collation = y(Role::CheckBox, "aggregate.options.collation.enabled");
    assert!(collation > limit, "{collation} under {limit}");
}

#[test]
fn typing_a_locale_edits_the_form_and_sends_nothing() {
    let state = shown(more_open()).options_edited(OptionsEdit::CollationOn(true));
    let mut harness = open(state, asked(), WIDE);
    let label = t("aggregate.options.collation.locale");
    harness
        .get_by_role_and_label(Role::TextInput, &label)
        .click();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, &label)
        .type_text("fr");
    harness.run_steps(3);
    assert_eq!(harness.state().state.options.locale, "fr");
    assert!(
        harness
            .state()
            .seen
            .iter()
            .all(|a| !matches!(a, FindAction::Run(_))),
        "nothing runs on an edit"
    );
}

#[test]
fn enter_in_an_options_field_finds() {
    let mut harness = open(shown(more_open()), asked(), WIDE);
    let label = t("aggregate.options.comment.label");
    harness
        .get_by_role_and_label(Role::TextInput, &label)
        .click();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, &label)
        .type_text("why");
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    let ran = harness
        .state()
        .seen
        .iter()
        .find_map(|a| match a {
            FindAction::Run(q) => Some(q.clone()),
            _ => None,
        })
        .expect("Enter runs the query");
    assert_eq!(ran.options.comment.as_deref(), Some("why"));
}

#[test]
fn a_problem_shows_under_its_field_and_find_is_off_with_the_reason() {
    let state = shown(with_options(vec![OptionsEdit::Comment("   ".into())]));
    let harness = open(state, asked(), WIDE);
    let comment = t("aggregate.options.comment.label");
    let field = harness.get_by_role_and_label(Role::TextInput, &comment);
    let said = field.accesskit_node().description().unwrap_or_default();
    assert!(said.contains("The comment is only spaces"), "{said}");
    let run = t("findbar.run");
    let find = harness.get_by_role_and_label(Role::Button, &run);
    let reason = find.accesskit_node().description().unwrap_or_default();
    assert_eq!(reason, t("query.options.blocked"));
    harness.get_by_label(&t("query.options.blocked"));
}

#[test]
fn the_hint_combo_lists_the_indexes_and_picking_sets_the_hint() {
    let mut harness = open(shown(more_open()), loaded(&["a_1", "status_1"]), WIDE);
    harness
        .get_by_role_and_label(Role::ComboBox, &t("aggregate.options.hint.existing"))
        .click();
    harness.run_steps(3);
    harness.get_by_label("status_1").click();
    harness.run_steps(3);
    assert_eq!(harness.state().state.options.hint_text, "status_1");
}

#[test]
fn the_open_section_asks_for_the_hint_list_once() {
    let harness = open(shown(more_open()), HintListUi::default(), WIDE);
    assert!(harness.state().seen.contains(&FindAction::HintListWanted));
    let answered = open(shown(more_open()), asked(), WIDE);
    assert!(!answered.state().seen.contains(&FindAction::HintListWanted));
}

#[test]
fn a_failed_hint_list_has_a_retry_that_asks_again() {
    let (waiting, request) = HintListUi::default().requested();
    let mut harness = open(shown(more_open()), waiting.failed(request), WIDE);
    harness
        .get_by_role_and_label(Role::Button, &t("common.retry"))
        .click();
    harness.run_steps(2);
    assert!(harness.state().seen.contains(&FindAction::HintListRetry));
}

#[test]
fn a_time_limit_above_thirty_seconds_says_finds_stop_there() {
    let long = shown(with_options(vec![OptionsEdit::MaxTime("45000".into())]));
    let harness = open(long, asked(), WIDE);
    harness.get_by_label_contains("Finds still stop at 30 s");
    let short = shown(with_options(vec![OptionsEdit::MaxTime("5000".into())]));
    let harness = open(short, asked(), WIDE);
    assert!(harness.query_by_label_contains("still stop at").is_none());
}

#[test]
fn the_comment_note_says_it_reaches_the_server() {
    let harness = open(shown(more_open()), asked(), WIDE);
    harness.get_by_label(&t("aggregate.options.comment_sent_note"));
    harness.get_by_label(&t("query.options.disk_use_note"));
}

#[test]
fn reset_empties_the_form() {
    let state = shown(with_options(vec![OptionsEdit::Comment("why".into())]));
    let mut harness = open(state, asked(), (1000.0, 1800.0));
    let reset = t("aggregate.options.reset");
    harness.get_by_role_and_label(Role::Button, &reset).click();
    harness.run_steps(3);
    assert_eq!(harness.state().state.options.comment, "");
    assert_eq!(
        harness.state().state.options_open,
        Some(true),
        "the section stays open"
    );
}

/// What lies between the bottom of the Limit field and the line after the bar.
fn section_extent(harness: &Harness<'static, Frame>) -> f32 {
    let limit = t("findbar.limit");
    let bottom = harness
        .get_by_role_and_label(Role::TextInput, &limit)
        .accesskit_node()
        .raw_bounds()
        .expect("laid out")
        .y1 as f32;
    harness.state().below - bottom
}

#[test]
fn the_section_scrolls_within_its_share_of_the_window() {
    // The frame, its margins and the reason line around the scrolling part, and the room the
    // floating labels' overhang gives the Limit field's cell under its box.
    const AROUND: f32 = 72.0;
    for size in [SMALL, WIDE, (1000.0, 1400.0)] {
        let state = shown(with_options(vec![OptionsEdit::CollationOn(true)]));
        let harness = open(state, asked(), size);
        let share = (size.1 * OPTIONS_MAX_HEIGHT_SHARE).max(OPTIONS_MIN_HEIGHT);
        let extent = section_extent(&harness);
        assert!(
            extent <= share + AROUND,
            "{extent} under the Limit field in a window of {size:?}"
        );
    }
    // Closed, nothing of it is left.
    let closed = open(more_open(), asked(), SMALL);
    assert!(section_extent(&closed) < 24.0);
}

#[test]
fn a_lowered_timeout_is_worded_as_yours() {
    let state = FindBarState {
        last_error: Some(CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into())),
        error_from_options: true,
        ..shown(more_open())
    };
    let harness = open(state, asked(), WIDE);
    harness.get_by_label_contains("time limit you set");
    let plain = FindBarState {
        last_error: Some(CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into())),
        ..more_open()
    };
    let harness = open(plain, asked(), WIDE);
    assert!(
        harness
            .query_by_label_contains("time limit you set")
            .is_none()
    );
}

#[test]
fn a_hint_not_found_error_is_said_in_the_bar() {
    let state = FindBarState {
        last_error: Some(CoreError::HintNotFound),
        error_from_options: true,
        ..shown(more_open())
    };
    let harness = open(state, asked(), WIDE);
    harness.get_by_label(&t("errors.hint_not_found"));
}
