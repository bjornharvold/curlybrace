//! The shared options form by accessible name and role: both layouts, the collation flags that
//! appear with the collation, two forms in one frame that never share a widget, and Enter told
//! apart from a focus loss.
use super::*;
use crate::I18n;
use crate::theme;
use crate::viewmodel::aggregate_hint_list::HintListUi;
use crate::viewmodel::aggregate_options::{OptionsEdit, OptionsForm};
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn cx(id_scope: &'static str, layout: FormLayout, flags: FlagsWhenOff) -> FormContext<'static> {
    FormContext {
        id_scope,
        layout,
        disk_use_note: "aggregate.options.allow_disk_use.preview_note",
        comment_placeholder: "aggregate.options.comment.placeholder",
        max_time_placeholder: (
            "aggregate.options.max_time.placeholder",
            vec![("preview", "10".to_string()), ("run", "30".to_string())],
        ),
        max_time_clamped: None,
        flags_when_off: flags,
    }
}

struct Frame {
    forms: Vec<(FormContext<'static>, OptionsForm)>,
    hints: HintListUi,
    /// One entry per frame: what each form gathered, in order.
    seen: Vec<Vec<Gathered>>,
}

fn open(
    forms: Vec<(FormContext<'static>, OptionsForm)>,
    size: (f32, f32),
) -> Harness<'static, Frame> {
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .build_ui_state(
            move |ui, frame: &mut Frame| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                let i18n = I18n::english();
                let gathered: Vec<Gathered> = frame
                    .forms
                    .iter()
                    .map(|(cx, form)| {
                        ui.push_id(cx.id_scope, |ui| {
                            show_options_form(ui, form, &frame.hints, cx, &i18n)
                        })
                        .inner
                    })
                    .collect();
                frame.seen.push(gathered);
            },
            Frame {
                forms,
                hints: HintListUi::default(),
                seen: Vec::new(),
            },
        );
    harness.run_steps(4);
    harness
}

fn wrapped(flags: FlagsWhenOff) -> FormContext<'static> {
    cx("find", FormLayout::Wrapped, flags)
}

fn ticked() -> OptionsForm {
    OptionsForm::default()
        .edited(OptionsEdit::CollationOn(true))
        .edited(OptionsEdit::Locale("en".into()))
}

fn position(harness: &Harness<'static, Frame>, role: Role, label: &str) -> (f32, f32) {
    let node = harness.get_by_role_and_label(role, label);
    let bounds = node.accesskit_node().raw_bounds().expect("laid out");
    (bounds.y0 as f32, bounds.x0 as f32)
}

#[test]
fn the_wrapped_layout_shows_every_control_in_reading_order() {
    let harness = open(
        vec![(wrapped(FlagsWhenOff::Hidden), ticked())],
        (1000.0, 700.0),
    );
    let controls = [
        (Role::CheckBox, "aggregate.options.collation.enabled"),
        (Role::TextInput, "aggregate.options.collation.locale"),
        (Role::ComboBox, "aggregate.options.collation.locale_pick"),
        (Role::ComboBox, "aggregate.options.collation.strength"),
        (Role::ComboBox, "aggregate.options.collation.case_level"),
        (Role::ComboBox, "aggregate.options.collation.case_first"),
        (
            Role::ComboBox,
            "aggregate.options.collation.numeric_ordering",
        ),
        (Role::ComboBox, "aggregate.options.collation.alternate"),
        (Role::TextInput, "aggregate.options.hint.label"),
        (Role::TextInput, "aggregate.options.max_time.label"),
        (Role::ComboBox, "aggregate.options.read_preference.label"),
        (Role::ComboBox, "aggregate.options.allow_disk_use.label"),
        (Role::TextInput, "aggregate.options.comment.label"),
        (Role::Button, "aggregate.options.reset"),
    ];
    let places: Vec<(f32, f32)> = controls
        .iter()
        .map(|(role, key)| position(&harness, *role, &t(key)))
        .collect();
    for pair in places.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let same_row = (a.0 - b.0).abs() < 4.0;
        assert!(
            b.0 > a.0 + 4.0 || (same_row && b.1 > a.1),
            "reading order broken: {a:?} then {b:?} in {places:?}"
        );
    }
}

