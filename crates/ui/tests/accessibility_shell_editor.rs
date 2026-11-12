//! The Shell editor's helpers in the accesskit tree: the completion list (announced, picked from
//! the keyboard) and the syntax check (the problem line under the editor, the notice of a stopped
//! Run with Go to problem and Run anyway). Every control is named. Fast, offline, no GPU.
use curlybrace_ui::I18n;
use curlybrace_ui::app::shell_actions::ShellAction;
use curlybrace_ui::viewmodel::ConnectionBadge;
use curlybrace_ui::viewmodel::shell::ShellState;
use curlybrace_ui::views::shell::{ShellViewContext, ShellViewEvent, show_shell};
use eframe::egui;
use eframe::egui::accesskit::Role;
use eframe::egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use uuid::Uuid;

const SESSION: Uuid = Uuid::from_u128(8);

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn tw(key: &str, params: &[(&str, &str)]) -> String {
    I18n::english().t_with(key, params)
}

struct Frame {
    shell: ShellState,
    /// Every action, in order.
    actions: Vec<ShellAction>,
    /// The loaded collection names the completion offers.
    collections: Vec<String>,
}

impl Frame {
    fn new(shell: ShellState) -> Self {
        Self {
            shell,
            actions: Vec::new(),
            collections: Vec::new(),
        }
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
                    session: SESSION,
                    connection: "local",
                    badge: ConnectionBadge::default(),
                    connected: true,
                    now_secs: 0,
                    collections: &frame.collections,
                };
                for event in show_shell(ui, &frame.shell, ctx, &i18n) {
                    if let ShellViewEvent::Action(action) = event {
                        if let ShellAction::Edit { script, selection } = &action {
                            frame.shell = frame.shell.edited(script.clone(), selection.clone());
                        }
                        frame.actions.push(action);
                    }
                }
            },
            frame,
        );
    harness.run_steps(3);
    harness
}

fn script(text: &str) -> ShellState {
    ShellState::default().edited(text.to_string(), None)
}

fn disabled(h: &Harness<'static, Frame>, label: &str) -> bool {
    h.get_by_role_and_label(Role::Button, label)
        .accesskit_node()
        .is_disabled()
}

fn ran(h: &Harness<'static, Frame>, action: &ShellAction) -> bool {
    h.state().actions.contains(action)
}

fn focus_editor(h: &mut Harness<'static, Frame>) {
    h.get_by_role_and_label(Role::MultilineTextInput, &t("shell.editor_label"))
        .focus();
    h.run_steps(2);
}

fn press(h: &mut Harness<'static, Frame>, (modifiers, key): (Modifiers, Key)) {
    h.key_press_modifiers(modifiers, key);
    h.run_steps(2);
}

// --- The completion list ---

fn with_collections(names: &[&str]) -> Harness<'static, Frame> {
    let mut frame = Frame::new(script(""));
    frame.collections = names.iter().map(|s| s.to_string()).collect();
    harness(frame)
}

/// Types `text` one key at a time, like a person.
fn type_keys(h: &mut Harness<'static, Frame>, text: &str) {
    for c in text.chars() {
        h.input_mut().events.push(egui::Event::Text(c.to_string()));
        h.run_steps(2);
    }
}

#[test]
fn the_completion_list_is_announced_with_its_items_and_the_selected_one() {
    let mut h = with_collections(&["orders", "users"]);
    focus_editor(&mut h);
    type_keys(&mut h, "db.");
    h.get_by_role_and_label(Role::List, &t("shell.complete.list_label"));
    let first = h.get_by_role_and_label(Role::ListItem, "orders");
    assert_eq!(first.accesskit_node().is_selected(), Some(true));
    let second = h.get_by_role_and_label(Role::ListItem, "users");
    assert_eq!(second.accesskit_node().is_selected(), Some(false));
}

#[test]
fn a_suggestion_is_picked_from_the_keyboard_alone() {
    let mut h = with_collections(&["orders", "users"]);
    focus_editor(&mut h);
    type_keys(&mut h, "db.u");
    press(&mut h, (Modifiers::NONE, Key::Enter));
    assert_eq!(h.state().shell.script, "db.users");
    assert!(
        h.query_by_role_and_label(Role::List, &t("shell.complete.list_label"))
            .is_none()
    );
    // With the list closed, Enter is a newline again.
    press(&mut h, (Modifiers::NONE, Key::Enter));
    assert_eq!(h.state().shell.script, "db.users\n");
}

