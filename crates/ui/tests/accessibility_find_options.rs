//! The Query options of the find bar in the accesskit tree, rendered with the REAL English locale:
//! every control has a role and a name, Tab walks the bar and then the section in reading order,
//! the toggles are named with their counts and report whether they are expanded, a problem is
//! text under its field and Find reads the reason, every option is usable on a read-only
//! connection and the status of the shown page is text. Fast, offline, no GPU.
use curlybrace_core::pipeline::{PipelineOptions, ReadPreferenceMode};
use curlybrace_ui::I18n;
use curlybrace_ui::theme;
use curlybrace_ui::viewmodel::FindBarState;
use curlybrace_ui::viewmodel::aggregate_hint_list::HintListUi;
use curlybrace_ui::viewmodel::aggregate_options::OptionsEdit;
use curlybrace_ui::views::findbar::{FindAction, show_findbar_with};
use curlybrace_ui::views::findbar_options::show_page_options_status;
use eframe::egui;
use eframe::egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

struct Frame {
    state: FindBarState,
    hints: HintListUi,
    read_only: bool,
    seen: Vec<FindAction>,
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn open(state: FindBarState) -> Harness<'static, Frame> {
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 900.0))
        .build_ui_state(
            move |ui, frame: &mut Frame| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                let i18n = I18n::english();
                // Previous and Next are on, so Tab reaches them.
                let actions =
                    show_findbar_with(ui, &mut frame.state, (true, true), &frame.hints, &i18n);
                frame.seen.extend(actions);
            },
            Frame {
                state,
                hints: HintListUi::default().from_names(vec!["a_1".into()]),
                read_only: false,
                seen: Vec::new(),
            },
        );
    harness.run_steps(4);
    harness
}

/// The section open, the collation ticked with its locale, More open.
fn full() -> FindBarState {
    [
        OptionsEdit::CollationOn(true),
        OptionsEdit::Locale("en".into()),
    ]
    .into_iter()
    .fold(
        FindBarState {
            more: Some(true),
            options_open: Some(true),
            ..FindBarState::default()
        },
        FindBarState::options_edited,
    )
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
        (Role::TextInput, t("aggregate.options.max_time.label")),
        (Role::ComboBox, t("aggregate.options.read_preference.label")),
        (Role::ComboBox, t("aggregate.options.allow_disk_use.label")),
        (Role::TextInput, t("aggregate.options.comment.label")),
        (Role::Button, t("aggregate.options.reset")),
    ]
}

/// The bar's own controls before the section, in reading order.
fn bar_controls() -> Vec<String> {
    let toggle = I18n::english().t_with("query.options.toggle_count", &[("n", "1")]);
    vec![
        t("findbar.filter"),
        t("findbar.run"),
        t("findbar.clear"),
        t("codegen.button"),
        t("findbar.prev"),
        t("findbar.next"),
        t("findbar.more"),
        t("findbar.sort"),
        t("findbar.projection"),
        t("findbar.skip"),
        t("findbar.limit"),
        toggle,
    ]
}

#[test]
fn every_query_options_control_has_a_role_and_a_name() {
    let harness = open(full());
    for (role, label) in section_controls() {
        // The hint's Clear shares its name with the bar's own Clear: both are there.
        let found = harness.query_all_by_role_and_label(role, &label).count();
        assert!(found >= 1, "{label}");
    }
    harness.get_by_role_and_label(
        Role::Button,
        &I18n::english().t_with("query.options.toggle_count", &[("n", "1")]),
    );
}

#[test]
fn tab_walks_the_bar_then_the_section_in_reading_order() {
    let mut harness = open(full());
    harness
        .get_by_role_and_label(Role::TextInput, &t("findbar.filter"))
        .focus();
    harness.run_steps(2);
    let wanted: Vec<String> = bar_controls()
        .into_iter()
        .chain(section_controls().into_iter().map(|(_, label)| label))
        .collect();
    let mut seen = vec![t("findbar.filter")];
    for _ in 0..80 {
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
fn the_toggles_are_toggled_and_named_with_their_counts() {
    let one = FindBarState {
        more: Some(true),
        ..FindBarState::default()
    }
    .options_edited(OptionsEdit::Comment("why".into()));
    let harness = open(FindBarState {
        options_open: Some(true),
        ..one.clone()
    });
    let label = I18n::english().t_with("query.options.toggle_count", &[("n", "1")]);
    let node = harness.get_by_role_and_label(Role::Button, &label);
    assert_eq!(node.accesskit_node().toggled(), Some(Toggled::True));
    // More closed: its label counts the set options and it says it is collapsed.
    let closed = open(FindBarState {
        more: Some(false),
        ..one
    });
    let more = I18n::english().t_with("findbar.more_set", &[("count", "1")]);
    let node = closed.get_by_role_and_label(Role::Button, &more);
    assert_eq!(node.accesskit_node().data().is_expanded(), Some(false));
}

#[test]
fn a_problem_is_text_under_its_field_and_find_reads_the_reason() {
    let state = FindBarState {
        more: Some(true),
        options_open: Some(true),
        ..FindBarState::default()
    }
    .options_edited(OptionsEdit::MaxTime("soon".into()));
    let harness = open(state);
    let label = t("aggregate.options.max_time.label");
    let field = harness.get_by_role_and_label(Role::TextInput, &label);
    let said = field.accesskit_node().description().unwrap_or_default();
    assert!(said.contains("whole number of milliseconds"), "{said}");
    let run = t("findbar.run");
    let find = harness.get_by_role_and_label(Role::Button, &run);
    assert_eq!(
        find.accesskit_node().description().unwrap_or_default(),
        t("query.options.blocked")
    );
    assert!(find.accesskit_node().is_disabled());
}

#[test]
fn every_option_is_usable_on_a_read_only_connection() {
    let mut harness = open(full());
    harness.state_mut().read_only = true;
    harness.run_steps(2);
    for (role, label) in section_controls() {
        for node in harness.query_all_by_role_and_label(role, &label) {
            assert!(!node.accesskit_node().is_disabled(), "{label}");
        }
    }
    let run = t("findbar.run");
    assert!(
        !harness
            .get_by_role_and_label(Role::Button, &run)
            .accesskit_node()
            .is_disabled()
    );
}

struct Status {
    options: PipelineOptions,
}

#[test]
fn the_page_status_is_text() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 200.0))
        .build_ui_state(
            |ui, status: &mut Status| {
                ui.horizontal_wrapped(|ui| {
                    show_page_options_status(ui, &status.options, &I18n::english())
                });
            },
            Status {
                options: PipelineOptions {
                    read_preference: Some(ReadPreferenceMode::SecondaryPreferred),
                    allow_disk_use: Some(true),
                    max_time_ms: Some(500),
                    ..PipelineOptions::default()
                },
            },
        );
    harness.run_steps(3);
    harness.get_by_label("Read from secondaryPreferred");
    harness.get_by_label("Time limit: 500 ms");
    harness.get_by_label("Disk use allowed");
    harness.get_by_label("Recent writes may not show yet.");
    harness.state_mut().options = PipelineOptions::default();
    harness.run_steps(3);
    assert!(harness.query_by_label_contains("Disk use").is_none());
}
