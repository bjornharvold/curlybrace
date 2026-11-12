//! The sentence the Shell tab shows for its notice: why a run was refused, how it ended, what
//! is wrong with `mongosh`, or where the script is visibly broken. Pure: the words come from
//! `I18n`, the severity picks the tone.
use crate::I18n;
use crate::theme::Tone;
use crate::viewmodel::shell::{ShellNotice, ToolProblem};
use crate::viewmodel::shell_check::{ProblemKind, SyntaxProblem};
use crate::viewmodel::tool_rows::version_text;
use curlybrace_core::runtime::ShellRefusal;
use curlybrace_core::shell::{Outcome, ShellError};
use curlybrace_core::tools::ServerVersion;

/// How a notice reads: it is told apart by its words; the tone only backs them up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Success,
    Warning,
    Error,
}

impl Severity {
    pub fn tone(self) -> Tone {
        match self {
            Self::Info => Tone::Neutral,
            Self::Success => Tone::Good,
            Self::Warning => Tone::Warning,
            Self::Error => Tone::Bad,
        }
    }
}

/// The severity and the translated sentence of `notice`.
pub fn notice_text(notice: &ShellNotice, i18n: &I18n) -> (Severity, String) {
    match notice {
        ShellNotice::Finished(outcome) => outcome_text(*outcome, i18n),
        ShellNotice::Refused(refusal) => refusal_text(refusal, i18n),
        ShellNotice::ToolProblem(problem) => (Severity::Error, tool_text(problem, i18n)),
        ShellNotice::EmptyScript => (Severity::Warning, i18n.t("shell.notice.empty_script")),
        ShellNotice::SyntaxProblem { problem, .. } => {
            (Severity::Warning, not_run_text(problem, i18n))
        }
        // The index notice is not drawn as a sentence of its own yet: no words.
        ShellNotice::IndexMiss { .. } => (Severity::Warning, String::new()),
    }
}

/// Whether the notice is a syntax problem, so Go to problem and Run anyway are the ways out.
pub fn offers_problem_actions(notice: &ShellNotice) -> bool {
    matches!(notice, ShellNotice::SyntaxProblem { .. })
}

/// The notice of a Run the check stopped: that nothing ran, then the problem.
fn not_run_text(problem: &SyntaxProblem, i18n: &I18n) -> String {
    let problem = problem_text(problem, i18n);
    i18n.t_with("shell.check.not_run", &[("problem", &problem)])
}

/// The problem and where it is (line and column of the whole script, 1-based). The notice and
/// the problem line under the editor both read it.
pub(crate) fn problem_text(problem: &SyntaxProblem, i18n: &I18n) -> String {
    let (line, column) = (problem.line.to_string(), problem.column.to_string());
    let (key, bracket, opener) = match problem.kind {
        ProblemKind::UnclosedBracket(c) => ("shell.check.unclosed_bracket", Some(c), None),
        ProblemKind::UnexpectedClosing(c) => ("shell.check.unexpected_closing", Some(c), None),
        ProblemKind::MismatchedClosing { opener, found } => {
            ("shell.check.mismatched_closing", Some(found), Some(opener))
        }
        ProblemKind::UnterminatedString => ("shell.check.unterminated_string", None, None),
        ProblemKind::UnterminatedTemplate => ("shell.check.unterminated_template", None, None),
        ProblemKind::UnterminatedComment => ("shell.check.unterminated_comment", None, None),
    };
    let (bracket, opener) = (char_text(bracket), char_text(opener));
    i18n.t_with(
        key,
        &[
            ("line", &line),
            ("column", &column),
            ("bracket", &bracket),
            ("opener", &opener),
        ],
    )
}

fn char_text(c: Option<char>) -> String {
    c.map(String::from).unwrap_or_default()
}

/// Whether the notice is about the `mongosh` file itself (missing or too old), so the Tool
/// paths dialog is the way out. A failed start is not offered it: the cause may as well be the
/// saved password, which the dialog cannot fix.
pub fn offers_tool_paths(notice: &ShellNotice) -> bool {
    matches!(notice, ShellNotice::ToolProblem(_))
}

fn outcome_text(outcome: Outcome, i18n: &I18n) -> (Severity, String) {
    let (severity, key) = match outcome {
        Outcome::Exited(0) => (Severity::Success, "shell.notice.finished_success"),
        Outcome::Exited(code) => {
            let code = code.to_string();
            let text = i18n.t_with("shell.notice.finished_exit_code", &[("code", &code)]);
            return (Severity::Error, text);
        }
        Outcome::ScriptErrors => (Severity::Error, "shell.notice.finished_script_errors"),
        Outcome::Incomplete => (Severity::Error, "shell.notice.finished_incomplete"),
        Outcome::ConnectFailed => (Severity::Error, "shell.notice.finished_connect_failed"),
        Outcome::Signalled => (Severity::Error, "shell.notice.finished_signalled"),
        Outcome::Cancelled => (Severity::Info, "shell.notice.finished_cancelled"),
        Outcome::CapReached => (Severity::Warning, "shell.notice.finished_cap_reached"),
        Outcome::StartFailed => (Severity::Error, "shell.notice.finished_start_failed"),
    };
    (severity, i18n.t(key))
}

fn refusal_text(refusal: &ShellRefusal, i18n: &I18n) -> (Severity, String) {
    let key = match refusal {
        ShellRefusal::ReadOnly => "shell.notice.refused_read_only",
        ShellRefusal::NotConnected => "shell.notice.refused_not_connected",
        ShellRefusal::AlreadyRunning => "shell.notice.refused_already_running",
        ShellRefusal::Invalid(ShellError::Uri(problem)) => {
            let reason = i18n.t(problem.i18n_key());
            let text = i18n.t_with("shell.notice.refused_uri", &[("reason", &reason)]);
            return (Severity::Error, text);
        }
        ShellRefusal::Invalid(ShellError::SrvWithTunnel) => "shell.notice.refused_srv_with_tunnel",
        ShellRefusal::Invalid(ShellError::UnverifiableTls) => {
            "shell.notice.refused_unverifiable_tls"
        }
        ShellRefusal::Invalid(ShellError::EmptyScript) => "shell.notice.empty_script",
    };
    (Severity::Warning, i18n.t(key))
}

fn tool_text(problem: &ToolProblem, i18n: &I18n) -> String {
    match problem {
        ToolProblem::Missing(error) => {
            let reason = i18n.t(error.i18n_key());
            i18n.t_with("shell.notice.tool_missing", &[("reason", &reason)])
        }
        ToolProblem::TooOld { tool, server } => i18n.t_with(
            "shell.notice.tool_too_old",
            &[
                ("tool", &version_text(*tool)),
                ("server", &server_version(*server)),
            ],
        ),
    }
}

fn server_version(v: ServerVersion) -> String {
    format!("{}.{}", v.major, v.minor)
}

#[cfg(test)]
#[path = "shell_notice_tests.rs"]
mod tests;
