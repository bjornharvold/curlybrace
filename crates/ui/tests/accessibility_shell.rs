//! The Shell tab in the accesskit tree: the editor and the output pane are named, Run, Cancel
//! and Format are buttons whose enabled state follows the run, stderr is announced in words,
//! every notice is text, a tool problem offers the Tool paths dialog, and Run and Format work
//! from the keyboard alone. Fast, offline, no GPU.
use curlybrace_core::history::{ShellHistoryEntry, ShellRunStatus};
use curlybrace_core::runtime::ShellRefusal;
use curlybrace_core::shell::{Outcome, ShellError, Stream};
use curlybrace_core::tools::{ServerVersion, ToolError, ToolVersion};
use curlybrace_ui::I18n;
use curlybrace_ui::app::shell_actions::ShellAction;
use curlybrace_ui::viewmodel::ConnectionBadge;
use curlybrace_ui::viewmodel::index_detail::Age;
use curlybrace_ui::viewmodel::shell::{ShellNotice, ShellState, ToolProblem};
use curlybrace_ui::viewmodel::shell_history::first_line;
use curlybrace_ui::views::shell::{ShellViewContext, ShellViewEvent, show_shell};
use eframe::egui;
use eframe::egui::accesskit::Role;
use eframe::egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use uuid::Uuid;

const SESSION: Uuid = Uuid::from_u128(7);
/// The frame's wall-clock time (Unix seconds).
const NOW: i64 = 1_700_000_000;

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn tw(key: &str, params: &[(&str, &str)]) -> String {
    I18n::english().t_with(key, params)
}

/// A focusable row and a text field outside the tab, standing in for the sidebar.
const ROW: &str = "Sidebar row";
const FIELD: &str = "Sidebar filter";

struct Frame {
    shell: ShellState,
    connected: bool,
    badge: ConnectionBadge,
    /// Every event, in order.
    events: Vec<ShellViewEvent>,
    /// The events of each frame that had any.
    frames: Vec<Vec<ShellViewEvent>>,
    field: String,
}

impl Frame {
    fn new(shell: ShellState) -> Self {
        Self {
            shell,
            connected: true,
            badge: ConnectionBadge::default(),
            events: Vec::new(),
            frames: Vec::new(),
            field: String::new(),
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
                ui.horizontal(|ui| {
                    let _ = ui.button(ROW);
                    ui.add(egui::TextEdit::singleline(&mut frame.field).hint_text(FIELD))
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, FIELD)
                        });
                });
                let ctx = ShellViewContext {
                    session: SESSION,
                    connection: "local",
                    badge: frame.badge,
                    connected: frame.connected,
                    now_secs: NOW,
                };
                let events = show_shell(ui, &frame.shell, ctx, &i18n);
                for event in &events {
                    if let ShellViewEvent::Action(action) = event {
                        frame.shell = applied(&frame.shell, action);
                    }
                }
                if !events.is_empty() {
                    frame.frames.push(events.clone());
                }
                frame.events.extend(events);
            },
            frame,
        );
    harness.run_steps(3);
    harness
}

