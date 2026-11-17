//! The Options section by accessible name and role, on real drafts: the toggle and its count,
//! the controls and their problems in words, the hint picker's states, the layout at two widths
//! and when typed text is applied.
use super::super::aggregate::{flush_typing, show_aggregate, with_due_options};
use crate::I18n;
use crate::app::{AggregateAction, OptionsAction};
use crate::theme;
use crate::viewmodel::aggregate::AggregateState;
use crate::viewmodel::aggregate_hint_list::HintListUi;
use crate::viewmodel::aggregate_options::{OptionsEdit, Tri};
use crate::viewmodel::aggregate_options_state::OptionsUi;
use crate::viewmodel::aggregate_view::{AggregateView, EDIT_PAUSE_SECS};
use crate::views::aggregate_run::RunContext;
use eframe::egui;
use eframe::egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

pub(super) struct Frame {
    pub draft: AggregateState,
    pub view: AggregateView,
    pub seen: Vec<AggregateAction>,
}

pub(super) fn t(key: &str) -> String {
    I18n::english().t(key)
}

pub(super) fn open(draft: AggregateState, size: (f32, f32)) -> Harness<'static, Frame> {
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .build_ui_state(
            move |ui, frame: &mut Frame| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                let actions = show_aggregate(
                    ui,
                    &frame.draft,
                    &mut frame.view,
                    &[],
                    &RunContext::default(),
                    &I18n::english(),
                );
                frame.seen.extend(actions);
            },
            Frame {
                draft,
                view: AggregateView::default(),
                seen: Vec::new(),
            },
        );
    harness.run_steps(4);
    harness
}

/// One `$match` stage, previewed (so the tab asks for no automatic refresh).
pub(super) fn base() -> AggregateState {
    let (draft, sent) = AggregateState::default()
        .add_stage("$match")
        .begin_preview();
    let (request, _) = sent.expect("a valid pipeline is sent");
    draft.arrived(request, Ok(Vec::new()))
}

fn applied(draft: AggregateState, edit: OptionsEdit) -> AggregateState {
    draft.options_edited(edit).options_applied().0
}

fn shown(draft: AggregateState) -> AggregateState {
    draft.options_toggled()
}

const WIDE: (f32, f32) = (1000.0, 700.0);
const NARROW: (f32, f32) = (800.0, 500.0);

fn options(a: OptionsAction) -> AggregateAction {
    AggregateAction::Options(a)
}

#[test]
fn the_toggle_reads_options_then_options_with_the_count() {
    let harness = open(base(), WIDE);
    harness.get_by_role_and_label(Role::Button, &t("aggregate.options.toggle"));
    let one = applied(base(), OptionsEdit::AllowDiskUse(Tri::On));
    let harness = open(one, WIDE);
    let label = I18n::english().t_with("aggregate.options.toggle_count", &[("n", "1")]);
    harness.get_by_role_and_label(Role::Button, &label);
}

#[test]
fn the_toggle_reports_its_expanded_state() {
    let label = t("aggregate.options.toggle");
    let closed = open(base(), WIDE);
    let node = closed.get_by_role_and_label(Role::Button, &label);
    assert_eq!(node.accesskit_node().toggled(), Some(Toggled::False));
    let opened = open(shown(base()), WIDE);
    let node = opened.get_by_role_and_label(Role::Button, &label);
    assert_eq!(node.accesskit_node().toggled(), Some(Toggled::True));
}

#[test]
fn clicking_the_toggle_asks_to_open_it() {
    let mut harness = open(base(), WIDE);
    harness
        .get_by_role_and_label(Role::Button, &t("aggregate.options.toggle"))
        .click();
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .seen
            .contains(&options(OptionsAction::Toggle))
    );
}

#[test]
fn a_closed_section_draws_nothing() {
    let harness = open(base(), WIDE);
    let label = t("aggregate.options.allow_disk_use.label");
    assert!(harness.query_by_label(&label).is_none());
    assert!(
        !harness
            .state()
            .seen
            .contains(&options(OptionsAction::HintListWanted))
    );
}

