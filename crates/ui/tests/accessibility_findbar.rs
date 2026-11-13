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

mod guard_setting {
    use curlybrace_ui::I18n;
    use curlybrace_ui::views::findbar::{FindAction, show_guard_setting};
    use eframe::egui;
    use eframe::egui::accesskit::{Role, Toggled};
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    const LABEL: &str = "Warn before queries that would scan a collection";

    struct Frame {
        guard: bool,
        seen: Vec<FindAction>,
    }

    fn open(guard: bool) -> Harness<'static, Frame> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 200.0))
            .build_ui_state(
                |ui, frame: &mut Frame| {
                    let action = show_guard_setting(ui, frame.guard, &I18n::english());
                    frame.seen.extend(action);
                },
                Frame {
                    guard,
                    seen: Vec::new(),
                },
            );
        harness.run_steps(4);
        harness
    }

    #[test]
    fn the_checkbox_has_its_label_and_state_and_no_missing_key_marker() {
        for guard in [true, false] {
            let harness = open(guard);
            let checkbox = harness.get_by_role_and_label(Role::CheckBox, LABEL);
            let expected = if guard { Toggled::True } else { Toggled::False };
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
        assert_eq!(harness.state().seen, vec![FindAction::SetGuard(false)]);
    }
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

    const HIDDEN: [&str; 4] = ["Sort", "Projection", "Skip", "Limit"];

    fn open(has_next: bool) -> Harness<'static, Bar> {
        open_with(FindBarState::default(), has_next)
    }

    fn open_with(state: FindBarState, has_next: bool) -> Harness<'static, Bar> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 400.0))
            .build_ui_state(
                |ui, bar: &mut Bar| {
                    let (state, has_next) = (&mut bar.state, bar.has_next);
                    let action = show_findbar(ui, state, false, has_next, &I18n::english());
                    bar.seen.extend(action);
                },
                Bar {
                    state,
                    seen: Vec::new(),
                    has_next,
                },
            );
        theme::install(&harness.ctx);
        harness.run_steps(4);
        harness
    }

    fn shown(harness: &Harness<'static, Bar>, label: &str) -> bool {
        harness
            .query_by_role_and_label(Role::TextInput, label)
            .is_some()
    }

    fn expanded(harness: &Harness<'static, Bar>, label: &str) -> Option<bool> {
        harness
            .get_by_role_and_label(Role::Button, label)
            .accesskit_node()
            .data()
            .is_expanded()
    }

    #[test]
    fn every_query_field_is_an_editable_text_field_with_its_label() {
        let mut harness = open(false);
        harness
            .get_by_role_and_label(Role::Button, "More options")
            .click();
        harness.run_steps(4);
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

    #[test]
    fn closed_and_empty_more_keeps_its_fields_out_of_the_tree_until_toggled() {
        let mut harness = open(false);
        assert!(shown(&harness, "Filter"));
        for label in HIDDEN {
            assert!(!shown(&harness, label), "{label} shown while closed");
        }
        assert_eq!(expanded(&harness, "More options"), Some(false));
        harness
            .get_by_role_and_label(Role::Button, "More options")
            .click();
        harness.run_steps(4);
        for label in HIDDEN {
            assert!(shown(&harness, label), "{label} missing once opened");
        }
        assert_eq!(expanded(&harness, "More options"), Some(true));
        harness
            .get_by_role_and_label(Role::Button, "More options")
            .click();
        harness.run_steps(4);
        assert!(HIDDEN.iter().all(|label| !shown(&harness, label)));
    }

    #[test]
    fn more_opens_by_itself_when_a_hidden_field_holds_a_value_and_says_so_when_closed() {
        let mut state = FindBarState::default();
        state.text.sort = "{\"a\": 1}".into();
        let mut harness = open_with(state, false);
        for label in HIDDEN {
            assert!(
                shown(&harness, label),
                "{label} hidden although Sort is set"
            );
        }
        assert_eq!(
            harness
                .get_by_role_and_label(Role::TextInput, "Sort")
                .value(),
            Some("{\"a\": 1}".to_string())
        );
        harness
            .get_by_role_and_label(Role::Button, "More options")
            .click();
        harness.run_steps(4);
        assert!(!shown(&harness, "Sort"));
        assert_eq!(expanded(&harness, "More options (1 set)"), Some(false));
        assert_eq!(harness.state().state.text.sort, "{\"a\": 1}", "kept");
    }

    #[test]
    fn tab_moves_from_the_filter_to_find() {
        let mut harness = open(false);
        harness
            .get_by_role_and_label(Role::TextInput, "Filter")
            .click();
        harness.run_steps(2);
        harness.key_press(egui::Key::Tab);
        harness.run_steps(2);
        assert!(
            harness
                .get_by_role_and_label(Role::Button, "Find")
                .accesskit_node()
                .is_focused()
        );
    }

    #[test]
    fn the_buttons_share_the_filter_row_on_a_wide_window() {
        let harness = open(false);
        let filter = harness
            .get_by_role_and_label(Role::TextInput, "Filter")
            .rect();
        let find = harness.get_by_role_and_label(Role::Button, "Find").rect();
        let more = harness
            .get_by_role_and_label(Role::Button, "More options")
            .rect();
        assert!(find.left() > filter.right(), "Find is beside the Filter");
        assert!(
            (find.center().y - filter.center().y).abs() < 1.0,
            "Find {find:?} is centred on the Filter's box {filter:?}"
        );
        assert!((more.center().y - find.center().y).abs() < 1.0, "same row");
        assert!(
            filter.width() > 300.0,
            "the Filter takes the rest: {filter:?}"
        );
    }
}
