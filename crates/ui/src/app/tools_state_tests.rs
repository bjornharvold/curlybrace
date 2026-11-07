//! The pure part of the tool paths: what a choice changes, which version checks it asks for and
//! which answers are still wanted. Nothing here runs a program or touches a file.
use super::tools_state::{ProbeScope, ProbeStatus, ToolsState};
use curlybrace_core::tools::{ToolChoice, ToolError, ToolKind, ToolVersion};
use uuid::Uuid;

fn exe(path: &str) -> ToolChoice {
    ToolChoice::Executable(path.into())
}

const V2: ToolVersion = ToolVersion::new(2, 0, 0);

#[test]
fn choosing_a_global_tool_saves_it_and_asks_for_one_version_check() {
    let state = ToolsState::default().choose_global(ToolKind::Mongosh, exe("/opt/mongosh"));
    assert_eq!(
        state.settings.global(ToolKind::Mongosh),
        exe("/opt/mongosh")
    );
    assert!(matches!(
        state.status(ProbeScope::Global, ToolKind::Mongosh),
        Some(ProbeStatus::Checking { .. })
    ));
    let (state, queue) = state.take_queue();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].scope, ProbeScope::Global);
    assert_eq!(queue[0].kind, ToolKind::Mongosh);
    assert_eq!(queue[0].choice, exe("/opt/mongosh"));
    assert!(
        state.take_queue().1.is_empty(),
        "the queue is emptied once taken"
    );
}

#[test]
fn a_matching_answer_is_stored() {
    let (state, queue) = ToolsState::default()
        .choose_global(ToolKind::Mongodump, ToolChoice::Path)
        .take_queue();
    let state = state.probed(
        ProbeScope::Global,
        ToolKind::Mongodump,
        queue[0].request,
        Ok(V2),
    );
    assert_eq!(
        state.status(ProbeScope::Global, ToolKind::Mongodump),
        Some(&ProbeStatus::Done(Ok(V2)))
    );
}

#[test]
fn an_answer_for_a_replaced_choice_is_dropped() {
    let (state, first) = ToolsState::default()
        .choose_global(ToolKind::Mongosh, exe("/old"))
        .take_queue();
    let (state, second) = state
        .choose_global(ToolKind::Mongosh, exe("/new"))
        .take_queue();
    assert_ne!(first[0].request, second[0].request);
    let state = state.probed(
        ProbeScope::Global,
        ToolKind::Mongosh,
        first[0].request,
        Err(ToolError::Missing),
    );
    assert!(matches!(
        state.status(ProbeScope::Global, ToolKind::Mongosh),
        Some(ProbeStatus::Checking { .. })
    ));
}

#[test]
fn an_answer_nobody_asked_for_is_ignored() {
    let state = ToolsState::default().probed(ProbeScope::Global, ToolKind::Mongosh, 7, Ok(V2));
    assert_eq!(state, ToolsState::default());
}

#[test]
fn a_connection_override_is_checked_on_its_own() {
    let id = Uuid::new_v4();
    let state = ToolsState::default()
        .choose_global(ToolKind::Mongosh, exe("/global"))
        .choose_override(id, ToolKind::Mongosh, exe("/mine"));
    assert!(state.settings.overrides(id, ToolKind::Mongosh));
    let (_, queue) = state.take_queue();
    let mine = queue
        .iter()
        .find(|r| r.scope == ProbeScope::Connection(id))
        .expect("a check for the connection");
    assert_eq!(mine.choice, exe("/mine"));
}

#[test]
fn using_the_global_choice_again_removes_the_override_and_its_check() {
    let id = Uuid::new_v4();
    let state = ToolsState::default()
        .choose_global(ToolKind::Mongosh, exe("/global"))
        .choose_override(id, ToolKind::Mongosh, exe("/mine"))
        .use_global(id, ToolKind::Mongosh);
    assert!(!state.settings.overrides(id, ToolKind::Mongosh));
    assert_eq!(
        state.status(ProbeScope::Connection(id), ToolKind::Mongosh),
        None
    );
    let (_, queue) = state.take_queue();
    assert!(
        queue.iter().all(|r| r.scope == ProbeScope::Global),
        "no check is left for the removed override"
    );
}