#[test]
fn the_open_section_asks_for_the_hint_list() {
    let harness = open(shown(base()), WIDE);
    assert!(
        harness
            .state()
            .seen
            .contains(&options(OptionsAction::HintListWanted))
    );
}

#[test]
fn a_problem_shows_under_its_field_in_words() {
    let draft = shown(base()).options_edited(OptionsEdit::Comment("   ".into()));
    let harness = open(draft, WIDE);
    let label = t("aggregate.options.comment.label");
    let field = harness.get_by_role_and_label(Role::TextInput, &label);
    let said = field.accesskit_node().description().unwrap_or_default();
    assert!(said.contains("The comment is only spaces"), "{said}");
    let harness = open(shown(base()), WIDE);
    let field = harness.get_by_role_and_label(Role::TextInput, &label);
    assert_eq!(field.accesskit_node().description(), None);
}

#[test]
fn allow_disk_use_says_explain_and_run_only() {
    let harness = open(shown(base()), WIDE);
    harness.get_by_label(&t("aggregate.options.allow_disk_use.preview_note"));
    harness.get_by_label(&t("aggregate.options.comment_sent_note"));
}

#[test]
fn a_time_limit_above_ten_seconds_says_previews_stop_there() {
    let long = applied(shown(base()), OptionsEdit::MaxTime("30000".into()));
    let harness = open(long, WIDE);
    harness.get_by_label_contains("still stop at 10 s");
    let short = applied(shown(base()), OptionsEdit::MaxTime("5000".into()));
    let harness = open(short, WIDE);
    assert!(harness.query_by_label_contains("still stop at").is_none());
}

fn with_names(draft: AggregateState, hint: HintListUi) -> AggregateState {
    let mut draft = draft;
    draft.options_ui = OptionsUi {
        hint,
        ..draft.options_ui.clone()
    };
    draft
}

fn loaded(names: &[&str]) -> HintListUi {
    HintListUi::default().from_names(names.iter().map(|n| n.to_string()).collect())
}

#[test]
fn the_hint_combo_lists_the_indexes_and_picking_sets_the_hint() {
    let draft = with_names(shown(base()), loaded(&["a_1", "status_1"]));
    let mut harness = open(draft, WIDE);
    harness
        .get_by_role_and_label(Role::ComboBox, &t("aggregate.options.hint.existing"))
        .click();
    harness.run_steps(3);
    harness.get_by_label("status_1").click();
    harness.run_steps(3);
    assert!(
        harness
            .state()
            .seen
            .contains(&options(OptionsAction::Edit(OptionsEdit::HintPicked(
                "status_1".into()
            ))))
    );
}

#[test]
fn the_hint_list_says_loading_and_could_not_list() {
    let (loading, _) = HintListUi::default().requested();
    let harness = open(with_names(shown(base()), loading), WIDE);
    harness.get_by_label(&t("aggregate.options.hint.loading"));
    let (waiting, request) = HintListUi::default().requested();
    let failed = waiting.failed(request);
    let harness = open(with_names(shown(base()), failed), WIDE);
    harness.get_by_label(&t("aggregate.options.hint.list_failed"));
}

#[test]
fn a_name_not_among_the_listed_indexes_is_noted() {
    let typed = shown(base()).options_edited(OptionsEdit::HintText("nope_1".into()));
    let harness = open(with_names(typed, loaded(&["a_1"])), WIDE);
    harness.get_by_label(&t("aggregate.options.hint.not_listed"));
    let known = shown(base()).options_edited(OptionsEdit::HintText("a_1".into()));
    let harness = open(with_names(known, loaded(&["a_1"])), WIDE);
    assert!(
        harness
            .query_by_label(&t("aggregate.options.hint.not_listed"))
            .is_none()
    );
}

