//! The Options section in the accesskit tree, rendered with the REAL English locale: every
//! control has a role and a name, Tab walks the section in reading order, the toggle is named
//! with its count and reports whether it is expanded, a problem is text under its field, every
//! option is usable on a read-only connection and the run dialog reads the options it sends.
//! Fast, offline, no GPU.
use curlybrace_core::pipeline::{ReadPreferenceMode, RunTarget};
use curlybrace_core::store::Namespace;
use curlybrace_ui::I18n;
use curlybrace_ui::app::{AggregateAction, OptionsAction};
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::AggregateState;
use curlybrace_ui::viewmodel::aggregate_hint_list::HintListUi;
use curlybrace_ui::viewmodel::aggregate_options::{OptionsEdit, Tri};
use curlybrace_ui::viewmodel::aggregate_run::{
    CountState, RunAvailability, RunDialog, RunDisabled, RunUi,
};
use curlybrace_ui::viewmodel::aggregate_view::AggregateView;
use curlybrace_ui::views::aggregate::show_aggregate;
use curlybrace_ui::views::aggregate_run::RunContext;
use eframe::egui;
use eframe::egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

struct Frame {
    draft: AggregateState,
    view: AggregateView,
    run: RunContext,
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn open(draft: AggregateState, run: RunContext) -> Harness<'static, Frame> {
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 1800.0))
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
                    &RunContext {
                        availability: frame.run.availability.clone(),
                        badge: frame.run.badge,
                    },
                    &I18n::english(),
                );
                // The reducer's part for the section, by the draft's own ops.
                for action in actions {
                    if let AggregateAction::Options(options) = action {
                        let draft = std::mem::take(&mut frame.draft);
                        frame.draft = match options {
                            OptionsAction::Toggle => draft.options_toggled(),
                            OptionsAction::Edit(e) => {
                                let typing = e.is_typing();
                                let draft = draft.options_edited(e);
                                match typing {
                                    true => draft,
                                    false => draft.options_applied().0,
                                }
                            }
                            OptionsAction::Apply => draft.options_applied().0,
                            OptionsAction::Reset => draft.options_reset().0,
                            OptionsAction::HintListWanted => draft,
                        };
                    }
                }
            },
            Frame {
                draft,
                view: AggregateView::default(),
                run,
            },
        );
    harness.run_steps(4);
    harness
}

/// One `$match` stage, previewed, the section open with the collation on and the indexes listed.
fn full_section() -> AggregateState {
    let (draft, sent) = AggregateState::default()
        .add_stage("$match")
        .begin_preview();
    let (request, _) = sent.expect("valid");
    let mut draft = draft
        .arrived(request, Ok(Vec::new()))
        .options_toggled()
        .options_edited(OptionsEdit::CollationOn(true))
        .options_edited(OptionsEdit::Locale("en".into()));
    draft.options_ui.hint = HintListUi::default().from_names(vec!["a_1".into()]);
    draft
}

/// The controls of the section, in reading order, with their roles.
fn section_controls() -> Vec<(Role, String)> {
    vec![
        (Role::CheckBox, t("aggregate.options.collation.enabled")),
        (Role::TextInput, t("aggregate.options.collation.locale")),
        (Role::ComboBox, t("aggregate.options.collation.locale_pick")),
        (Role::ComboBox, t("aggregate.options.collation.strength")),
        (Role::ComboBox, t("aggregate.options.collation.case_level")),
        (Role::ComboBox, t("aggregate.options.collation.case_first")),
        (
            Role::ComboBox,
            t("aggregate.options.collation.numeric_ordering"),
        ),
        (Role::ComboBox, t("aggregate.options.collation.alternate")),
        (Role::ComboBox, t("aggregate.options.hint.existing")),
        (Role::TextInput, t("aggregate.options.hint.label")),
        (Role::Button, t("aggregate.options.hint.clear")),
        (Role::ComboBox, t("aggregate.options.allow_disk_use.label")),
        (Role::TextInput, t("aggregate.options.max_time.label")),
        (Role::TextInput, t("aggregate.options.comment.label")),
        (Role::ComboBox, t("aggregate.options.read_preference.label")),
        (Role::Button, t("aggregate.options.reset")),
    ]
}

