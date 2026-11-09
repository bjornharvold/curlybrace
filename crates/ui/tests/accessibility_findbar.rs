//! The automatic-verdict checkbox in the accesskit tree, rendered with the REAL English locale:
//! it has its accessible name, shows its state, reports a toggle and never shows a missing-key
//! marker. Fast, offline.
use curlybrace_ui::I18n;
use curlybrace_ui::views::findbar::{FindAction, show_automatic_verdict};
use eframe::egui;
use eframe::egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const LABEL: &str = "Check index use automatically";

struct Frame {
    automatic: bool,
    seen: Vec<FindAction>,
}

fn open(automatic: bool) -> Harness<'static, Frame> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 200.0))
        .build_ui_state(
            |ui, frame: &mut Frame| {
                let action = show_automatic_verdict(ui, frame.automatic, &I18n::english());
                frame.seen.extend(action);
            },
            Frame {
                automatic,
                seen: Vec::new(),
            },
        );
    harness.run_steps(4);
    harness
}

#[test]
fn the_checkbox_has_its_label_and_state_and_no_missing_key_marker() {
    for automatic in [true, false] {
        let harness = open(automatic);
        let checkbox = harness.get_by_role_and_label(Role::CheckBox, LABEL);
        let expected = if automatic {
            Toggled::True
        } else {
            Toggled::False
        };
        assert_eq!(checkbox.accesskit_node().toggled(), Some(expected));
        let mut labels = Vec::new();
        for node in harness.root().children_recursive() {
            labels.extend(node.accesskit_node().label());
        }
        assert!(labels.iter().all(|l| !l.contains('\u{27e6}')), "{labels:?}");
    }
}

#[test]
fn clicking_reports_the_opposite_setting() {
    let mut harness = open(true);
    harness.get_by_role_and_label(Role::CheckBox, LABEL).click();
    harness.run_steps(4);
    assert_eq!(
        harness.state().seen,
        vec![FindAction::SetAutomaticVerdict(false)]
    );
}

mod query_bar {
    use curlybrace_ui::I18n;
    use curlybrace_ui::theme;
    use curlybrace_ui::viewmodel::FindBarState;
    use curlybrace_ui::views::findbar::{FindAction, show_findbar};
    use eframe::egui::accesskit::Role;
    use eframe::egui::{self, Shape};
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    struct Bar {
        state: FindBarState,
        seen: Vec<FindAction>,
        has_next: bool,
    }

    fn open(has_next: bool) -> Harness<'static, Bar> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 400.0))
            .build_ui_state(
                |ui, bar: &mut Bar| {
                    let (state, has_next) = (&mut bar.state, bar.has_next);
                    let action = show_findbar(ui, state, false, has_next, &I18n::english());
                    bar.seen.extend(action);
                },
                Bar {
                    state: FindBarState::default(),
                    seen: Vec::new(),
                    has_next,
                },
            );
        theme::install(&harness.ctx);
        harness.run_steps(4);
        harness
    }

    #[test]
    fn every_query_field_is_an_editable_text_field_with_its_label() {
        let harness = open(false);
        for label in ["Filter", "Sort", "Projection", "Skip", "Limit"] {
            let field = harness.get_by_role_and_label(Role::TextInput, label);
            assert_eq!(field.value().as_deref(), Some(""), "{label}");
        }
    }

    #[test]
    fn typing_in_a_field_edits_the_query_text() {
        let mut harness = open(false);
        harness
            .get_by_role_and_label(Role::TextInput, "Filter")
            .click();
        harness.run_steps(2);
        harness
            .get_by_role_and_label(Role::TextInput, "Filter")
            .type_text("{}");
        harness.run_steps(2);
        assert_eq!(harness.state().state.text.filter, "{}");
    }

    #[test]
    fn find_is_a_button_and_prev_next_keep_their_enabled_state() {
        let harness = open(false);
        harness.get_by_role_and_label(Role::Button, "Find");
        harness.get_by_role_and_label(Role::Button, "Clear");
        for label in ["Previous", "Next"] {
            assert!(
                harness
                    .get_by_role_and_label(Role::Button, label)
                    .accesskit_node()
                    .is_disabled(),
                "{label}"
            );
        }
        let harness = open(true);
        assert!(
            !harness
                .get_by_role_and_label(Role::Button, "Next")
                .accesskit_node()
                .is_disabled()
        );
    }

    #[test]
    fn clicking_find_runs_the_query() {
        let mut harness = open(false);
        harness.get_by_role_and_label(Role::Button, "Find").click();
        harness.run_steps(4);
        assert!(
            matches!(harness.state().seen.as_slice(), [FindAction::Run(_)]),
            "{:?}",
            harness.state().seen
        );
    }

    #[test]
    fn find_is_drawn_as_the_filled_primary_button() {
        let harness = open(false);
        let find = harness.get_by_role_and_label(Role::Button, "Find").rect();
        let primary = theme::current(&harness.ctx).primary;
        let filled = harness.output().shapes.iter().any(|c| match &c.shape {
            Shape::Rect(r) => r.fill == primary && r.rect.contains_rect(find.shrink(1.0)),
            _ => false,
        });
        assert!(filled, "no primary fill behind the Find button at {find:?}");
    }
}