#[test]
fn reset_emits_reset() {
    let mut harness = open(shown(base()), (1000.0, 1800.0));
    let label = t("aggregate.options.reset");
    harness.get_by_role_and_label(Role::Button, &label).click();
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .seen
            .contains(&options(OptionsAction::Reset))
    );
}

#[test]
fn typing_a_comment_emits_an_edit_and_enter_applies_it_at_once() {
    let mut harness = open(shown(base()), WIDE);
    let label = t("aggregate.options.comment.label");
    harness
        .get_by_role_and_label(Role::TextInput, &label)
        .click();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, &label)
        .type_text("hi");
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .seen
            .contains(&options(OptionsAction::Edit(OptionsEdit::Comment(
                "hi".into()
            ))))
    );
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .seen
            .contains(&options(OptionsAction::Apply))
    );
}

fn typing() -> AggregateAction {
    options(OptionsAction::Edit(OptionsEdit::Comment("x".into())))
}

#[test]
fn typing_a_locale_emits_an_edit_and_apply_after_the_pause() {
    let mut view = AggregateView::default();
    let (actions, left) = with_due_options(&mut view, 1.0, vec![typing()]);
    assert_eq!(actions, [typing()], "nothing applies while typing");
    assert!(left.is_some());
    let (actions, left) = with_due_options(&mut view, 1.0 + EDIT_PAUSE_SECS, Vec::new());
    assert_eq!(actions, [options(OptionsAction::Apply)]);
    assert_eq!(left, None);
}

#[test]
fn enter_applies_the_typed_text_at_once() {
    let mut view = AggregateView::default();
    let (actions, left) = with_due_options(
        &mut view,
        1.0,
        vec![typing(), options(OptionsAction::Apply)],
    );
    assert_eq!(actions, [typing(), options(OptionsAction::Apply)]);
    assert_eq!(left, None, "nothing waits after a commit");
}

#[test]
fn another_action_in_the_frame_applies_the_typing_first() {
    let mut view = AggregateView::default();
    let _ = with_due_options(&mut view, 1.0, vec![typing()]);
    let select = AggregateAction::Select(0);
    let (actions, _) = with_due_options(&mut view, 1.1, vec![select.clone()]);
    assert_eq!(actions, [options(OptionsAction::Apply), select]);
}

#[test]
fn leaving_the_tab_flushes_the_typed_options() {
    let mut view = AggregateView::default().options_typed(1.0);
    assert_eq!(flush_typing(&mut view), Some(options(OptionsAction::Apply)));
    assert_eq!(flush_typing(&mut view), None);
    let (view, _) = AggregateView::default().edited(1, "x".into(), 1.0);
    let mut both = view.options_typed(1.0);
    assert_eq!(
        flush_typing(&mut both),
        Some(AggregateAction::Batch(vec![
            AggregateAction::Edit(1, "x".into()),
            options(OptionsAction::Apply)
        ]))
    );
}

#[test]
fn the_section_has_two_columns_when_wide_and_one_when_narrow() {
    let x_of = |harness: &Harness<'static, Frame>, role: Role, label: &str| {
        harness
            .get_by_role_and_label(role, label)
            .accesskit_node()
            .raw_bounds()
            .map(|b| (b.x0, b.y0))
            .expect("bounds")
    };
    let collation = t("aggregate.options.collation.enabled");
    let hint = t("aggregate.options.hint.existing");
    let draft = || with_names(shown(base()), loaded(&["a_1"]));
    let wide = open(draft(), WIDE);
    let narrow = open(draft(), NARROW);
    let (cw, hw) = (
        x_of(&wide, Role::CheckBox, &collation),
        x_of(&wide, Role::ComboBox, &hint),
    );
    assert!(hw.0 > cw.0 + 100.0, "side by side: {cw:?} {hw:?}");
    let (cn, hn) = (
        x_of(&narrow, Role::CheckBox, &collation),
        x_of(&narrow, Role::ComboBox, &hint),
    );
    assert!(
        (hn.0 - cn.0).abs() < 60.0 && hn.1 > cn.1,
        "stacked: {cn:?} {hn:?}"
    );
}
