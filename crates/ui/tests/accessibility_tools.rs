//! The tool paths dialog in the accesskit tree: every tool has a named group of controls, the
//! path boxes and buttons say which tool they belong to, a problem is stated in words, and the
//! suggested places are buttons that spell out the path. Fast, offline, no GPU.
use curlybrace_core::tools::{ToolChoice, ToolError, ToolKind, ToolVersion};
use curlybrace_ui::I18n;
use curlybrace_ui::app::tools_state::{ProbeScope, ToolsAction, ToolsState};
use curlybrace_ui::views::tools_settings::show_tools_settings;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn tw(key: &str, params: &[(&str, &str)]) -> String {
    I18n::english().t_with(key, params)
}

struct Run {
    tools: ToolsState,
    actions: Vec<ToolsAction>,
}

fn harness(tools: ToolsState) -> Harness<'static, Run> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 1100.0))
        .build_state(
            |ctx, run: &mut Run| {
                run.actions
                    .extend(show_tools_settings(ctx, &run.tools, &I18n::english()));
            },
            Run {
                tools,
                actions: Vec::new(),
            },
        );
    harness.run_steps(3);
    harness
}

fn answered(
    tools: ToolsState,
    kind: ToolKind,
    result: Result<ToolVersion, ToolError>,
) -> ToolsState {
    let (tools, queue) = tools.recheck(ProbeScope::Global, kind).take_queue();
    let request = queue.last().expect("a request").request;
    tools.probed(ProbeScope::Global, kind, request, result)
}

fn opened() -> ToolsState {
    let suggestions: BTreeMap<ToolKind, Vec<PathBuf>> = [(
        ToolKind::Mongosh,
        vec![PathBuf::from("/opt/homebrew/bin/mongosh")],
    )]
    .into_iter()
    .collect();
    let (tools, _) = ToolsState::default()
        .open_settings(suggestions)
        .take_queue();
    tools
}

#[test]
fn a_closed_dialog_draws_nothing() {
    let harness = harness(ToolsState::default());
    assert!(harness.query_by_label(&t("tools.heading")).is_none());
}

#[test]
fn every_tool_has_named_controls() {
    let harness = harness(opened());
    harness.get_by_label(&t("tools.heading"));
    for kind in ToolKind::ALL {
        let name = t(&format!("tools.name_{}", kind.binary_name()));
        harness.get_by_label(&name);
        harness.get_by_role_and_label(Role::TextInput, &tw("tools.path_field", &[("tool", &name)]));
        harness.get_by_role_and_label(
            Role::Button,
            &tw("tools.choose_file_named", &[("tool", &name)]),
        );
        harness.get_by_role_and_label(
            Role::Button,
            &tw("tools.check_again_named", &[("tool", &name)]),
        );
    }
}

fn tool_label(key: &str, kind: ToolKind) -> String {
    let name = t(&format!("tools.name_{}", kind.binary_name()));
    tw(key, &[("tool", &name)])
}

#[test]
fn the_two_ways_to_get_a_tool_are_radio_buttons_that_name_their_tool() {
    let harness = harness(opened());
    for kind in ToolKind::ALL {
        harness.get_by_role_and_label(Role::RadioButton, &tool_label("tools.path_named", kind));
        harness.get_by_role_and_label(
            Role::RadioButton,
            &tool_label("tools.executable_named", kind),
        );
    }
}

#[test]
fn a_problem_is_stated_in_words() {
    let tools = answered(opened(), ToolKind::Mongodump, Err(ToolError::NotExecutable));
    let harness = harness(tools);
    harness.get_by_label(&t(ToolError::NotExecutable.i18n_key()));
}

#[test]
fn a_found_version_is_stated() {
    let tools = answered(opened(), ToolKind::Mongosh, Ok(ToolVersion::new(2, 3, 1)));
    let harness = harness(tools);
    harness.get_by_label(&tw("tools.version", &[("version", "2.3.1")]));
}

