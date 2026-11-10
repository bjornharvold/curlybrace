//! The Shell tab in the accesskit tree: the editor and the output pane are named, Run, Cancel
//! and Format are buttons whose enabled state follows the run, stderr is announced in words,
//! every notice is text, a tool problem offers the Tool paths dialog, and Run and Format work
//! from the keyboard alone. Fast, offline, no GPU.
use curlybrace_core::runtime::ShellRefusal;
use curlybrace_core::shell::{Outcome, ShellError, Stream};
use curlybrace_core::tools::{ServerVersion, ToolError, ToolVersion};
use curlybrace_ui::I18n;
use curlybrace_ui::app::shell_actions::ShellAction;
use curlybrace_ui::viewmodel::ConnectionBadge;
use curlybrace_ui::viewmodel::shell::{ShellNotice, ShellState, ToolProblem};
use curlybrace_ui::views::shell::{ShellViewContext, ShellViewEvent, show_shell};
use eframe::egui;
use eframe::egui::accesskit::Role;
use eframe::egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn tw(key: &str, params: &[(&str, &str)]) -> String {
    I18n::english().t_with(key, params)
}

struct Frame {
    shell: ShellState,
    connected: bool,
    badge: ConnectionBadge,
    events: Vec<ShellViewEvent>,
}

impl Frame {
    fn new(shell: ShellState) -> Self {
        Self {
            shell,
            connected: true,
            badge: ConnectionBadge::default(),
            events: Vec::new(),
        }
    }

    fn actions(&self) -> Vec<ShellAction> {
        self.events
            .iter()
            .filter_map(|e| match e {
                ShellViewEvent::Action(a) => Some(a.clone()),
                ShellViewEvent::OpenToolPaths => None,
            })
            .collect()
    }
}

/// Draws the tab; edits are applied back to the shell like the reducer would.
fn harness(frame: Frame) -> Harness<'static, Frame> {
    let i18n = I18n::english();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui_state(
            move |ui, frame: &mut Frame| {
                let ctx = ShellViewContext {
                    connection: "local",
                    badge: frame.badge,
                    connected: frame.connected,
                };
                let Some(event) = show_shell(ui, &frame.shell, ctx, &i18n) else {
                    return;
                };
                if let ShellViewEvent::Action(ShellAction::Edit { script, selection }) = &event {
                    frame.shell = frame.shell.edited(script.clone(), selection.clone());
                }
                frame.events.push(event);
            },
            frame,
        );
    harness.run_steps(3);
    harness
}

fn script(text: &str) -> ShellState {
    ShellState::default().edited(text.to_string(), None)
}

fn running(text: &str) -> ShellState {
    script(text).run_requested().0
}

fn disabled(h: &Harness<'static, Frame>, label: &str) -> bool {
    h.get_by_role_and_label(Role::Button, label)
        .accesskit_node()
        .is_disabled()
}

#[test]
fn the_editor_has_an_accessible_name() {
    let h = harness(Frame::new(script("db.users.find({})")));
    h.get_by_role_and_label(Role::MultilineTextInput, &t("shell.editor_label"));
}

#[test]
fn run_cancel_and_format_are_named_buttons() {
    let h = harness(Frame::new(script("db.users.find({})")));
    for key in ["shell.run", "shell.cancel", "shell.format"] {
        h.get_by_role_and_label(Role::Button, &t(key));
    }
}

#[test]
fn cancel_is_disabled_while_idle_and_run_while_running() {
    let idle = harness(Frame::new(script("db.users.find({})")));
    assert!(disabled(&idle, &t("shell.cancel")));
    assert!(!disabled(&idle, &t("shell.run")));
    let busy = harness(Frame::new(running("db.users.find({})")));
    assert!(disabled(&busy, &t("shell.run")));
    assert!(!disabled(&busy, &t("shell.cancel")));
}

#[test]
fn run_is_disabled_for_an_empty_script_a_lost_connection_and_a_read_only_one() {
    let empty = harness(Frame::new(script("   ")));
    assert!(disabled(&empty, &t("shell.run")));

    let mut offline = Frame::new(script("db.users.find({})"));
    offline.connected = false;
    let offline = harness(offline);
    assert!(disabled(&offline, &t("shell.run")));
    offline.get_by_label(&t("shell.not_connected_note"));

    let mut locked = Frame::new(script("db.users.find({})"));
    locked.badge = ConnectionBadge::default().with_read_only(true);
    let locked = harness(locked);
    assert!(disabled(&locked, &t("shell.run")));
    locked.get_by_label(&t("shell.read_only_note"));
}

#[test]
fn a_production_connection_says_every_run_asks_first() {
    let mut frame = Frame::new(script("db.users.find({})"));
    frame.badge = ConnectionBadge::new(None, true);
    let h = harness(frame);
    h.get_by_label(&t("shell.production_note"));
}

