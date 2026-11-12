//! Fixtures for the Shell tab screenshots: the completion list open in the editor, and a run
//! stopped by the syntax check (its notice, and the problem line under the editor).
use super::shots::{Shot, open_session};
use curlybrace_ui::I18n;
use curlybrace_ui::app::state::{AppState, Workspace};
use curlybrace_ui::viewmodel::SidebarState;
use curlybrace_ui::viewmodel::shell::ShellState;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::kittest::Queryable;

/// A script with two problems: line 1 has a stray `)`, and the selected line 2 never closes
/// its `find(`.
const BROKEN: &str = "db.customers.find({ status: \"active\" }))\ndb.invoices.find({ total: 1 }";
/// The selection that was run: line 2.
const SELECTED_FROM: usize = 41;

/// The Shell tab of a connected session, with `shell` in it and `crm` loaded in the sidebar.
fn shell_tab(state: &mut AppState, shell: ShellState) {
    let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let id = open_session(state, "Production");
    state.sidebar = SidebarState::default()
        .with_databases(names(&["billing", "crm"]))
        .toggled("crm")
        .with_collections("crm", names(&["customers", "invoices", "notes"]));
    *state.shell = (*state.shell).clone().with(id, shell);
    state.workspace = Workspace::Shell;
}

/// The Run of line 2 of [`BROKEN`] was stopped: the notice (line 2) with Go to problem and Run
/// anyway, and under the editor the first problem of the whole script (line 1).
pub fn syntax_notice(state: &mut AppState) {
    assert_eq!(&BROKEN[SELECTED_FROM - 1..SELECTED_FROM], "\n");
    let (stopped, _) = ShellState::default()
        .edited(BROKEN.into(), Some(SELECTED_FROM..BROKEN.len()))
        .run_requested();
    shell_tab(state, stopped);
}

/// An empty Shell tab; [`type_completion`] then opens the list.
pub fn empty(state: &mut AppState) {
    shell_tab(state, ShellState::default());
}

/// Focuses the editor and types `db.` one key at a time, so the list of collections opens.
pub fn type_completion(shot: &mut Shot) {
    let label = I18n::english().t("shell.editor_label");
    shot.harness
        .get_by_role_and_label(Role::MultilineTextInput, &label)
        .focus();
    shot.harness.run();
    for c in "db.".chars() {
        shot.harness
            .input_mut()
            .events
            .push(egui::Event::Text(c.to_string()));
        shot.harness.run();
    }
}
