//! The Shell tab in the app frame: the switcher opens it, the central panel draws it for the
//! active session (connected or not), and its buttons reach the reducer.
use super::index_write_tests::{browsing, with_flags};
use super::panels::{UiEvent, show_workspace, shows_manager};
use super::reducer::{Input, update};
use super::shell_actions::ShellAction;
use super::state::{AppState, Workspace};
use crate::I18n;
use crate::viewmodel::shell::ToolProblem;
use crate::views::compare::show_workspace_switcher;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Event;
use curlybrace_core::tools::ToolError;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn open_shell(read_only: bool) -> AppState {
    let (s, _) = browsing(with_flags(false, read_only));
    update(s, Input::Shell(ShellAction::Open)).0
}

fn lost(s: AppState) -> AppState {
    let id = s.active.expect("an active session");
    let failed = Event::Failed {
        session: id,
        context: "find",
        error: CoreError::Unreachable,
    };
    update(s, Input::Event(failed)).0
}

struct Frame {
    state: AppState,
    out: Vec<UiEvent>,
}

fn workspace(state: AppState) -> Harness<'static, Frame> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_state(
            |ctx, frame: &mut Frame| {
                show_workspace(ctx, &mut frame.state, &I18n::english(), &mut frame.out, 0);
            },
            Frame {
                state,
                out: Vec::new(),
            },
        );
    harness.run_steps(3);
    harness
}

#[test]
fn the_shell_workspace_draws_the_shell_tab() {
    let s = open_shell(false);
    assert_eq!(s.workspace, Workspace::Shell);
    let h = workspace(s);
    h.get_by_role_and_label(Role::MultilineTextInput, &t("shell.editor_label"));
    h.get_by_role_and_label(Role::Region, &t("shell.output.region"));
}

#[test]
fn a_read_only_session_shows_why_run_is_off() {
    let h = workspace(open_shell(true));
    h.get_by_label(&t("shell.read_only_note"));
    assert!(run_is_disabled(&h));
}

fn run_is_disabled(h: &Harness<'static, Frame>) -> bool {
    let label = t("shell.run");
    h.get_by_role_and_label(Role::Button, &label)
        .accesskit_node()
        .is_disabled()
}

#[test]
fn a_lost_connection_keeps_the_shell_tab_up_with_run_off() {
    let s = lost(open_shell(false));
    assert_eq!(s.workspace, Workspace::Shell);
    assert!(!shows_manager(&s));
    let h = workspace(s);
    h.get_by_label(&t("shell.not_connected_note"));
    assert!(run_is_disabled(&h));
}

#[test]
fn other_workspaces_still_show_the_manager_without_a_connection() {
    let (s, _) = browsing(with_flags(false, false));
    let s = lost(s);
    assert_eq!(s.workspace, Workspace::Browse);
    assert!(shows_manager(&s));
    let none = AppState {
        workspace: Workspace::Shell,
        ..AppState::new(Vec::new())
    };
    assert!(
        shows_manager(&none),
        "no session, nothing to show a shell for"
    );
}

#[test]
fn the_tool_paths_button_opens_the_tool_paths_dialog() {
    let s = open_shell(false);
    let id = s.active.expect("active");
    let problem = ToolProblem::Missing(ToolError::NotOnPath);
    let shell = s.shell.get_or_new(id).tool_problem(problem);
    let tabs = (*s.shell).clone().with(id, shell);
    let mut h = workspace(AppState {
        shell: Box::new(tabs),
        ..s
    });
    h.get_by_role_and_label(Role::Button, &t("shell.open_tool_paths"))
        .click();
    h.run_steps(2);
    let opened = h
        .state()
        .out
        .iter()
        .any(|e| matches!(e, UiEvent::OpenToolSettings));
    assert!(opened);
}

fn switcher(current: Workspace, connected: bool) -> Harness<'static, Option<Input>> {
    let mut h = Harness::new_ui_state(
        move |ui, out: &mut Option<Input>| {
            let i18n = I18n::english();
            if let Some(input) = show_workspace_switcher(ui, current, connected, &i18n) {
                *out = Some(input);
            }
        },
        None,
    );
    h.run_steps(2);
    h
}

#[test]
fn the_shell_entry_is_disabled_while_not_connected() {
    let (shell, compare) = (t("workspace.shell"), t("workspace.compare"));
    let mut h = switcher(Workspace::Browse, false);
    let entry = h.get_by_role_and_label(Role::RadioButton, &shell);
    assert!(entry.accesskit_node().is_disabled());
    entry.click();
    h.run_steps(2);
    let picked = h.state_mut().take();
    assert!(picked.is_none(), "{picked:?}");
    // The other entries still work.
    let other = h.get_by_role_and_label(Role::RadioButton, &compare);
    assert!(!other.accesskit_node().is_disabled());
}

#[test]
fn the_switcher_opens_the_shell_and_goes_back_to_browse() {
    let picked = |current: Workspace, label: &str| {
        let mut h = switcher(current, true);
        h.get_by_role_and_label(Role::RadioButton, label).click();
        h.run_steps(2);
        h.state_mut().take()
    };
    let opened = picked(Workspace::Browse, &t("workspace.shell"));
    assert!(matches!(opened, Some(Input::Shell(ShellAction::Open))));
    let back = picked(Workspace::Shell, &t("workspace.browse")).expect("an input");
    let s = update(open_shell(false), back).0;
    assert_eq!(s.workspace, Workspace::Browse);
}