#[test]
fn every_options_control_has_a_role_and_a_name() {
    let harness = open(full_section(), RunContext::default());
    for (role, label) in section_controls() {
        harness.get_by_role_and_label(role, &label);
    }
}

#[test]
fn tab_walks_the_section_in_reading_order() {
    let mut harness = open(full_section(), RunContext::default());
    harness
        .get_by_role_and_label(Role::Button, &t("aggregate.options.toggle"))
        .focus();
    harness.run_steps(2);
    let wanted: Vec<String> = section_controls().into_iter().map(|(_, l)| l).collect();
    let mut seen = Vec::new();
    for _ in 0..60 {
        harness.key_press(egui::Key::Tab);
        harness.run_steps(2);
        let label = harness
            .root()
            .children_recursive()
            .find(|n| n.accesskit_node().is_focused())
            .and_then(|n| n.accesskit_node().label());
        if let Some(label) = label.filter(|l| wanted.contains(l)) {
            if seen.last() != Some(&label) {
                seen.push(label);
            }
        }
    }
    assert_eq!(seen[..wanted.len()], wanted[..], "reading order");
}

#[test]
fn the_toggle_is_toggled_and_named_with_the_count() {
    let one = full_section()
        .options_edited(OptionsEdit::AllowDiskUse(Tri::On))
        .options_applied()
        .0;
    let harness = open(one, RunContext::default());
    let label = I18n::english().t_with("aggregate.options.toggle_count", &[("n", "2")]);
    let node = harness.get_by_role_and_label(Role::Button, &label);
    assert_eq!(node.accesskit_node().toggled(), Some(Toggled::True));
}

#[test]
fn a_problem_is_text_under_its_field() {
    let draft = full_section().options_edited(OptionsEdit::MaxTime("soon".into()));
    let harness = open(draft, RunContext::default());
    let label = t("aggregate.options.max_time.label");
    let field = harness.get_by_role_and_label(Role::TextInput, &label);
    let said = field.accesskit_node().description().unwrap_or_default();
    assert!(said.contains("whole number of milliseconds"), "{said}");
}

#[test]
fn every_option_is_usable_on_a_read_only_connection() {
    let read_only = RunContext {
        availability: RunAvailability::Disabled(RunDisabled::ReadOnly),
        ..RunContext::default()
    };
    let harness = open(full_section(), read_only);
    for (role, label) in section_controls() {
        let node = harness.get_by_role_and_label(role, &label);
        assert!(!node.accesskit_node().is_disabled(), "{label}");
    }
}

#[test]
fn the_run_dialog_reads_the_options_and_the_primary_rule() {
    let (draft, sent) = AggregateState::default()
        .add_stage("$match")
        .add_stage("$out")
        .begin_preview();
    let (request, _) = sent.expect("valid");
    let mut draft = draft.arrived(request, Ok(Vec::new()));
    draft = draft
        .options_edited(OptionsEdit::Comment("report".into()))
        .options_edited(OptionsEdit::ReadPreference(Some(
            ReadPreferenceMode::Nearest,
        )))
        .options_applied()
        .0;
    draft.run = RunUi::default().opened(RunDialog {
        target: RunTarget {
            db: None,
            collection: "dest".into(),
        },
        stage: "$out".into(),
        source: Namespace::new("app", "people"),
        typed: String::new(),
        count: CountState::Loading,
    });
    assert_eq!(draft.options.set_count(), 2);
    let harness = open(draft, RunContext::default());
    harness.get_by_label_contains("This run sends these options: ");
    harness.get_by_label(&t("aggregate.options.run.primary"));
}
