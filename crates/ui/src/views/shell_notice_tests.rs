use super::*;
use curlybrace_core::tools::{ToolError, ToolVersion};
use curlybrace_core::uri_problem::UriProblem;
use std::collections::BTreeSet;

fn all_notices() -> Vec<ShellNotice> {
    let outcomes = [
        Outcome::Exited(0),
        Outcome::Exited(2),
        Outcome::ScriptErrors,
        Outcome::Incomplete,
        Outcome::ConnectFailed,
        Outcome::Signalled,
        Outcome::Cancelled,
        Outcome::CapReached,
        Outcome::StartFailed,
    ];
    let refusals = [
        ShellRefusal::ReadOnly,
        ShellRefusal::NotConnected,
        ShellRefusal::AlreadyRunning,
        ShellRefusal::Invalid(ShellError::Uri(UriProblem::BadPort)),
        ShellRefusal::Invalid(ShellError::SrvWithTunnel),
        ShellRefusal::Invalid(ShellError::UnverifiableTls),
    ];
    let tools = [
        ToolProblem::Missing(ToolError::NotOnPath),
        ToolProblem::TooOld {
            tool: ToolVersion::new(1, 0, 2),
            server: ServerVersion::new(8, 0),
        },
    ];
    outcomes
        .into_iter()
        .map(ShellNotice::Finished)
        .chain(refusals.into_iter().map(ShellNotice::Refused))
        .chain(tools.into_iter().map(ShellNotice::ToolProblem))
        .chain([ShellNotice::EmptyScript])
        .collect()
}

#[test]
fn every_notice_has_its_own_translated_sentence() {
    let i18n = I18n::english();
    let texts: Vec<String> = all_notices()
        .iter()
        .map(|n| notice_text(n, &i18n).1)
        .collect();
    for text in &texts {
        assert!(!text.contains('⟦'), "missing key: {text}");
        assert!(!text.contains('{'), "unfilled placeholder: {text}");
    }
    let distinct: BTreeSet<_> = texts.iter().collect();
    assert_eq!(distinct.len(), texts.len(), "{texts:#?}");
}

#[test]
fn an_empty_script_reads_the_same_from_the_ui_and_from_the_core() {
    let i18n = I18n::english();
    let core = ShellNotice::Refused(ShellRefusal::Invalid(ShellError::EmptyScript));
    assert_eq!(
        notice_text(&core, &i18n),
        notice_text(&ShellNotice::EmptyScript, &i18n)
    );
}

#[test]
fn the_exit_code_and_the_versions_are_named() {
    let i18n = I18n::english();
    let (severity, text) = notice_text(&ShellNotice::Finished(Outcome::Exited(7)), &i18n);
    assert_eq!(severity, Severity::Error);
    assert!(text.contains('7'), "{text}");
    let old = ShellNotice::ToolProblem(ToolProblem::TooOld {
        tool: ToolVersion::new(1, 2, 3),
        server: ServerVersion::new(8, 0),
    });
    let (_, text) = notice_text(&old, &i18n);
    assert!(text.contains("1.2.3") && text.contains("8.0"), "{text}");
}

#[test]
fn a_bad_address_names_the_problem() {
    let i18n = I18n::english();
    let notice = ShellNotice::Refused(ShellRefusal::Invalid(ShellError::Uri(UriProblem::BadPort)));
    let (_, text) = notice_text(&notice, &i18n);
    assert!(
        text.contains(&i18n.t(UriProblem::BadPort.i18n_key())),
        "{text}"
    );
}

#[test]
fn a_failed_start_does_not_blame_only_the_mongosh_path() {
    let i18n = I18n::english();
    let (_, text) = notice_text(&ShellNotice::Finished(Outcome::StartFailed), &i18n);
    assert!(text.contains("password"), "{text}");
}

#[test]
fn success_cancel_and_failure_differ_in_severity() {
    let i18n = I18n::english();
    let of = |o| notice_text(&ShellNotice::Finished(o), &i18n).0;
    assert_eq!(of(Outcome::Exited(0)), Severity::Success);
    assert_eq!(of(Outcome::Cancelled), Severity::Info);
    assert_eq!(of(Outcome::Incomplete), Severity::Error);
    assert_eq!(of(Outcome::ConnectFailed), Severity::Error);
}

#[test]
fn only_a_missing_or_too_old_mongosh_offers_the_tool_paths() {
    let offering: Vec<_> = all_notices()
        .into_iter()
        .filter(offers_tool_paths)
        .collect();
    assert_eq!(offering.len(), 2, "{offering:?}");
    assert!(
        offering
            .iter()
            .all(|n| matches!(n, ShellNotice::ToolProblem(_)))
    );
    assert!(!offers_tool_paths(&ShellNotice::Finished(
        Outcome::StartFailed
    )));
}
