//! The Shell tab's index-miss guard in the accesskit tree, with the REAL English locale: the
//! checking line is a polite status beside an enabled Cancel, the notice is an alert whose Run
//! anyway and Cancel are named, work from the keyboard and emit the right `ShellAction`, Cancel
//! has the first focus, and the "could not be checked" line reads right for 1 and for 3.
//! Fast, offline, no GPU.
use curlybrace_core::explain::{Reason, SuggestedIndex, VerdictKind};
use curlybrace_core::guard::{GuardFinding, GuardReport};
use curlybrace_core::store::{Namespace, Query};
use curlybrace_ui::I18n;
use curlybrace_ui::app::shell_actions::ShellAction;
use curlybrace_ui::viewmodel::ConnectionBadge;
use curlybrace_ui::viewmodel::shell::{ShellNotice, ShellState};
use curlybrace_ui::views::guard_notice::GuardNoticeAction;
use curlybrace_ui::views::shell::{ShellViewContext, ShellViewEvent, show_shell};
use curlybrace_ui::views::shell_guard::show_index_miss;
use eframe::egui;
use eframe::egui::Key;
use eframe::egui::accesskit::{Live, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use uuid::Uuid;

const SESSION: Uuid = Uuid::from_u128(12);
const COMMAND: &str =
    "db.getSiblingDB(\"shop\").getCollection(\"orders\").createIndex({ \"status\": 1 })";

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn finding(collection: &str, kind: VerdictKind) -> GuardFinding {
    GuardFinding {
        ns: Namespace::new("shop", collection),
        query: Query::default(),
        kind,
        reasons: vec![Reason::new("collscan")],
        suggestion: Some(SuggestedIndex {
            keys: vec![("status".into(), 1)],
        }),
    }
}

fn report() -> GuardReport {
    GuardReport {
        findings: vec![
            finding("orders", VerdictKind::CollectionScan),
            finding("users", VerdictKind::SortNotCovered),
        ],
        unchecked: 0,
    }
}

// --- The notice on its own ---

struct Notice {
    unchecked: usize,
    seen: Vec<GuardNoticeAction>,
}

fn notice(unchecked: usize) -> Harness<'static, Notice> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 900.0))
        .build_ui_state(
            |ui, state: &mut Notice| {
                let action = show_index_miss(ui, &report(), state.unchecked, &I18n::english());
                state.seen.extend(action);
            },
            Notice {
                unchecked,
                seen: Vec::new(),
            },
        );
    harness.run_steps(4);
    harness
}