#[test]
fn checking_again_uses_the_choice_that_applies_to_the_scope() {
    let id = Uuid::new_v4();
    let (state, _) = ToolsState::default()
        .choose_global(ToolKind::Mongorestore, exe("/global"))
        .take_queue();
    let (_, queue) = state
        .recheck(ProbeScope::Connection(id), ToolKind::Mongorestore)
        .take_queue();
    assert_eq!(queue.len(), 1);
    assert_eq!(
        queue[0].choice,
        exe("/global"),
        "no override, so the global one"
    );
    assert_eq!(
        queue[0].scope,
        ProbeScope::Global,
        "a connection that follows the global choice shares the global check"
    );
}

#[test]
fn checking_a_connection_with_its_own_file_checks_that_file() {
    let id = Uuid::new_v4();
    let (state, _) = ToolsState::default()
        .choose_override(id, ToolKind::Mongosh, exe("/mine"))
        .take_queue();
    let (_, queue) = state
        .recheck(ProbeScope::Connection(id), ToolKind::Mongosh)
        .take_queue();
    assert_eq!(queue[0].scope, ProbeScope::Connection(id));
    assert_eq!(queue[0].choice, exe("/mine"));
}

#[test]
fn a_removed_connection_loses_its_overrides_and_checks() {
    let id = Uuid::new_v4();
    let other = Uuid::new_v4();
    let state = ToolsState::default()
        .choose_override(id, ToolKind::Mongosh, exe("/a"))
        .choose_override(other, ToolKind::Mongosh, exe("/b"))
        .connection_removed(id);
    assert!(!state.settings.overrides(id, ToolKind::Mongosh));
    assert_eq!(
        state.status(ProbeScope::Connection(id), ToolKind::Mongosh),
        None
    );
    assert!(state.settings.overrides(other, ToolKind::Mongosh));
    assert!(
        state
            .status(ProbeScope::Connection(other), ToolKind::Mongosh)
            .is_some()
    );
}

#[test]
fn a_duplicated_connection_starts_with_the_same_overrides_unchecked() {
    let from = Uuid::new_v4();
    let to = Uuid::new_v4();
    let state = ToolsState::default()
        .choose_override(from, ToolKind::Mongosh, exe("/a"))
        .connection_duplicated(from, to);
    assert_eq!(
        state.settings.effective(ToolKind::Mongosh, Some(to)),
        exe("/a")
    );
    assert_eq!(
        state.status(ProbeScope::Connection(to), ToolKind::Mongosh),
        None
    );
}

fn suggestions(
    kind: ToolKind,
    path: &str,
) -> std::collections::BTreeMap<ToolKind, Vec<std::path::PathBuf>> {
    [(kind, vec![path.into()])].into_iter().collect()
}

#[test]
fn opening_the_settings_checks_every_global_tool_once() {
    let state = ToolsState::default()
        .open_settings(suggestions(ToolKind::Mongosh, "/opt/homebrew/bin/mongosh"));
    assert!(state.dialog().is_some());
    let (_, queue) = state.take_queue();
    let mut kinds: Vec<_> = queue.iter().map(|r| (r.scope, r.kind)).collect();
    kinds.sort();
    let mut all: Vec<_> = ToolKind::ALL
        .iter()
        .map(|k| (ProbeScope::Global, *k))
        .collect();
    all.sort();
    assert_eq!(kinds, all);
}

#[test]
fn the_suggestions_found_at_opening_are_kept_for_the_dialog() {
    let state = ToolsState::default()
        .open_settings(suggestions(ToolKind::Mongodump, "/usr/local/bin/mongodump"));
    let dialog = state.dialog().unwrap();
    assert_eq!(
        dialog.suggestions(ToolKind::Mongodump),
        [std::path::PathBuf::from("/usr/local/bin/mongodump")]
    );
    assert!(dialog.suggestions(ToolKind::Mongosh).is_empty());
}