/// What the reducer does to the shell for the actions that change only the shell itself (a run
/// or a clear also sends a command, which this harness has no runtime for).
fn applied(shell: &ShellState, action: &ShellAction) -> ShellState {
    match action {
        ShellAction::Edit { script, selection } => shell.edited(script.clone(), selection.clone()),
        ShellAction::ToggleHistory => shell.history_toggled(),
        ShellAction::RecallEntry(id) => shell.history_entry_recalled(*id),
        ShellAction::ClearHistory => shell.clear_requested(),
        ShellAction::CancelClear => shell.clear_cancelled(),
        ShellAction::ConfirmClear => shell.clear_confirmed().0,
        _ => shell.clone(),
    }
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
fn the_tab_says_how_a_script_runs() {
    let h = harness(Frame::new(script("db.users.find({})")));
    h.get_by_label(&t("shell.script_hint"));
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
    // The painted marker is not a second node: "error" is spoken once.
    assert!(
        h.query_by_label_contains(&t("shell.output.stderr_marker"))
            .is_none()
    );
}

#[test]
fn dropped_lines_are_counted_in_words_with_the_right_plural() {
    for (count, key) in [
        (42, "shell.output.dropped_other"),
        (1, "shell.output.dropped_one"),
    ] {
        let shell = ShellState {
            dropped_lines: count,
            ..running("print(1)").output_arrived(1, Stream::Stdout, "last\n")
        };
        let h = harness(Frame::new(shell));
        h.get_by_label(&tw(key, &[("count", &count.to_string())]));
    }
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
            ShellNotice::Finished(Outcome::Incomplete),
            t("shell.notice.finished_incomplete"),
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

const RUN: (Modifiers, Key) = (Modifiers::COMMAND, Key::Enter);
const FORMAT: (Modifiers, Key) = (Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::F);

fn focus(h: &mut Harness<'static, Frame>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).focus();
    h.run_steps(2);
}

fn focus_editor(h: &mut Harness<'static, Frame>) {
    focus(h, Role::MultilineTextInput, &t("shell.editor_label"));
}

fn press(h: &mut Harness<'static, Frame>, (modifiers, key): (Modifiers, Key)) {
    h.key_press_modifiers(modifiers, key);
    h.run_steps(2);
}

/// Text typed and a shortcut pressed within ONE frame (the harness otherwise steps per event).
fn type_and_press(h: &mut Harness<'static, Frame>, text: &str, keys: Option<(Modifiers, Key)>) {
    let input = h.input_mut();
    input.events.push(egui::Event::Text(text.into()));
    if let Some((modifiers, key)) = keys {
        input.modifiers = modifiers;
        input.events.push(egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            repeat: false,
            physical_key: None,
        });
    }
    h.step();
    h.input_mut().modifiers = Modifiers::default();
    h.run_steps(2);
}

fn ran(h: &Harness<'static, Frame>, action: &ShellAction) -> bool {
    h.state().actions().contains(action)
}

#[test]
fn run_works_from_the_keyboard() {
    let mut h = harness(Frame::new(ShellState::default()));
    let editor = t("shell.editor_label");
    focus_editor(&mut h);
    h.get_by_role_and_label(Role::MultilineTextInput, &editor)
        .type_text("db.users.find({})");
    h.run_steps(2);
    assert_eq!(h.state().shell.script, "db.users.find({})");
    press(&mut h, RUN);
    assert!(ran(&h, &ShellAction::Run));
    // The shortcut is not typed into the script.
    assert_eq!(h.state().shell.script, "db.users.find({})");
}

#[test]
fn format_works_from_the_keyboard() {
    let mut h = harness(Frame::new(script("if (a) {\nb()\n}")));
    focus_editor(&mut h);
    press(&mut h, FORMAT);
    assert!(ran(&h, &ShellAction::Format));
}

#[test]
fn the_shortcuts_work_with_nothing_focused() {
    for (keys, action) in [(RUN, ShellAction::Run), (FORMAT, ShellAction::Format)] {
        let mut h = harness(Frame::new(script("db.users.find({})")));
        press(&mut h, keys);
        assert!(ran(&h, &action), "{action:?}");
    }
}

#[test]
fn the_shortcuts_leave_a_focused_sidebar_row_or_field_alone() {
    for (role, label) in [(Role::Button, ROW), (Role::TextInput, FIELD)] {
        for keys in [RUN, FORMAT] {
            let mut h = harness(Frame::new(script("db.users.find({})")));
            focus(&mut h, role, label);
            press(&mut h, keys);
            assert!(
                h.state().actions().is_empty(),
                "{label}: {:?}",
                h.state().events
            );
        }
    }
}

#[test]
fn text_typed_in_the_frame_of_a_shortcut_is_kept_and_comes_first() {
    for (keys, action) in [(RUN, ShellAction::Run), (FORMAT, ShellAction::Format)] {
        let mut h = harness(Frame::new(script("db.a.find()")));
        focus_editor(&mut h);
        type_and_press(&mut h, ";", Some(keys));
        let edit = ShellViewEvent::Action(ShellAction::Edit {
            script: "db.a.find();".into(),
            selection: None,
        });
        let last = h.state().frames.last().cloned();
        assert_eq!(last, Some(vec![edit, ShellViewEvent::Action(action)]));
    }
}

#[test]
fn text_typed_in_the_frame_of_a_run_click_is_kept_and_comes_first() {
    let mut h = harness(Frame::new(script("db.a.find()")));
    focus_editor(&mut h);
    // An assistive-technology click on Run and a typed character arrive in the same frame.
    h.get_by_role_and_label(Role::Button, &t("shell.run"))
        .click_accesskit();
    type_and_press(&mut h, ";", None);
    let edit = ShellViewEvent::Action(ShellAction::Edit {
        script: "db.a.find();".into(),
        selection: None,
    });
    let run = ShellViewEvent::Action(ShellAction::Run);
    assert!(
        h.state().frames.contains(&vec![edit, run]),
        "{:?}",
        h.state().frames
    );
}

#[test]
fn a_run_shortcut_right_after_typing_into_an_empty_editor_runs_the_typed_text() {
    let mut h = harness(Frame::new(ShellState::default()));
    focus_editor(&mut h);
    type_and_press(&mut h, "it", Some(RUN));
    assert!(ran(&h, &ShellAction::Run));
    assert_eq!(h.state().shell.script, "it");
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

// --- The History panel ---

fn entry(id: i64, ago_secs: i64, script: &str, status: ShellRunStatus) -> ShellHistoryEntry {
    ShellHistoryEntry {
        id,
        ran_at_secs: NOW - ago_secs,
        script: script.to_string(),
        status,
    }
}

/// A shell whose history answered with `entries`, with its panel open.
fn open_history(entries: Vec<ShellHistoryEntry>) -> ShellState {
    script("db.a.find()")
        .history_loaded(entries)
        .history_toggled()
}

fn status_key(status: ShellRunStatus) -> &'static str {
    match status {
        ShellRunStatus::Succeeded => "shell.history.status.succeeded",
        ShellRunStatus::ScriptErrors => "shell.history.status.script_errors",
        ShellRunStatus::Incomplete => "shell.history.status.incomplete",
        ShellRunStatus::Cancelled => "shell.history.status.cancelled",
        ShellRunStatus::CapReached => "shell.history.status.cap_reached",
        ShellRunStatus::Failed => "shell.history.status.failed",
    }
}

/// The accessible name of `entry`'s row: its status, when it ran and its first line.
fn row_label(entry: &ShellHistoryEntry) -> String {
    let age = Age::of(NOW - entry.ran_at_secs);
    let time = tw(age.key(), &[("n", &age.count().to_string())]);
    tw(
        "shell.history.row_label",
        &[
            ("status", &t(status_key(entry.status))),
            ("time", &time),
            ("script", &first_line(&entry.script, 80)),
        ],
    )
}

fn expanded(h: &Harness<'static, Frame>, label: &str) -> Option<bool> {
    h.get_by_role_and_label(Role::Button, label)
        .accesskit_node()
        .data()
        .is_expanded()
}

fn has_button(h: &Harness<'static, Frame>, label: &str) -> bool {
    h.query_by_role_and_label(Role::Button, label).is_some()
}

/// Clicks the button and lets the layout settle: a change of what the panel shows is measured in
/// one frame and laid out in the next (see `shell_layout`), so the following click aims at where
/// the buttons now are.
fn click(h: &mut Harness<'static, Frame>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).click();
    h.run_steps(3);
}

#[test]
fn the_history_toggle_is_a_labelled_button() {
    let mut h = harness(Frame::new(script("db.a.find()").history_loaded(Vec::new())));
    assert_eq!(expanded(&h, &t("shell.history.toggle_open")), Some(false));
    assert!(h.query_by_label(&t("shell.history.empty")).is_none());
    // Drawing frames alone emits nothing.
    assert!(h.state().actions().is_empty(), "{:?}", h.state().events);

    click(&mut h, &t("shell.history.toggle_open"));
    assert_eq!(h.state().actions(), vec![ShellAction::ToggleHistory]);
    assert_eq!(expanded(&h, &t("shell.history.toggle_close")), Some(true));
    h.get_by_label(&t("shell.history.empty"));
}

#[test]
fn each_history_row_is_a_labelled_button_with_its_status() {
    let statuses = [
        ShellRunStatus::Succeeded,
        ShellRunStatus::ScriptErrors,
        ShellRunStatus::Incomplete,
        ShellRunStatus::Cancelled,
        ShellRunStatus::CapReached,
        ShellRunStatus::Failed,
    ];
    // The list is virtualised, so each status gets a list of its own next to an older run.
    for (i, status) in (0_i64..).zip(statuses) {
        let run = entry(10 + i, 30 + i * 4_000, &format!("db.c{i}.find()"), status);
        let older = entry(1, 90_000, "db.old.find()", ShellRunStatus::Succeeded);
        let h = harness(Frame::new(open_history(vec![run.clone(), older.clone()])));
        for row in [&run, &older] {
            let label = row_label(row);
            let node = h.get_by_role_and_label(Role::Button, &label);
            let name = node.accesskit_node().label().unwrap_or_default();
            assert!(name.contains(&t(status_key(row.status))), "{name}");
        }
        h.get_by_label(&t("shell.history.plaintext_note"));
        assert!(h.state().actions().is_empty(), "{:?}", h.state().events);
    }
}

#[test]
fn the_unavailable_and_empty_states_are_announced() {
    let cases = [
        (
            ShellState::default().history_toggled(),
            "shell.history.loading",
        ),
        (
            ShellState::default()
                .history_unavailable()
                .history_toggled(),
            "shell.history.unavailable",
        ),
        (open_history(Vec::new()), "shell.history.empty"),
    ];
    for (shell, key) in cases {
        let h = harness(Frame::new(shell));
        h.get_by_label(&t(key));
        assert!(!has_button(&h, &t("shell.history.clear")), "{key}");
    }
}

#[test]
fn clear_history_asks_before_it_clears() {
    let first = entry(1, 120, "db.a.find()", ShellRunStatus::Succeeded);
    let mut h = harness(Frame::new(open_history(vec![first.clone()])));
    click(&mut h, &t("shell.history.clear"));
    assert_eq!(h.state().actions(), vec![ShellAction::ClearHistory]);
    h.get_by_label(&t("shell.history.clear_confirm"));
    h.get_by_role_and_label(Role::Button, &t("shell.history.clear_confirm_yes"));
    // Nothing is cleared yet.
    h.get_by_role_and_label(Role::Button, &row_label(&first));
    assert!(!has_button(&h, &t("shell.history.clear")));

    click(&mut h, &t("shell.history.clear_cancel"));
    assert!(
        h.state().actions().ends_with(&[ShellAction::CancelClear]),
        "{:?}",
        h.state().actions()
    );
    assert!(
        h.query_by_label(&t("shell.history.clear_confirm"))
            .is_none()
    );
    assert!(!h.state().actions().contains(&ShellAction::ConfirmClear));

    click(&mut h, &t("shell.history.clear"));
    click(&mut h, &t("shell.history.clear_confirm_yes"));
    assert!(h.state().actions().ends_with(&[ShellAction::ConfirmClear]));
}

#[test]
fn clicking_a_row_emits_recall() {
    let older = entry(3, 7_200, "db.old.find()", ShellRunStatus::Failed);
    let newer = entry(4, 10, "db.new.find()", ShellRunStatus::Succeeded);
    let mut h = harness(Frame::new(open_history(vec![newer, older.clone()])));
    click(&mut h, &row_label(&older));
    assert_eq!(h.state().actions(), vec![ShellAction::RecallEntry(3)]);
    assert_eq!(h.state().shell.script, "db.old.find()");
}

#[test]
fn a_history_row_is_recalled_from_the_keyboard() {
    let only = entry(5, 10, "db.k.find()", ShellRunStatus::Cancelled);
    let mut h = harness(Frame::new(open_history(vec![only.clone()])));
    focus(&mut h, Role::Button, &row_label(&only));
    press(&mut h, (Modifiers::NONE, Key::Enter));
    assert_eq!(h.state().actions(), vec![ShellAction::RecallEntry(5)]);
}

#[test]
fn a_huge_multi_line_script_is_one_bounded_row() {
    let huge = format!("{}\n{}", "x".repeat(20_000), "db.more()\n".repeat(2_000));
    let big = entry(8, 60, &huge, ShellRunStatus::CapReached);
    let small = entry(7, 90, "db.s.find()", ShellRunStatus::Succeeded);
    let h = harness(Frame::new(open_history(vec![big.clone(), small.clone()])));
    let label = row_label(&big);
    assert!(label.chars().count() < 200, "{label}");
    let big_rect = h.get_by_role_and_label(Role::Button, &label).rect();
    let small_rect = h
        .get_by_role_and_label(Role::Button, &row_label(&small))
        .rect();
    assert_eq!(big_rect.height(), small_rect.height());
    assert!(big_rect.width() <= 900.0, "{big_rect:?}");
}