fn all_text<S>(harness: &Harness<'static, S>) -> String {
    harness
        .root()
        .children_recursive()
        .flat_map(|node| {
            let n = node.accesskit_node();
            [n.label(), n.value()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_shows<S>(harness: &Harness<'static, S>, expected: &[&str]) {
    let all = all_text(harness);
    assert!(!all.contains('\u{27e6}'), "missing key marker in:\n{all}");
    for want in expected {
        assert!(all.contains(want), "`{want}` not in:\n{all}");
    }
}

#[test]
fn the_notice_is_an_alert_naming_each_collection_and_problem() {
    let h = notice(0);
    let alert = h.get_by_role(Role::Alert);
    assert_eq!(alert.accesskit_node().live(), Live::Assertive);
    assert_shows(
        &h,
        &[
            &t("shell.guard.title"),
            "shop.orders: This query would read every document in the collection",
            "shop.users: This query would sort in memory instead of using an index",
            "Suggested index",
            COMMAND,
        ],
    );
}

#[test]
fn both_buttons_and_the_copy_buttons_are_named() {
    let h = notice(0);
    for key in ["shell.guard.run_anyway", "shell.guard.cancel"] {
        h.get_by_role_and_label(Role::Button, &t(key));
    }
    assert_eq!(h.get_all_by_label(&t("query.verdict.copy")).count(), 2);
}

#[test]
fn cancel_has_the_first_focus_and_enter_cancels() {
    let mut h = notice(0);
    let name = t("shell.guard.cancel");
    let cancel = h.get_by_role_and_label(Role::Button, &name);
    assert!(cancel.accesskit_node().is_focused());
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().seen, vec![GuardNoticeAction::Cancel]);
}

#[test]
fn run_anyway_works_from_enter_and_from_space() {
    for key in [Key::Enter, Key::Space] {
        let mut h = notice(0);
        h.get_by_role_and_label(Role::Button, &t("shell.guard.run_anyway"))
            .focus();
        h.run_steps(2);
        h.key_press(key);
        h.run_steps(3);
        assert_eq!(
            h.state().seen,
            vec![GuardNoticeAction::RunAnyway],
            "{key:?}"
        );
    }
}

#[test]
fn tab_reaches_cancel_from_run_anyway_and_space_cancels() {
    let mut h = notice(0);
    h.get_by_role_and_label(Role::Button, &t("shell.guard.run_anyway"))
        .focus();
    h.run_steps(2);
    h.key_press(Key::Tab);
    h.run_steps(3);
    let name = t("shell.guard.cancel");
    let cancel = h.get_by_role_and_label(Role::Button, &name);
    assert!(cancel.accesskit_node().is_focused());
    h.key_press(Key::Space);
    h.run_steps(3);
    assert_eq!(h.state().seen, vec![GuardNoticeAction::Cancel]);
}

#[test]
fn the_copy_button_reports_the_command() {
    let mut h = notice(0);
    h.get_all_by_label(&t("query.verdict.copy"))
        .next()
        .expect("a copy button")
        .click();
    h.run_steps(3);
    assert_eq!(
        h.state().seen,
        vec![GuardNoticeAction::Copy(COMMAND.to_string())]
    );
}

#[test]
fn the_unchecked_line_is_singular_for_one_and_plural_for_three() {
    assert_shows(&notice(1), &["1 call could not be checked"]);
    assert_shows(&notice(3), &["3 calls could not be checked"]);
    let none = all_text(&notice(0));
    assert!(!none.contains("could not be checked"), "{none}");
}

// --- In the Shell tab ---

struct Frame {
    shell: ShellState,
    actions: Vec<ShellAction>,
}

fn tab(shell: ShellState) -> Harness<'static, Frame> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui_state(
            |ui, frame: &mut Frame| {
                let ctx = ShellViewContext {
                    session: SESSION,
                    connection: "local",
                    badge: ConnectionBadge::default(),
                    connected: true,
                    now_secs: 0,
                    collections: &[],
                };
                for event in show_shell(ui, &frame.shell, ctx, &I18n::english()) {
                    if let ShellViewEvent::Action(action) = event {
                        frame.actions.push(action);
                    }
                }
            },
            Frame {
                shell,
                actions: Vec::new(),
            },
        );
    harness.run_steps(4);
    harness
}

fn script() -> ShellState {
    ShellState::default().edited("db.orders.find({ status: 1 })".to_string(), None)
}

fn button_disabled(h: &Harness<'static, Frame>, label: &str) -> bool {
    h.get_by_role_and_label(Role::Button, label)
        .accesskit_node()
        .is_disabled()
}

#[test]
fn checking_is_a_polite_status_beside_an_enabled_cancel_and_a_disabled_run() {
    let mut h = tab(script().checking(1, None, 0));
    let line = t("shell.guard.checking");
    assert_eq!(line, "Checking the queries in the script\u{2026}");
    let status = h.get_by_role_and_label(Role::Status, &line);
    assert_eq!(status.accesskit_node().live(), Live::Polite);
    assert!(button_disabled(&h, &t("shell.run")));
    assert!(!button_disabled(&h, &t("shell.cancel")));
    assert!(h.query_by_label(&t("shell.status_running")).is_none());
    assert!(
        h.query_by_role_and_label(Role::Button, &t("shell.guard.run_anyway"))
            .is_none()
    );
    h.get_by_role_and_label(Role::Button, &t("shell.cancel"))
        .click();
    h.run_steps(2);
    assert_eq!(h.state().actions, vec![ShellAction::Cancel]);
}

fn missed() -> ShellState {
    // One call the reader could not read, plus the report's own count: 2 in all.
    let report = GuardReport {
        unchecked: 1,
        ..report()
    };
    script().checking(1, None, 1).index_missed(report)
}

#[test]
fn the_tab_shows_the_notice_and_its_buttons_emit_the_actions() {
    let mut h = tab(missed());
    h.get_by_role(Role::Alert);
    assert_shows(&h, &["2 calls could not be checked"]);
    h.get_by_role_and_label(Role::Button, &t("shell.guard.run_anyway"))
        .click();
    h.run_steps(2);
    // The toolbar's own Cancel is off (nothing runs); the notice's is the one that works.
    h.get_all_by_role_and_label(Role::Button, &t("shell.guard.cancel"))
        .find(|node| !node.accesskit_node().is_disabled())
        .expect("the notice's Cancel")
        .click();
    h.run_steps(2);
    assert_eq!(
        h.state().actions,
        vec![ShellAction::RunAnyway, ShellAction::Cancel]
    );
}

#[test]
fn a_cleared_run_that_was_not_started_says_so_in_words() {
    let shell = ShellState {
        notice: Some(ShellNotice::GuardNotStarted),
        ..script()
    };
    let h = tab(shell);
    assert_shows(
        &h,
        &[
            "A confirmation is waiting, so the script was not started. Press Run again after answering it.",
        ],
    );
}