#[test]
fn clicking_run_and_cancel_reports_them() {
    let mut h = harness(Frame::new(script("db.users.find({})")));
    h.get_by_role_and_label(Role::Button, &t("shell.run"))
        .click();
    h.run_steps(2);
    assert!(h.state().actions().contains(&ShellAction::Run));

    let mut busy = harness(Frame::new(running("db.users.find({})")));
    busy.get_by_role_and_label(Role::Button, &t("shell.cancel"))
        .click();
    busy.run_steps(2);
    assert!(busy.state().actions().contains(&ShellAction::Cancel));
}

#[test]
fn the_output_pane_is_a_named_region() {
    let h = harness(Frame::new(script("db.users.find({})")));
    h.get_by_role_and_label(Role::Region, &t("shell.output.region"));
    h.get_by_label(&t("shell.output.empty"));
}

#[test]
fn stderr_lines_are_announced_as_errors_in_words() {
    let shell = running("print(1)")
        .output_arrived(1, Stream::Stdout, "all good\n")
        .output_arrived(1, Stream::Stderr, "boom\n");
    let h = harness(Frame::new(shell));
    h.get_by_label("all good");
    h.get_by_label(&tw("shell.output.stderr_line", &[("text", "boom")]));
}

#[test]
fn dropped_lines_are_counted_in_words() {
    let shell = ShellState {
        dropped_lines: 42,
        ..running("print(1)").output_arrived(1, Stream::Stdout, "last\n")
    };
    let h = harness(Frame::new(shell));
    h.get_by_label(&tw("shell.output.dropped", &[("count", "42")]));
}

fn with_notice(notice: ShellNotice) -> Harness<'static, Frame> {
    let shell = ShellState {
        notice: Some(notice),
        ..script("db.users.find({})")
    };
    harness(Frame::new(shell))
}

#[test]
fn every_notice_kind_is_shown_as_text() {
    let cases = [
        (
            ShellNotice::Refused(ShellRefusal::ReadOnly),
            t("shell.notice.refused_read_only"),
        ),
        (
            ShellNotice::Refused(ShellRefusal::NotConnected),
            t("shell.notice.refused_not_connected"),
        ),
        (
            ShellNotice::Refused(ShellRefusal::Invalid(ShellError::SrvWithTunnel)),
            t("shell.notice.refused_srv_with_tunnel"),
        ),
        (
            ShellNotice::Finished(Outcome::Exited(3)),
            tw("shell.notice.finished_exit_code", &[("code", "3")]),
        ),
        (
            ShellNotice::Finished(Outcome::ScriptErrors),
            t("shell.notice.finished_script_errors"),
        ),
        (
            ShellNotice::Finished(Outcome::ConnectFailed),
            t("shell.notice.finished_connect_failed"),
        ),
        (ShellNotice::EmptyScript, t("shell.notice.empty_script")),
    ];
    for (notice, text) in cases {
        let h = with_notice(notice);
        h.get_by_label(&text);
    }
}

#[test]
fn a_tool_problem_is_stated_and_opens_the_tool_paths_dialog() {
    let missing = ToolProblem::Missing(ToolError::NotOnPath);
    let mut h = with_notice(ShellNotice::ToolProblem(missing));
    h.get_by_label(&tw(
        "shell.notice.tool_missing",
        &[("reason", &t(ToolError::NotOnPath.i18n_key()))],
    ));
    h.get_by_role_and_label(Role::Button, &t("shell.open_tool_paths"))
        .click();
    h.run_steps(2);
    assert!(h.state().events.contains(&ShellViewEvent::OpenToolPaths));

    let old = ToolProblem::TooOld {
        tool: ToolVersion::new(1, 2, 3),
        server: ServerVersion::new(8, 0),
    };
    let h = with_notice(ShellNotice::ToolProblem(old));
    h.get_by_label(&tw(
        "shell.notice.tool_too_old",
        &[("tool", "1.2.3"), ("server", "8.0")],
    ));
    h.get_by_role_and_label(Role::Button, &t("shell.open_tool_paths"));
}

#[test]
fn run_works_from_the_keyboard() {
    let mut h = harness(Frame::new(ShellState::default()));
    let editor = t("shell.editor_label");
    h.get_by_role_and_label(Role::MultilineTextInput, &editor)
        .focus();
    h.run_steps(2);
    h.get_by_role_and_label(Role::MultilineTextInput, &editor)
        .type_text("db.users.find({})");
    h.run_steps(2);
    assert_eq!(h.state().shell.script, "db.users.find({})");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run_steps(2);
    assert!(h.state().actions().contains(&ShellAction::Run));
    // The shortcut is not typed into the script.
    assert_eq!(h.state().shell.script, "db.users.find({})");
}

#[test]
fn format_works_from_the_keyboard() {
    let mut h = harness(Frame::new(script("if (a) {\nb()\n}")));
    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::F);
    h.run_steps(2);
    assert!(h.state().actions().contains(&ShellAction::Format));
}

#[test]
fn the_run_shortcut_does_nothing_while_run_is_disabled() {
    let mut frame = Frame::new(script("db.users.find({})"));
    frame.connected = false;
    let mut h = harness(frame);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.run_steps(2);
    assert!(!h.state().actions().contains(&ShellAction::Run));
}