#[test]
fn hidden_flags_appear_when_collation_is_ticked() {
    let off = open(
        vec![(wrapped(FlagsWhenOff::Hidden), OptionsForm::default())],
        (1000.0, 700.0),
    );
    assert!(
        off.query_by_label(&t("aggregate.options.collation.case_level"))
            .is_none()
    );
    let on = open(
        vec![(wrapped(FlagsWhenOff::Hidden), ticked())],
        (1000.0, 700.0),
    );
    on.get_by_role_and_label(Role::ComboBox, &t("aggregate.options.collation.case_level"));
}

#[test]
fn disabled_flags_stay_in_the_aggregate_layout() {
    let off = open(
        vec![(
            cx("aggregate", FormLayout::Columns, FlagsWhenOff::Disabled),
            OptionsForm::default(),
        )],
        (1000.0, 700.0),
    );
    off.get_by_role_and_label(Role::ComboBox, &t("aggregate.options.collation.case_level"));
}

#[test]
fn two_scopes_never_share_a_widget_id() {
    let label = t("aggregate.options.comment.label");
    let mut harness = open(
        vec![
            (
                cx("aggregate", FormLayout::Columns, FlagsWhenOff::Disabled),
                OptionsForm::default(),
            ),
            (wrapped(FlagsWhenOff::Hidden), OptionsForm::default()),
        ],
        (1000.0, 1400.0),
    );
    let fields = harness.get_all_by_role_and_label(Role::TextInput, &label);
    assert_eq!(fields.count(), 2);
    harness
        .get_all_by_role_and_label(Role::TextInput, &label)
        .nth(1)
        .expect("the second form's field")
        .click();
    harness.run_steps(2);
    harness
        .get_all_by_role_and_label(Role::TextInput, &label)
        .nth(1)
        .expect("the second form's field")
        .type_text("hi");
    harness.run_steps(2);
    let typed_in = |index: usize| {
        harness
            .state()
            .seen
            .iter()
            .filter(|frame| {
                frame[index]
                    .edits
                    .iter()
                    .any(|e| matches!(e, OptionsEdit::Comment(_)))
            })
            .count()
    };
    assert_eq!(typed_in(0), 0, "the first form heard nothing");
    assert!(typed_in(1) > 0, "the second form heard the typing");
}

#[test]
fn enter_is_reported_apart_from_focus_loss() {
    let label = t("aggregate.options.comment.label");
    let mut harness = open(
        vec![(wrapped(FlagsWhenOff::Hidden), OptionsForm::default())],
        (1000.0, 700.0),
    );
    harness
        .get_by_role_and_label(Role::TextInput, &label)
        .click();
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::TextInput, &label)
        .type_text("hi");
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    let seen: Vec<&Gathered> = harness.state().seen.iter().flatten().collect();
    assert!(seen.iter().any(|g| g.enter && g.commit), "Enter commits");
    // Leaving the field without Enter commits but is not an Enter.
    let mut away = open(
        vec![(wrapped(FlagsWhenOff::Hidden), OptionsForm::default())],
        (1000.0, 700.0),
    );
    away.get_by_role_and_label(Role::TextInput, &label).click();
    away.run_steps(2);
    away.get_by_role_and_label(Role::TextInput, &t("aggregate.options.hint.label"))
        .click();
    away.run_steps(3);
    let seen: Vec<&Gathered> = away.state().seen.iter().flatten().collect();
    assert!(seen.iter().any(|g| g.commit), "focus loss commits");
    assert!(seen.iter().all(|g| !g.enter), "but is no Enter");
}
