//! The profiler consent dialog in the accesskit tree, rendered with the REAL English locale:
//! it explains what the change does, shows the exact command, defaults to Cancel, and its
//! confirm is off while a change is in flight or the connection is read-only. Fast, offline.
use curlybrace_ui::I18n;
use curlybrace_ui::viewmodel::{ProfilerConsent, QueriesAction};
use curlybrace_ui::views::queries_consent::show_profiler_consent;
use eframe::egui;
use eframe::egui::OutputCommand;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

struct Frame {
    consent: ProfilerConsent,
    busy: bool,
    read_only: bool,
    production: bool,
    seen: Vec<QueriesAction>,
    copied: Vec<String>,
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn on() -> ProfilerConsent {
    ProfilerConsent {
        db: "shop".into(),
        level: 2,
    }
}

fn open(
    consent: ProfilerConsent,
    busy: bool,
    read_only: bool,
    production: bool,
) -> Harness<'static, Frame> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .build_ui_state(
            |ui, f: &mut Frame| {
                let action = show_profiler_consent(
                    ui.ctx(),
                    &f.consent,
                    f.busy,
                    f.read_only,
                    f.production,
                    &I18n::english(),
                );
                f.seen.extend(action);
                let copies: Vec<String> = ui.ctx().output(|o| {
                    o.commands
                        .iter()
                        .filter_map(|c| match c {
                            OutputCommand::CopyText(text) => Some(text.clone()),
                            _ => None,
                        })
                        .collect()
                });
                f.copied.extend(copies);
            },
            Frame {
                consent,
                busy,
                read_only,
                production,
                seen: Vec::new(),
                copied: Vec::new(),
            },
        );
    harness.run_steps(6);
    harness
}

fn texts(harness: &Harness<'static, Frame>) -> String {
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

fn button<'a>(harness: &'a Harness<'static, Frame>, label: &'a str) -> egui_kittest::Node<'a> {
    harness.get_by_role_and_label(Role::Button, label)
}

const COMMAND_ON: &str = "db.getSiblingDB(\"shop\").setProfilingLevel(2)";
const COMMAND_OFF: &str = "db.getSiblingDB(\"shop\").setProfilingLevel(0)";

#[test]
fn turning_on_explains_the_overhead_and_shows_the_exact_command() {
    let harness = open(on(), false, false, false);
    let all = texts(&harness);
    assert!(!all.contains('\u{27e6}'), "{all}");
    for want in [
        t("indexes.queries.consent.title_on"),
        COMMAND_ON.to_string(),
        t("indexes.queries.consent.confirm_on"),
        t("common.cancel"),
    ] {
        assert!(all.contains(&want), "`{want}` not in:\n{all}");
    }
    let message = I18n::english().t_with("indexes.queries.consent.message_on", &[("db", "shop")]);
    assert!(all.contains(&message), "{all}");
    assert!(message.contains("overhead") && message.contains("capped"));
    assert!(!all.contains(&t("write_warning.production_line")));
}

#[test]
fn turning_off_has_its_own_text_and_command() {
    let off = ProfilerConsent {
        db: "shop".into(),
        level: 0,
    };
    let mut harness = open(off, false, false, false);
    let all = texts(&harness);
    for want in [
        t("indexes.queries.consent.title_off"),
        COMMAND_OFF.to_string(),
        t("indexes.queries.consent.confirm_off"),
    ] {
        assert!(all.contains(&want), "`{want}` not in:\n{all}");
    }
    button(&harness, &t("indexes.queries.consent.confirm_off")).click();
    harness.run_steps(3);
    assert_eq!(harness.state().seen, vec![QueriesAction::ConfirmDisable]);
}

#[test]
fn confirm_reports_and_cancel_reports() {
    let mut harness = open(on(), false, false, false);
    button(&harness, &t("indexes.queries.consent.confirm_on")).click();
    harness.run_steps(3);
    assert_eq!(harness.state().seen, vec![QueriesAction::ConfirmEnable]);
    button(&harness, &t("common.cancel")).click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().seen.last(),
        Some(&QueriesAction::CancelEnable)
    );
}

#[test]
fn the_command_has_a_copy_button() {
    let mut harness = open(on(), false, false, false);
    button(&harness, &t("indexes.queries.consent.copy")).click();
    harness.run_steps(3);
    assert!(harness.state().copied.iter().any(|c| c == COMMAND_ON));
}

#[test]
fn cancel_is_the_focused_default_so_a_stray_enter_changes_nothing() {
    let mut harness = open(on(), false, false, false);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(harness.state().seen, vec![QueriesAction::CancelEnable]);
}

#[test]
fn escape_cancels() {
    let mut harness = open(on(), false, false, false);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert_eq!(harness.state().seen, vec![QueriesAction::CancelEnable]);
}

#[test]
fn read_only_and_busy_turn_confirm_off_and_say_why() {
    let confirm = t("indexes.queries.consent.confirm_on");
    let read_only = open(on(), false, true, false);
    assert!(button(&read_only, &confirm).accesskit_node().is_disabled());
    assert!(texts(&read_only).contains(&t("connection.read_only_reason")));

    let busy = open(on(), true, false, false);
    assert!(button(&busy, &confirm).accesskit_node().is_disabled());
    assert!(texts(&busy).contains(&t("indexes.queries.busy")));
}

#[test]
fn production_shows_the_notice() {
    let harness = open(on(), false, false, true);
    assert!(texts(&harness).contains(&t("write_warning.production_line")));
}
