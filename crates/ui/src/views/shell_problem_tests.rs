use super::*;
use crate::viewmodel::ConnectionBadge;
use crate::viewmodel::shell::ShellState;
use crate::viewmodel::shell_check::ProblemKind;
use crate::views::shell::{ShellViewContext, show_shell};
use crate::views::shell_editor::editor_id;
use crate::views::shell_notice::problem_text;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use uuid::Uuid;

const A: Uuid = Uuid::from_u128(0xA);

/// The Shell tab drawn around `shell`.
fn tab(shell: ShellState) -> Harness<'static, ShellState> {
    let i18n = I18n::english();
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui_state(
            move |ui, shell: &mut ShellState| {
                let ctx = ShellViewContext {
                    session: A,
                    connection: "local",
                    badge: ConnectionBadge::default(),
                    connected: true,
                    now_secs: 0,
                    collections: &[],
                };
                let _ = show_shell(ui, shell, ctx, &i18n);
            },
            shell,
        );
    h.run_steps(3);
    h
}

fn line_for(line: usize, column: usize, kind: ProblemKind) -> String {
    let i18n = I18n::english();
    let found = SyntaxProblem {
        kind,
        range: 0..1,
        line,
        column,
    };
    i18n.t_with(
        "shell.check.live",
        &[("problem", &problem_text(&found, &i18n))],
    )
}

#[test]
fn the_line_reads_the_problem_sentence_of_the_notice_without_saying_nothing_ran() {
    let i18n = I18n::english();
    let found = SyntaxProblem {
        kind: ProblemKind::UnterminatedString,
        range: 3..4,
        line: 2,
        column: 4,
    };
    let text = line_text(&found, &i18n);
    assert!(text.contains(&problem_text(&found, &i18n)), "{text}");
    assert!(!text.contains(&i18n.t_with("shell.check.not_run", &[("problem", "")])));
}

#[test]
fn under_the_editor_the_first_problem_of_the_whole_script_is_shown_with_its_place() {
    // The selection (`db`) is fine; the line still judges the whole editor.
    let script = "db.a.find({ x: 1 }\n).limit(]";
    let shell = ShellState::default().edited(script.into(), Some(0..2));
    let h = tab(shell);
    let expected = line_for(
        2,
        9,
        ProblemKind::MismatchedClosing {
            opener: '(',
            found: ']',
        },
    );
    let line = h.get_by_label(&expected);
    let editor_label = I18n::english().t("shell.editor_label");
    let editor = h.get_by_role_and_label(Role::MultilineTextInput, &editor_label);
    assert!(
        line.rect().top() >= editor.rect().bottom(),
        "under the editor"
    );
    // It is information only: nothing to press.
    assert_eq!(line.accesskit_node().role(), Role::Label);
}

#[test]
fn a_good_script_shows_no_line() {
    let h = tab(ShellState::default().edited("db.a.find({ x: 1 })".into(), None));
    let lead = I18n::english().t_with("shell.check.live", &[("problem", "")]);
    assert!(
        h.query_by_label_contains(lead.trim()).is_none(),
        "no problem line for a good script"
    );
}

#[test]
fn the_line_follows_the_text_as_it_is_fixed() {
    let mut h = tab(ShellState::default().edited("print((1)".into(), None));
    h.get_by_label(&line_for(1, 6, ProblemKind::UnclosedBracket('(')));
    *h.state_mut() = h.state().edited("print((1))".into(), None);
    h.run_steps(2);
    let lead = I18n::english().t_with("shell.check.live", &[("problem", "")]);
    assert!(h.query_by_label_contains(lead.trim()).is_none());
}

#[test]
fn the_problem_is_checked_once_per_script_and_again_when_it_changes() {
    let ctx = egui::Context::default();
    let id = editor_id(A);
    let script = "f(";
    let first = cached_problem(&ctx, id, script);
    assert_eq!(
        first.as_ref().map(|p| p.kind),
        Some(ProblemKind::UnclosedBracket('('))
    );
    // A planted answer for the same script is served from the cache, so nothing re-scans.
    let planted = SyntaxProblem {
        kind: ProblemKind::UnterminatedComment,
        range: 0..1,
        line: 9,
        column: 9,
    };
    plant(&ctx, id, script, Some(planted.clone()));
    assert_eq!(cached_problem(&ctx, id, script), Some(planted));
    // Another script is checked afresh.
    assert_eq!(cached_problem(&ctx, id, "f()"), None);
}

/// Line 1 has a stray `)`; line 2 (selected) never closes its `(`.
const TWO_PROBLEMS: &str = "db.a.find({}))\ndb.b.find({ x: 1 }";

#[test]
fn the_line_does_not_repeat_the_problem_the_notice_names() {
    let (stopped, _) = ShellState::default()
        .edited("db.a.find(".into(), None)
        .run_requested();
    let h = tab(stopped);
    let lead = I18n::english().t_with("shell.check.live", &[("problem", "")]);
    assert!(h.query_by_label_contains(lead.trim()).is_none());
}

#[test]
fn a_different_problem_than_the_notice_names_is_still_shown() {
    let selected = 15..TWO_PROBLEMS.len();
    let (stopped, _) = ShellState::default()
        .edited(TWO_PROBLEMS.into(), Some(selected))
        .run_requested();
    let h = tab(stopped);
    h.get_by_label(&line_for(1, 14, ProblemKind::UnexpectedClosing(')')));
}

fn plant(ctx: &egui::Context, id: egui::Id, script: &str, problem: Option<SyntaxProblem>) {
    let cached = CachedProblem {
        hash: egui::util::hash(script),
        problem,
    };
    ctx.data_mut(|d| d.insert_temp(id.with(SLOT), cached));
}