#[test]
fn a_suggested_place_is_a_button_that_spells_out_the_path() {
    let mut harness = harness(opened());
    let label = tw("tools.suggestion", &[("path", "/opt/homebrew/bin/mongosh")]);
    harness.get_by_role_and_label(Role::Button, &label).click();
    harness.run_steps(2);
    assert_eq!(
        harness.state().actions,
        vec![ToolsAction::ChooseGlobal {
            kind: ToolKind::Mongosh,
            choice: ToolChoice::Executable("/opt/homebrew/bin/mongosh".into()),
        }]
    );
}

#[test]
fn choose_file_asks_for_the_dialog_for_that_tool() {
    let mut harness = harness(opened());
    let name = t("tools.name_mongorestore");
    harness
        .get_by_role_and_label(
            Role::Button,
            &tw("tools.choose_file_named", &[("tool", &name)]),
        )
        .click();
    harness.run_steps(2);
    assert_eq!(
        harness.state().actions,
        vec![ToolsAction::Browse {
            scope: ProbeScope::Global,
            kind: ToolKind::Mongorestore,
        }]
    );
}

#[test]
fn picking_the_path_search_when_already_on_it_does_nothing_and_close_closes() {
    let mut harness = harness(opened());
    harness
        .get_by_role_and_label(
            Role::RadioButton,
            &tool_label("tools.path_named", ToolKind::Mongosh),
        )
        .click();
    harness.run_steps(2);
    assert!(harness.state().actions.is_empty());
    harness
        .get_by_role_and_label(Role::Button, &t("common.close"))
        .click();
    harness.run_steps(2);
    assert_eq!(harness.state().actions, vec![ToolsAction::CloseSettings]);
}

fn path_box_label(kind: ToolKind) -> String {
    let name = t(&format!("tools.name_{}", kind.binary_name()));
    tw("tools.path_field", &[("tool", &name)])
}

#[test]
fn a_typed_path_is_applied_with_enter() {
    let mut harness = harness(opened());
    let label = path_box_label(ToolKind::Mongodump);
    let input = harness.get_by_role_and_label(Role::TextInput, &label);
    input.click();
    input.type_text("  /opt/tools/mongodump  ");
    harness.run_steps(2);
    assert!(
        harness.state().actions.is_empty(),
        "nothing is applied while typing"
    );
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(
        harness.state().actions,
        vec![ToolsAction::ChooseGlobal {
            kind: ToolKind::Mongodump,
            choice: ToolChoice::Executable("/opt/tools/mongodump".into()),
        }]
    );
}

#[test]
fn an_empty_box_applies_nothing() {
    let mut harness = harness(opened());
    let label = path_box_label(ToolKind::Mongosh);
    harness
        .get_by_role_and_label(Role::TextInput, &label)
        .click();
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert!(harness.state().actions.is_empty());
}

#[test]
fn closing_right_after_typing_a_path_keeps_the_path() {
    let mut harness = harness(opened());
    let label = path_box_label(ToolKind::Mongosh);
    let input = harness.get_by_role_and_label(Role::TextInput, &label);
    input.click();
    input.type_text("/opt/tools/mongosh");
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::Button, &t("common.close"))
        .click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().actions,
        vec![
            ToolsAction::ChooseGlobal {
                kind: ToolKind::Mongosh,
                choice: ToolChoice::Executable("/opt/tools/mongosh".into()),
            },
            ToolsAction::CloseSettings,
        ]
    );
}

#[test]
fn check_again_is_unavailable_while_a_check_is_running() {
    // Opening the dialog checks every tool; Mongosh has answered, Mongodump has not.
    let tools = answered(opened(), ToolKind::Mongosh, Ok(ToolVersion::new(2, 3, 1)));
    let harness = harness(tools);
    let label = tool_label("tools.check_again_named", ToolKind::Mongodump);
    let button = harness.get_by_role_and_label(Role::Button, &label);
    assert!(button.accesskit_node().is_disabled());
    let idle_label = tool_label("tools.check_again_named", ToolKind::Mongosh);
    let idle = harness.get_by_role_and_label(Role::Button, &idle_label);
    assert!(!idle.accesskit_node().is_disabled());
}
