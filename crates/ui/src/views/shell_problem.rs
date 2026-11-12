//! The problem line under the Shell editor: the first problem the syntax check finds in the
//! WHOLE editor text (whatever is selected), with its line and column, or nothing for a good
//! script. It only informs: Run is not gated on it (the check can be wrong), and the notice of a
//! stopped Run says the same problem with the same words (`shell_notice::problem_text`).
//!
//! The check scans the whole script, so its answer is kept per editor and keyed by the script's
//! hash (like the highlight), and a frame that changed nothing scans nothing.
use super::shell_notice::problem_text;
use crate::I18n;
use crate::theme::{Tone, current_ui};
use crate::viewmodel::shell_check::{SyntaxProblem, check};
use eframe::egui;

/// Key of the cached answer, salted with the editor's id.
const SLOT: &str = "problem";

/// The answer of the check for the script whose hash is `hash`.
#[derive(Clone)]
struct CachedProblem {
    hash: u64,
    problem: Option<SyntaxProblem>,
}

/// The first problem of `script`, checked only when the script differs from the one last checked
/// for the editor `editor`.
pub(crate) fn cached_problem(
    ctx: &egui::Context,
    editor: egui::Id,
    script: &str,
) -> Option<SyntaxProblem> {
    let id = editor.with(SLOT);
    let hash = egui::util::hash(script);
    let hit = ctx
        .data(|d| d.get_temp::<CachedProblem>(id))
        .filter(|cached| cached.hash == hash);
    if let Some(cached) = hit {
        return cached.problem;
    }
    let problem = check(script);
    let cached = CachedProblem {
        hash,
        problem: problem.clone(),
    };
    ctx.data_mut(|d| d.insert_temp(id, cached));
    problem
}

/// What the line says about `problem`.
pub(crate) fn line_text(problem: &SyntaxProblem, i18n: &I18n) -> String {
    let problem = problem_text(problem, i18n);
    i18n.t_with("shell.check.live", &[("problem", &problem)])
}

/// Draws the line for the editor `editor` holding `script`: one small row in the warning color
/// while the script has a problem, nothing otherwise. A plain label, so screen readers read its
/// words as its name. `in_notice` is the problem the notice of a stopped Run names: the line does
/// not say it a second time (it would cost a row and repeat the same words).
pub(crate) fn show_problem_line(
    ui: &mut egui::Ui,
    editor: egui::Id,
    script: &str,
    in_notice: Option<&SyntaxProblem>,
    i18n: &I18n,
) {
    let problem = cached_problem(ui.ctx(), editor, script);
    let Some(problem) = problem.filter(|p| Some(p) != in_notice) else {
        return;
    };
    let color = current_ui(ui).tone(Tone::Warning).fg;
    let text = egui::RichText::new(line_text(&problem, i18n))
        .small()
        .color(color);
    ui.label(text);
}

#[cfg(test)]
#[path = "shell_problem_tests.rs"]
mod tests;
