//! The index-miss guard's notice under the find bar in the accesskit tree, rendered with the REAL
//! English locale: the checking line has no buttons, the warning is announced as an alert, Run
//! anyway and Cancel are named and work from the keyboard, and Cancel (the safe one) has the
//! focus. Fast, offline.
use curlybrace_core::explain::{Reason, SuggestedIndex, VerdictKind};
use curlybrace_core::guard::{GuardFinding, GuardReport};
use curlybrace_core::store::{Namespace, Query};
use curlybrace_ui::I18n;
use curlybrace_ui::app::GuardState;
use curlybrace_ui::views::guard_notice::{GuardNoticeAction, show_guard_notice};
use eframe::egui;
use eframe::egui::Key;
use eframe::egui::accesskit::{Live, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const COMMAND: &str =
    "db.getSiblingDB(\"shop\").getCollection(\"orders\").createIndex({ \"status\": 1 })";

struct Frame {
    guard: GuardState,
    seen: Vec<GuardNoticeAction>,
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn finding(kind: VerdictKind) -> GuardFinding {
    GuardFinding {
        ns: Namespace::new("shop", "orders"),
        method: "find".into(),
        query: Query::default(),
        kind,
        reasons: vec![Reason::new("collscan")],
        suggestion: Some(SuggestedIndex {
            keys: vec![("status".into(), 1)],
        }),
    }
}

fn warn(kind: VerdictKind) -> GuardState {
    GuardState::Warn {
        query: Query::default(),
        report: GuardReport {
            findings: vec![finding(kind)],
            unchecked: 0,
        },
    }
}

fn checking() -> GuardState {
    GuardState::Checking {
        request: 1,
        query: Query::default(),
    }
}

fn open(guard: GuardState) -> Harness<'static, Frame> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 400.0))
        .build_ui_state(
            |ui, frame: &mut Frame| {
                let action = show_guard_notice(ui, &frame.guard, &I18n::english());
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

fn texts(harness: &Harness<'static, Frame>) -> Vec<String> {
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
        .collect()
}

fn assert_shows(harness: &Harness<'static, Frame>, expected: &[&str]) {
    let all = texts(harness).join("\n");
    assert!(!all.contains('\u{27e6}'), "missing key marker in:\n{all}");
    for want in expected {
        assert!(all.contains(want), "`{want}` not in:\n{all}");
    }
}

fn press(harness: &mut Harness<'static, Frame>, key: Key) {
    harness.key_press(key);
    harness.run_steps(3);
}

fn focus(harness: &mut Harness<'static, Frame>, label: &str) {
    harness.get_by_role_and_label(Role::Button, label).focus();
    harness.run_steps(2);
}

fn focused(harness: &Harness<'static, Frame>, label: &str) -> bool {
    harness
        .get_by_role_and_label(Role::Button, label)
        .accesskit_node()
        .is_focused()
}

fn run_anyway() -> String {
    t("query.guard.run_anyway")
}

fn cancel() -> String {
    t("query.guard.cancel")
}

#[test]
fn nothing_is_drawn_without_a_guard_state() {
    let harness = open(GuardState::None);
    assert!(harness.query_by_role(Role::Button).is_none());
    assert!(harness.query_by_role(Role::Alert).is_none());
    assert!(harness.query_by_role(Role::Status).is_none());
}

#[test]
fn checking_is_a_polite_status_line_without_buttons() {
    let harness = open(checking());
    assert_shows(&harness, &["Checking how the query runs\u{2026}"]);
    let status = harness.get_by_role(Role::Status);
    assert_eq!(status.accesskit_node().live(), Live::Polite);
    assert!(harness.query_by_role(Role::Button).is_none());
}

#[test]
fn a_scan_is_announced_as_an_alert_with_its_problem_reasons_and_suggestion() {
    let harness = open(warn(VerdictKind::CollectionScan));
    let alert = harness.get_by_role(Role::Alert);
    assert_eq!(alert.accesskit_node().live(), Live::Assertive);
    assert_shows(
        &harness,
        &[
            "This query would read every document in the collection",
            "reads every document in the collection",
            "Suggested index",
            COMMAND,
        ],
    );
}

#[test]
fn a_blocking_sort_says_it_would_sort_in_memory() {
    let harness = open(warn(VerdictKind::SortNotCovered));
    assert_shows(
        &harness,
        &["This query would sort in memory instead of using an index"],
    );
}

#[test]
fn both_buttons_and_the_copy_button_are_named() {
    let harness = open(warn(VerdictKind::CollectionScan));
    for label in [run_anyway(), cancel(), t("query.verdict.copy")] {
        harness.get_by_role_and_label(Role::Button, &label);
    }
    assert_eq!(run_anyway(), "Run anyway");
    assert_eq!(cancel(), "Cancel");
}

#[test]
fn the_copy_button_reports_the_command() {
    let mut harness = open(warn(VerdictKind::CollectionScan));
    harness
        .get_by_role_and_label(Role::Button, &t("query.verdict.copy"))
        .click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().seen,
        vec![GuardNoticeAction::Copy(COMMAND.into())]
    );
}

#[test]
fn cancel_has_the_first_focus_so_a_stray_enter_never_runs_the_query() {
    let mut harness = open(warn(VerdictKind::CollectionScan));
    assert!(focused(&harness, &cancel()));
    assert!(!focused(&harness, &run_anyway()));
    press(&mut harness, Key::Enter);
    assert_eq!(harness.state().seen, vec![GuardNoticeAction::Cancel]);
}

#[test]
fn run_anyway_works_from_the_keyboard() {
    for key in [Key::Enter, Key::Space] {
        let mut harness = open(warn(VerdictKind::CollectionScan));
        focus(&mut harness, &run_anyway());
        press(&mut harness, key);
        assert_eq!(
            harness.state().seen,
            vec![GuardNoticeAction::RunAnyway],
            "{key:?}"
        );
    }
}

#[test]
fn tab_reaches_cancel_from_run_anyway_and_space_activates_it() {
    let mut harness = open(warn(VerdictKind::CollectionScan));
    focus(&mut harness, &run_anyway());
    press(&mut harness, Key::Tab);
    assert!(focused(&harness, &cancel()));
    press(&mut harness, Key::Space);
    assert_eq!(harness.state().seen, vec![GuardNoticeAction::Cancel]);
}

#[test]
fn clicking_the_buttons_reports_the_actions() {
    let mut harness = open(warn(VerdictKind::CollectionScan));
    harness
        .get_by_role_and_label(Role::Button, &run_anyway())
        .click();
    harness.run_steps(3);
    harness
        .get_by_role_and_label(Role::Button, &cancel())
        .click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().seen,
        vec![GuardNoticeAction::RunAnyway, GuardNoticeAction::Cancel,]
    );
}
