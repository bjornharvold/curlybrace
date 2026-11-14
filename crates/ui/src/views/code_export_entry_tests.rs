//! The two Copy as code entry points by accessible name: on for a valid pipeline or query, off
//! with the reason as its description otherwise, and never needing a connection.
use super::*;
use crate::app::AggregateAction;
use crate::theme;
use crate::viewmodel::aggregate_view::{AggregateView, PendingEdit};
use crate::viewmodel::{FindAction, FindBarState};
use crate::views::aggregate::show_aggregate;
use crate::views::aggregate_run::RunContext;
use crate::views::findbar::show_findbar;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const SIZE: (f32, f32) = (1000.0, 700.0);

fn opens_aggregate_code() -> AggregateAction {
    AggregateAction::Code(crate::app::CodeAction::Open(
        crate::app::CodeSource::Aggregate,
    ))
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn draft(stages: &[(&str, &str)]) -> AggregateState {
    stages
        .iter()
        .fold(AggregateState::default(), |draft, (name, text)| {
            let at = draft.stages.len();
            draft.add_stage(name).edit(at, text)
        })
}

struct Tab {
    draft: AggregateState,
    view: AggregateView,
    seen: Vec<AggregateAction>,
}

fn tab(draft: AggregateState) -> Harness<'static, Tab> {
    let mut installed = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(SIZE.0, SIZE.1))
        .build_ui_state(
            move |ui, tab: &mut Tab| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                let run = RunContext::default();
                let actions =
                    show_aggregate(ui, &tab.draft, &mut tab.view, &[], &run, &I18n::english());
                tab.seen.extend(actions);
            },
            Tab {
                draft,
                view: AggregateView::default(),
                seen: Vec::new(),
            },
        );
    h.run_steps(4);
    h
}

struct Bar {
    state: FindBarState,
    seen: Vec<FindAction>,
}

fn bar(filter: &str) -> Harness<'static, Bar> {
    let mut installed = false;
    let state = FindBarState {
        text: QueryText {
            filter: filter.into(),
            ..QueryText::default()
        },
        ..FindBarState::default()
    };
    let mut h = Harness::builder()
        .with_size(egui::vec2(SIZE.0, SIZE.1))
        .build_ui_state(
            move |ui, bar: &mut Bar| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                let action = show_findbar(ui, &mut bar.state, false, false, &I18n::english());
                bar.seen.extend(action);
            },
            Bar {
                state,
                seen: Vec::new(),
            },
        );
    h.run_steps(4);
    h
}

