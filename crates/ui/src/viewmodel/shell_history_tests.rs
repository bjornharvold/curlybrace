use super::*;
use crate::viewmodel::shell::{ShellNotice, ShellState, ShellStatus};
use curlybrace_core::history::ShellRunStatus;
use curlybrace_core::shell::{Outcome, Stream};

fn entry(id: i64, script: &str) -> ShellHistoryEntry {
    ShellHistoryEntry {
        id,
        ran_at_secs: 1_700_000_000 + id,
        script: script.to_string(),
        status: ShellRunStatus::Succeeded,
    }
}

fn loaded() -> ShellState {
    ShellState::default().history_loaded(vec![entry(2, "db.b.find()"), entry(1, "db.a.find()")])
}

#[test]
fn defaults_are_not_loaded_and_not_available() {
    let h = ShellState::default().history;
    assert!(!h.loaded && !h.available && !h.open && !h.confirm_clear);
    assert!(h.entries.is_empty());
}

#[test]
fn loading_replaces_the_list_and_marks_available() {
    let state = loaded();
    assert!(state.history.loaded && state.history.available);
    assert_eq!(state.history.entries.len(), 2);
    let again = state.history_loaded(vec![entry(3, "x")]);
    assert_eq!(again.history.entries.len(), 1);
    assert_eq!(again.history.entries[0].id, 3);
    let empty = again.history_loaded(vec![]);
    assert!(empty.history.available && empty.history.entries.is_empty());
}

#[test]
fn unavailable_empties_the_list_and_marks_loaded() {
    let state = loaded().history_unavailable();
    assert!(state.history.loaded);
    assert!(!state.history.available);
    assert!(state.history.entries.is_empty());
}

#[test]
fn toggling_flips_open() {
    let state = loaded().history_toggled();
    assert!(state.history.open);
    assert!(!state.history_toggled().history.open);
}

#[test]
fn recalling_replaces_the_script_and_clears_the_selection() {
    let state = loaded().edited("old".to_string(), Some(0..2));
    let state = state.history_entry_recalled(1);
    assert_eq!(state.script, "db.a.find()");
    assert_eq!(state.selection, None);
    assert_eq!(state.history.entries.len(), 2);
}

#[test]
fn recalling_while_running_only_edits_the_buffer() {
    let (running, request) = ShellState::default()
        .edited("db.x.find()".to_string(), None)
        .run_requested();
    let run_id = request.expect("request").run_id;
    let running = running
        .history_loaded(vec![entry(1, "db.a.find()")])
        .output_arrived(run_id, Stream::Stdout, "hi");
    let after = running.history_entry_recalled(1);
    assert_eq!(after.script, "db.a.find()");
    assert_eq!(after.status, ShellStatus::Running { run_id });
    assert_eq!(after.output, running.output);
    assert_eq!(after.notice, running.notice);
}

#[test]
fn recalling_keeps_the_notice_of_a_finished_run() {
    let (running, request) = ShellState::default()
        .edited("x".to_string(), None)
        .run_requested();
    let finished = running
        .finished(request.expect("request").run_id, Outcome::Exited(0))
        .history_loaded(vec![entry(1, "y")]);
    let after = finished.history_entry_recalled(1);
    assert!(matches!(after.notice, Some(ShellNotice::Finished(_))));
}

#[test]
fn an_unknown_id_changes_nothing() {
    let state = loaded().edited("keep".to_string(), Some(1..2));
    assert_eq!(state.history_entry_recalled(99), state);
}

#[test]
fn clear_needs_a_confirm_step() {
    let state = loaded();
    let (untouched, send) = state.clear_confirmed();
    assert!(!send);
    assert_eq!(untouched, state);

    let asked = state.clear_requested();
    assert!(asked.history.confirm_clear);
    assert_eq!(asked.history.entries.len(), 2);
    let (confirmed, send) = asked.clear_confirmed();
    assert!(send);
    assert!(!confirmed.history.confirm_clear);
    assert_eq!(confirmed.history.entries.len(), 2);
}

#[test]
fn cancelling_the_clear_keeps_the_entries() {
    let cancelled = loaded().clear_requested().clear_cancelled();
    assert!(!cancelled.history.confirm_clear);
    assert_eq!(cancelled.history.entries.len(), 2);
    assert!(!cancelled.clear_confirmed().1);
}

#[test]
fn first_line_skips_blank_lines_and_cuts_on_a_char_boundary() {
    assert_eq!(first_line("\n  \n  db.a.find()  \nnext", 40), "db.a.find()");
    assert_eq!(first_line("", 5), "");
    assert_eq!(first_line(" \n\t\n", 5), "");
    assert_eq!(first_line("abcde", 5), "abcde");
    assert_eq!(first_line("abcdef", 5), "abcde…");
    assert_eq!(first_line("héllo wörld ✓✓", 8), "héllo wö…");
    assert_eq!(first_line("日本語日本語", 3), "日本語…");
}