// --- The syntax check ---

/// The controls a person can reach; each must carry a name of its own or one given by another
/// node.
const INTERACTIVE: [Role; 5] = [
    Role::Button,
    Role::TextInput,
    Role::MultilineTextInput,
    Role::CheckBox,
    Role::ListItem,
];

fn unlabelled(h: &Harness<'static, Frame>) -> (usize, Vec<String>) {
    let named = |text: Option<String>| text.is_some_and(|t| !t.trim().is_empty());
    let mut interactive = 0;
    let mut bad = Vec::new();
    for node in h.root().children_recursive() {
        let ak = node.accesskit_node();
        if !INTERACTIVE.contains(&ak.role()) {
            continue;
        }
        interactive += 1;
        let by_other = ak
            .labelled_by()
            .any(|l| named(l.label()) || named(l.value()));
        if !(named(ak.label()) || by_other) {
            bad.push(format!("{:?}", ak.role()));
        }
    }
    (interactive, bad)
}

/// A broken script whose Run was stopped by the check.
const BROKEN: &str = "db.users.find({ age: 1 }";

fn problem_sentence() -> String {
    // `(` of `find(` at line 1, column 14 is never closed.
    tw(
        "shell.check.unclosed_bracket",
        &[("line", "1"), ("column", "14"), ("bracket", "(")],
    )
}

#[test]
fn a_broken_script_is_described_under_the_editor_in_words() {
    let h = harness(Frame::new(script(BROKEN)));
    let text = tw("shell.check.live", &[("problem", &problem_sentence())]);
    let line = h.get_by_label(&text);
    assert_eq!(line.accesskit_node().role(), Role::Label);
    // Information only: Run stays available (the check can be wrong).
    assert!(!disabled(&h, &t("shell.run")));
    // A good script has no line.
    let good = harness(Frame::new(script("db.users.find({ age: 1 })")));
    let lead = tw("shell.check.live", &[("problem", "")]);
    assert!(good.query_by_label_contains(lead.trim()).is_none());
}

#[test]
fn the_syntax_notice_reads_out_and_offers_two_named_buttons() {
    let (stopped, _) = script(BROKEN).run_requested();
    let mut h = harness(Frame::new(stopped));
    h.get_by_label(&tw(
        "shell.check.not_run",
        &[("problem", &problem_sentence())],
    ));
    for key in ["shell.check.go_to", "shell.check.run_anyway"] {
        h.get_by_role_and_label(Role::Button, &t(key));
    }
    let (interactive, bad) = unlabelled(&h);
    // Run, Cancel, Format, the editor, History, Go to problem and Run anyway.
    assert!(interactive >= 7, "only {interactive} interactive nodes");
    assert!(bad.is_empty(), "unlabelled controls: {bad:?}");
    h.get_by_role_and_label(Role::Button, &t("shell.check.go_to"))
        .click();
    h.run_steps(2);
    assert!(ran(&h, &ShellAction::GoToProblem));
    h.get_by_role_and_label(Role::Button, &t("shell.check.run_anyway"))
        .click();
    h.run_steps(2);
    assert!(ran(&h, &ShellAction::RunAnyway));
}

#[test]
fn the_open_completion_list_leaves_nothing_unlabelled() {
    let mut h = with_collections(&["orders", "users"]);
    focus_editor(&mut h);
    type_keys(&mut h, "db.");
    h.get_by_role_and_label(Role::List, &t("shell.complete.list_label"));
    let (interactive, bad) = unlabelled(&h);
    assert!(interactive >= 6, "only {interactive} interactive nodes");
    assert!(bad.is_empty(), "unlabelled controls: {bad:?}");
}

#[test]
fn the_editor_hint_names_the_completion_shortcut() {
    let h = harness(Frame::new(script("db.users.find({})")));
    let script_hint = t("shell.script_hint");
    let hints = h.get_by_label_contains(&script_hint);
    let value = hints.accesskit_node().value().unwrap_or_default();
    assert!(value.starts_with(&t("shell.editor_hint")), "{value}");
    assert!(t("shell.editor_hint").contains("Ctrl+Space"), "{value}");
}