#[test]
fn the_aggregate_button_is_on_for_valid_stages_without_a_connection() {
    let mut h = tab(draft(&[("$match", r#"{ "$match": {} }"#)]));
    let label = t("codegen.button");
    let button = h.get_by_role_and_label(Role::Button, &label);
    assert!(!button.accesskit_node().is_disabled());
    button.click();
    h.run_steps(3);
    assert!(h.state().seen.contains(&opens_aggregate_code()));
}

#[test]
fn the_aggregate_button_is_off_with_the_pipeline_error_as_its_reason() {
    let cases = [
        (AggregateState::default(), t("aggregate.error.empty")),
        (
            draft(&[("$match", "{ nope")]),
            I18n::english().t_with("aggregate.error.not_a_stage", &[("n", "1")]),
        ),
        (
            draft(&[("$match", r#"{ "$match": {} }"#)]).toggle(0),
            t("aggregate.error.empty"),
        ),
        (
            draft(&[("$match", r#"{ "$match": {} }"#)])
                .to_text()
                .with_text("[]"),
            t("codegen.disabled.text_not_applied"),
        ),
    ];
    let label = t("codegen.button");
    for (state, reason) in cases {
        let mut h = tab(state);
        let button = h.get_by_role_and_label(Role::Button, &label);
        assert!(button.accesskit_node().is_disabled(), "{reason}");
        assert_eq!(button.accesskit_node().description(), Some(reason));
        button.click();
        h.run_steps(2);
        assert!(!h.state().seen.contains(&opens_aggregate_code()));
    }
}

/// Types `text` into stage 0 and leaves it waiting (far from due), as a keystroke does.
fn typing(h: &mut Harness<'static, Tab>, text: &str) {
    h.state_mut().view.pending = Some(PendingEdit {
        index: 0,
        text: text.to_string(),
        at: 1.0e9,
    });
    h.run_steps(3);
}

#[test]
fn an_invalid_edit_still_waiting_turns_the_button_off_with_its_reason() {
    let mut h = tab(draft(&[("$match", r#"{ "$match": {} }"#)]));
    typing(&mut h, "{ nope");
    let label = t("codegen.button");
    let button = h.get_by_role_and_label(Role::Button, &label);
    let reason = I18n::english().t_with("aggregate.error.not_a_stage", &[("n", "1")]);
    assert!(button.accesskit_node().is_disabled());
    assert_eq!(button.accesskit_node().description(), Some(reason));
    button.click();
    h.run_steps(2);
    assert!(!h.state().seen.contains(&opens_aggregate_code()));
}

#[test]
fn a_valid_edit_still_waiting_turns_the_button_on() {
    let mut h = tab(draft(&[("$match", "{ nope")]));
    typing(&mut h, r#"{ "$match": {} }"#);
    let label = t("codegen.button");
    let button = h.get_by_role_and_label(Role::Button, &label);
    assert!(!button.accesskit_node().is_disabled());
    assert!(button.accesskit_node().description().is_none());
}

#[test]
fn the_query_bar_button_is_on_for_a_valid_query_and_asks_to_open() {
    let mut h = bar(r#"{ "a": 1 }"#);
    let label = t("codegen.button");
    let button = h.get_by_role_and_label(Role::Button, &label);
    assert!(!button.accesskit_node().is_disabled());
    button.click();
    h.run_steps(3);
    assert_eq!(h.state().seen, [FindAction::CopyAsCode]);
}

#[test]
fn the_query_bar_button_is_off_with_the_parse_error_as_its_reason() {
    let mut h = bar("{ not json");
    let label = t("codegen.button");
    let button = h.get_by_role_and_label(Role::Button, &label);
    assert!(button.accesskit_node().is_disabled());
    let reason = find_blocked(&h.state().state.text, &I18n::english()).expect("does not parse");
    assert_eq!(button.accesskit_node().description(), Some(reason));
    button.click();
    h.run_steps(2);
    assert!(h.state().seen.is_empty());
}

#[test]
fn it_is_an_icon_between_clear_and_previous_on_the_find_row() {
    let h = bar("{}");
    let rect = |key: &str| {
        let label = t(key);
        h.get_by_role_and_label(Role::Button, &label).rect()
    };
    let (find, clear, code, prev, next) = (
        rect("findbar.run"),
        rect("findbar.clear"),
        rect("codegen.button"),
        rect("findbar.prev"),
        rect("findbar.next"),
    );
    assert!(find.left() < clear.left() && clear.left() < code.left());
    assert!(code.left() < prev.left() && prev.left() < next.left());
    for other in [clear, prev, next] {
        assert!((code.center().y - other.center().y).abs() < 1.0, "one row");
    }
    assert!(
        code.width() < 60.0,
        "an icon, not a labelled button: {code:?}"
    );
}

#[test]
fn the_find_row_stays_on_one_line_with_the_icon() {
    let h = bar("{}");
    let filter = h.get_by_label(&t("findbar.filter")).rect();
    let code = h
        .get_by_role_and_label(Role::Button, &t("codegen.button"))
        .rect();
    let find = h
        .get_by_role_and_label(Role::Button, &t("findbar.run"))
        .rect();
    assert!(
        (find.center().y - code.center().y).abs() < 1.0 && filter.bottom() > find.top(),
        "the buttons share the Filter's row: {filter:?} {find:?}"
    );
}

#[test]
fn the_icon_is_named_in_words_and_described_by_its_hint() {
    let h = bar("{}");
    let label = t("codegen.button");
    let button = h.get_by_role_and_label(Role::Button, &label);
    assert_eq!(button.accesskit_node().label(), Some(label.clone()));
    assert_eq!(
        button.accesskit_node().description(),
        Some(t("codegen.button_hint"))
    );
    assert!(h.query_by_label(&t("codegen.button_icon")).is_none());
}