#[test]
fn closing_the_settings_keeps_the_choices() {
    let state = ToolsState::default()
        .choose_global(ToolKind::Mongosh, exe("/x"))
        .open_settings(Default::default())
        .close_settings();
    assert!(state.dialog().is_none());
    assert_eq!(state.settings.global(ToolKind::Mongosh), exe("/x"));
}

#[test]
fn browsing_remembers_what_the_file_is_for_and_only_one_dialog_opens() {
    let id = Uuid::new_v4();
    let state = ToolsState::default()
        .begin_browse(ProbeScope::Global, ToolKind::Mongosh)
        .begin_browse(ProbeScope::Connection(id), ToolKind::Mongodump);
    assert_eq!(
        state.browsing(),
        Some((ProbeScope::Global, ToolKind::Mongosh))
    );
}

#[test]
fn a_picked_file_becomes_the_global_choice_and_is_checked() {
    let state = ToolsState::default()
        .begin_browse(ProbeScope::Global, ToolKind::Mongosh)
        .browsed(Some("/opt/mongosh".into()));
    assert_eq!(state.browsing(), None);
    assert_eq!(
        state.settings.global(ToolKind::Mongosh),
        exe("/opt/mongosh")
    );
    assert!(matches!(
        state.status(ProbeScope::Global, ToolKind::Mongosh),
        Some(ProbeStatus::Checking { .. })
    ));
}

#[test]
fn a_picked_file_for_a_connection_becomes_its_override() {
    let id = Uuid::new_v4();
    let state = ToolsState::default()
        .begin_browse(ProbeScope::Connection(id), ToolKind::Mongorestore)
        .browsed(Some("/opt/restore".into()));
    assert!(state.settings.overrides(id, ToolKind::Mongorestore));
    assert_eq!(
        state.settings.global(ToolKind::Mongorestore),
        ToolChoice::Path
    );
}

#[test]
fn a_cancelled_dialog_changes_nothing() {
    let state = ToolsState::default()
        .begin_browse(ProbeScope::Global, ToolKind::Mongosh)
        .browsed(None);
    assert_eq!(state.browsing(), None);
    assert_eq!(state.settings, ToolsState::default().settings);
    assert_eq!(state.status(ProbeScope::Global, ToolKind::Mongosh), None);
}

#[test]
fn an_answer_with_no_dialog_waiting_is_ignored() {
    let state = ToolsState::default().browsed(Some("/stray".into()));
    assert_eq!(state, ToolsState::default());
}

#[test]
fn the_revision_moves_only_when_the_settings_can_have_changed() {
    let id = Uuid::new_v4();
    let start = ToolsState::default();
    assert_eq!(start.revision(), ToolsState::default().revision());
    let chosen = start.clone().choose_global(ToolKind::Mongosh, exe("/x"));
    assert_ne!(chosen.revision(), start.revision());
    let overridden = chosen
        .clone()
        .choose_override(id, ToolKind::Mongosh, exe("/y"));
    assert_ne!(overridden.revision(), chosen.revision());
    let removed = overridden.clone().connection_removed(id);
    assert_ne!(removed.revision(), overridden.revision());
    // Checks, dialogs and answers never touch the settings.
    let (idle, queue) = chosen
        .clone()
        .recheck(ProbeScope::Global, ToolKind::Mongosh)
        .take_queue();
    let answered = idle.clone().probed(
        ProbeScope::Global,
        ToolKind::Mongosh,
        queue[0].request,
        Ok(V2),
    );
    let dialog = answered
        .clone()
        .open_settings(Default::default())
        .close_settings();
    assert_eq!(dialog.revision(), chosen.revision());
}

#[test]
fn a_state_with_nothing_queued_says_so() {
    let state = ToolsState::default();
    assert!(!state.has_requests());
    assert!(
        state
            .recheck(ProbeScope::Global, ToolKind::Mongosh)
            .has_requests()
    );
}
