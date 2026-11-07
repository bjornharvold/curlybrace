use super::*;
use crate::I18n;
use crate::app::tools_state::{ProbeScope, ToolsState};
use curlybrace_core::tools::{ToolChoice, ToolError, ToolKind, ToolVersion};
use uuid::Uuid;

fn done(
    state: ToolsState,
    scope: ProbeScope,
    kind: ToolKind,
    r: Result<ToolVersion, ToolError>,
) -> ToolsState {
    let (state, queue) = state.recheck(scope, kind).take_queue();
    state.probed(scope, kind, queue[0].request, r)
}

#[test]
fn a_tool_nobody_chose_follows_the_path_and_is_unchecked() {
    let row = tool_row(
        &ToolsState::default(),
        ProbeScope::Global,
        ToolKind::Mongosh,
        None,
    );
    assert_eq!(row.choice, ToolChoice::Path);
    assert!(row.own);
    assert_eq!(row.status, ToolStatus::Unchecked);
}

#[test]
fn a_check_in_flight_is_shown_as_checking() {
    let state = ToolsState::default().recheck(ProbeScope::Global, ToolKind::Mongosh);
    let row = tool_row(&state, ProbeScope::Global, ToolKind::Mongosh, None);
    assert_eq!(row.status, ToolStatus::Checking);
}

#[test]
fn a_version_is_shown_and_is_not_old_without_a_server() {
    let state = done(
        ToolsState::default(),
        ProbeScope::Global,
        ToolKind::Mongosh,
        Ok(ToolVersion::new(1, 0, 0)),
    );
    let row = tool_row(&state, ProbeScope::Global, ToolKind::Mongosh, None);
    assert_eq!(
        row.status,
        ToolStatus::Found {
            version: ToolVersion::new(1, 0, 0),
            too_old: false
        }
    );
}

#[test]
fn a_version_older_than_the_server_needs_is_marked() {
    let state = done(
        ToolsState::default(),
        ProbeScope::Global,
        ToolKind::Mongosh,
        Ok(ToolVersion::new(1, 10, 0)),
    );
    let server = ServerVersion::new(8, 0);
    let row = tool_row(&state, ProbeScope::Global, ToolKind::Mongosh, Some(server));
    assert_eq!(
        row.status,
        ToolStatus::Found {
            version: ToolVersion::new(1, 10, 0),
            too_old: true
        }
    );
}

#[test]
fn a_failed_check_is_a_problem() {
    let state = done(
        ToolsState::default(),
        ProbeScope::Global,
        ToolKind::Mongodump,
        Err(ToolError::Missing),
    );
    let row = tool_row(&state, ProbeScope::Global, ToolKind::Mongodump, None);
    assert_eq!(row.status, ToolStatus::Problem(ToolError::Missing));
}

#[test]
fn a_connection_without_an_override_follows_the_global_choice() {
    let id = Uuid::new_v4();
    let state =
        ToolsState::default().choose_global(ToolKind::Mongosh, ToolChoice::Executable("/g".into()));
    let row = tool_row(&state, ProbeScope::Connection(id), ToolKind::Mongosh, None);
    assert!(!row.own);
    assert_eq!(row.choice, ToolChoice::Executable("/g".into()));
}

#[test]
fn a_following_connection_shows_the_global_check() {
    let id = Uuid::new_v4();
    let state = done(
        ToolsState::default(),
        ProbeScope::Global,
        ToolKind::Mongosh,
        Ok(ToolVersion::new(2, 3, 1)),
    );
    let row = tool_row(&state, ProbeScope::Connection(id), ToolKind::Mongosh, None);
    assert!(!row.own);
    assert_eq!(
        row.status,
        ToolStatus::Found {
            version: ToolVersion::new(2, 3, 1),
            too_old: false
        }
    );
}

#[test]
fn a_connection_with_its_own_file_ignores_the_global_check() {
    let id = Uuid::new_v4();
    let state = done(
        ToolsState::default(),
        ProbeScope::Global,
        ToolKind::Mongosh,
        Ok(ToolVersion::new(2, 3, 1)),
    )
    .choose_override(
        id,
        ToolKind::Mongosh,
        ToolChoice::Executable("/mine".into()),
    );
    let row = tool_row(&state, ProbeScope::Connection(id), ToolKind::Mongosh, None);
    assert!(row.own);
    assert_eq!(row.status, ToolStatus::Checking);
}

#[test]
fn a_connection_override_is_its_own() {
    let id = Uuid::new_v4();
    let state = ToolsState::default()
        .choose_global(ToolKind::Mongosh, ToolChoice::Executable("/g".into()))
        .choose_override(
            id,
            ToolKind::Mongosh,
            ToolChoice::Executable("/mine".into()),
        );
    let row = tool_row(&state, ProbeScope::Connection(id), ToolKind::Mongosh, None);
    assert!(row.own);
    assert_eq!(row.choice, ToolChoice::Executable("/mine".into()));
}

#[test]
fn the_status_line_names_the_version() {
    let i18n = I18n::english();
    let line = status_line(
        &ToolStatus::Found {
            version: ToolVersion::new(2, 3, 1),
            too_old: false,
        },
        None,
        &i18n,
    );
    assert!(line.text.contains("2.3.1"));
    assert_eq!(line.tone, Tone::Neutral);
}

#[test]
fn an_old_version_is_a_warning_that_names_both_versions() {
    let i18n = I18n::english();
    let status = ToolStatus::Found {
        version: ToolVersion::new(1, 10, 0),
        too_old: true,
    };
    let line = status_line(&status, Some(ServerVersion::new(8, 0)), &i18n);
    assert!(line.text.contains("1.10.0") && line.text.contains("8.0"));
    assert_eq!(line.tone, Tone::Warning);
}

#[test]
fn a_problem_is_an_error_with_its_own_text() {
    let i18n = I18n::english();
    let line = status_line(&ToolStatus::Problem(ToolError::NotExecutable), None, &i18n);
    assert_eq!(line.tone, Tone::Error);
    assert_eq!(line.text, i18n.t(ToolError::NotExecutable.i18n_key()));
}

#[test]
fn every_status_has_a_text() {
    let i18n = I18n::english();
    for status in [ToolStatus::Unchecked, ToolStatus::Checking] {
        assert!(!status_line(&status, None, &i18n).text.is_empty());
    }
}
