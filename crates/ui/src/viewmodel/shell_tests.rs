use super::*;
use crate::viewmodel::shell_check::ProblemKind;

fn with_script(script: &str, selection: Option<Range<usize>>) -> ShellState {
    ShellState::default().edited(script.to_string(), selection)
}

fn running(script: &str) -> (ShellState, u64) {
    let (state, request) = with_script(script, None).run_requested();
    (state, request.expect("a request").run_id)
}

fn texts(state: &ShellState) -> Vec<(Stream, &str)> {
    state
        .output
        .iter()
        .map(|l| (l.stream, l.text.as_str()))
        .collect()
}

#[test]
fn run_uses_the_selection_else_the_whole_script() {
    let (state, request) = with_script("db.a.find()\ndb.b.find()", Some(12..23)).run_requested();
    let request = request.expect("request");
    assert_eq!(request.script, "db.b.find()");
    assert_eq!(request.run_id, 1);
    assert_eq!(state.status, ShellStatus::Running { run_id: 1 });
    assert_eq!(state.next_run_id, 2);

    let (_, whole) = with_script("db.a.find()", None).run_requested();
    assert_eq!(whole.expect("request").script, "db.a.find()");
    let (_, empty_sel) = with_script("db.a.find()", Some(3..3)).run_requested();
    assert_eq!(empty_sel.expect("request").script, "db.a.find()");
    let (_, out_of_range) = with_script("db.a.find()", Some(50..60)).run_requested();
    assert_eq!(out_of_range.expect("request").script, "db.a.find()");
}

#[test]
fn run_clears_output_and_the_previous_notice() {
    let (state, id) = running("x");
    let state = state
        .output_arrived(id, Stream::Stdout, "hi\n")
        .finished(id, Outcome::Cancelled);
    let (state, request) = state.run_requested();
    assert!(request.is_some());
    assert!(state.output.is_empty());
    assert_eq!(state.notice, None);
}

#[test]
fn empty_or_whitespace_script_gives_no_request_and_a_notice() {
    for script in ["", "  \n\t "] {
        let (state, request) = with_script(script, None).run_requested();
        assert_eq!(request, None);
        assert_eq!(state.notice, Some(ShellNotice::EmptyScript));
        assert_eq!(state.status, ShellStatus::Idle);
        assert_eq!(state.next_run_id, 1);
    }
    let (_, request) = with_script("db.a.find()   ", Some(11..14)).run_requested();
    assert_eq!(request, None);
}

#[test]
fn run_while_running_gives_no_request() {
    let (state, _) = running("db.a.find()");
    let (again, request) = state.run_requested();
    assert_eq!(request, None);
    assert_eq!(again, state);
    assert!(state.is_running());
    assert!(!state.can_run());
}

#[test]
fn output_of_an_old_run_is_ignored() {
    let (state, id) = running("x");
    assert_eq!(state.output_arrived(id + 1, Stream::Stdout, "no\n"), state);
    assert_eq!(
        ShellState::default().output_arrived(1, Stream::Stdout, "no\n"),
        ShellState::default()
    );
}

#[test]
fn output_appends_without_mutating_the_old_state() {
    let (state, id) = running("x");
    let before = state.clone();
    let next = state.output_arrived(id, Stream::Stdout, "a\nb\n");
    assert_eq!(state, before);
    assert_eq!(
        texts(&next),
        vec![(Stream::Stdout, "a"), (Stream::Stdout, "b")]
    );
}

#[test]
fn a_line_printed_in_pieces_stays_one_line() {
    let (state, id) = running("x");
    let state = state
        .output_arrived(id, Stream::Stdout, "hel")
        .output_arrived(id, Stream::Stdout, "lo wor")
        .output_arrived(id, Stream::Stdout, "ld\nnext");
    assert_eq!(
        texts(&state),
        vec![(Stream::Stdout, "hello world"), (Stream::Stdout, "next")]
    );
    let state = state
        .output_arrived(id, Stream::Stdout, "!\n")
        .output_arrived(id, Stream::Stdout, "z");
    assert_eq!(state.line(1).unwrap().text, "next!");
    assert_eq!(state.line(2).unwrap().text, "z");
}

#[test]
fn streams_do_not_merge_lines() {
    let (state, id) = running("x");
    let state = state
        .output_arrived(id, Stream::Stdout, "out")
        .output_arrived(id, Stream::Stderr, "err\n")
        .output_arrived(id, Stream::Stdout, "more");
    assert_eq!(
        texts(&state),
        vec![
            (Stream::Stdout, "out"),
            (Stream::Stderr, "err"),
            (Stream::Stdout, "more")
        ]
    );
}

#[test]
fn finished_returns_to_idle_with_a_notice() {
    let (state, id) = running("x");
    let done = state.finished(id, Outcome::Exited(0));
    assert_eq!(done.status, ShellStatus::Idle);
    assert_eq!(done.notice, Some(ShellNotice::Finished(Outcome::Exited(0))));
    assert!(done.can_run());
}

#[test]
fn finished_ignored_for_a_stale_run() {
    let (state, id) = running("x");
    assert_eq!(state.finished(id + 1, Outcome::Cancelled), state);
}

#[test]
fn refused_returns_to_idle_only_for_the_running_run() {
    let (state, id) = running("x");
    assert_eq!(state.refused(id + 1, ShellRefusal::ReadOnly), state);
    let refused = state.refused(id, ShellRefusal::ReadOnly);
    assert_eq!(refused.status, ShellStatus::Idle);
    assert_eq!(
        refused.notice,
        Some(ShellNotice::Refused(ShellRefusal::ReadOnly))
    );
}

#[test]
fn tool_problem_sets_the_notice_and_keeps_the_status() {
    let (state, _) = running("x");
    let problem = ToolProblem::Missing(ToolError::Missing);
    let next = state.tool_problem(problem.clone());
    assert_eq!(next.notice, Some(ShellNotice::ToolProblem(problem)));
    assert_eq!(next.status, state.status);
}

#[test]
fn edit_while_running_keeps_the_run() {
    let (state, id) = running("x");
    let state = state.output_arrived(id, Stream::Stdout, "a\n");
    let edited = state.edited("y".to_string(), Some(0..1));
    assert_eq!(edited.status, ShellStatus::Running { run_id: id });
    assert_eq!(edited.output, state.output);
    assert_eq!(edited.script, "y");
    assert_eq!(edited.selection, Some(0..1));
}

#[test]
fn format_keeps_the_selection_mapped() {
    let script = "if (a) {\nb();\n}";
    let start = script.find("b()").unwrap();
    let state = with_script(script, Some(start..start + 4));
    let out = state.formatted();
    assert_eq!(out.script, "if (a) {\n  b();\n}");
    let sel = out.selection.expect("selection");
    assert_eq!(&out.script[sel], "b();");
    assert_eq!(state.script, script);
}

#[test]
fn output_is_bounded() {
    let (state, id) = running("x");
    let big: String = (0..MAX_OUTPUT_LINES + 5)
        .map(|i| format!("l{i}\n"))
        .collect();
    let state = state.output_arrived(id, Stream::Stdout, &big);
    assert_eq!(state.line_count(), MAX_OUTPUT_LINES);
    assert_eq!(state.line(0).unwrap().text, "l5");
    assert_eq!(
        state.line(MAX_OUTPUT_LINES - 1).unwrap().text,
        format!("l{}", MAX_OUTPUT_LINES + 4)
    );
    assert_eq!(state.line(MAX_OUTPUT_LINES), None);
}

#[test]
fn dropped_lines_are_counted_across_chunks_and_reset_by_a_new_run() {
    let (state, id) = running("x");
    let mut state = state;
    for chunk in 0..5 {
        let text: String = (0..(MAX_OUTPUT_LINES + 25) / 5)
            .map(|i| format!("l{}\n", chunk * ((MAX_OUTPUT_LINES + 25) / 5) + i))
            .collect();
        state = state.output_arrived(id, Stream::Stdout, &text);
    }
    assert_eq!(state.line_count(), MAX_OUTPUT_LINES);
    assert_eq!(state.dropped_lines, 25);
    assert_eq!(state.line(0).unwrap().text, "l25");
    let (next, request) = state.finished(id, Outcome::Exited(0)).run_requested();
    assert!(request.is_some());
    assert_eq!(next.dropped_lines, 0);
    assert_eq!(next.line_count(), 0);
}

#[test]
fn a_chunk_of_a_stale_run_does_not_touch_the_counters() {
    let (state, id) = running("x");
    let state = state.output_arrived(id, Stream::Stdout, "a\n");
    let same = state.output_arrived(id + 1, Stream::Stdout, "b\n");
    assert_eq!(same, state);
    assert!(same.output.first_segment_shared_with(&state.output));
}

#[test]
fn output_arrived_shares_the_old_lines_with_the_new_state() {
    let (state, id) = running("x");
    // More than one segment, so the first is not the one being appended to.
    let lines: String = (0..600).map(|i| format!("{i}\n")).collect();
    let before = state.output_arrived(id, Stream::Stdout, &lines);
    let snapshot = before.clone();
    let after = before.output_arrived(id, Stream::Stdout, "c\n");
    assert_eq!(before, snapshot);
    assert_eq!(before.line_count(), 600);
    assert_eq!(after.line_count(), 601);
    assert_eq!(before.line(599).unwrap().text, "599");
    assert!(before.output.first_segment_shared_with(&after.output));
}

#[test]
fn many_short_chunks_keep_the_window_and_share_the_head() {
    let (state, id) = running("x");
    let lines: String = (0..600).map(|i| format!("first{i}\n")).collect();
    let start = state.output_arrived(id, Stream::Stdout, &lines);
    let mut state = start.clone();
    for i in 0..50_000 {
        state = state.output_arrived(id, Stream::Stdout, &format!("{i}\n"));
        if i == 1000 {
            assert!(start.output.first_segment_shared_with(&state.output));
        }
    }
    assert_eq!(state.line_count(), MAX_OUTPUT_LINES);
    assert_eq!(state.dropped_lines, 50_600 - MAX_OUTPUT_LINES as u64);
    assert_eq!(state.line(MAX_OUTPUT_LINES - 1).unwrap().text, "49999");
}

#[test]
fn finishing_closes_the_partial_line() {
    let (state, id) = running("x");
    let state = state.output_arrived(id, Stream::Stdout, "part");
    let done = state.finished(id, Outcome::Cancelled);
    assert_eq!(texts(&done), vec![(Stream::Stdout, "part")]);
    // A later run restarts the log; a closed tail is never continued.
    assert_eq!(done.output, state.output.closed());
    let refused = state.refused(id, ShellRefusal::ReadOnly);
    assert_eq!(refused.output, state.output.closed());
}

#[test]
fn a_selection_off_a_char_boundary_does_not_panic() {
    // 'é' is two bytes at 1..3; both bounds can fall inside a char.
    let (_, request) = with_script("aébc", Some(2..3)).run_requested();
    assert_eq!(request.expect("request").script, "é");
    let (_, request) = with_script("éé", Some(1..3)).run_requested();
    assert_eq!(request.expect("request").script, "éé");
    let formatted = with_script("éé", Some(1..3)).formatted();
    assert_eq!(formatted.script, "éé");
    let selection = formatted.selection.expect("selection");
    assert_eq!(&formatted.script[selection], "éé");
}

#[test]
fn started_runs_the_given_text_whatever_the_editor_holds() {
    let (state, _) = running("first");
    let state = state
        .output_arrived(1, Stream::Stdout, "old output\n")
        .output_arrived(1, Stream::Stderr, "old error\n")
        .finished(1, Outcome::ScriptErrors)
        .edited(String::new(), None);
    // What the new run must clear.
    assert_eq!(state.line_count(), 2);
    assert!(state.notice.is_some());

    let (state, request) = state.started("db.a.find()");

    assert_eq!(request.script, "db.a.find()");
    assert_eq!(request.run_id, 2);
    assert_eq!(state.status, ShellStatus::Running { run_id: 2 });
    assert_eq!(state.next_run_id, 3);
    assert_eq!(state.notice, None);
    assert_eq!(state.line_count(), 0);
}

fn syntax_problem(state: &ShellState) -> &SyntaxProblem {
    match &state.notice {
        Some(ShellNotice::SyntaxProblem(problem)) => problem,
        other => panic!("no syntax problem: {other:?}"),
    }
}

#[test]
fn a_broken_script_is_not_run_and_says_why() {
    let state = with_script("db.a.find({)", None);
    let (next, request) = state.run_requested();
    assert_eq!(request, None);
    let problem = syntax_problem(&next);
    assert_eq!(
        problem.kind,
        ProblemKind::MismatchedClosing {
            opener: '{',
            found: ')'
        }
    );
    assert_eq!(problem.range, 11..12);
    assert_eq!((problem.line, problem.column), (1, 12));
    assert_eq!(next.status, ShellStatus::Idle);
    assert_eq!(next.next_run_id, 1);
    assert_eq!(next.script, state.script);
}

#[test]
fn a_refused_broken_script_keeps_the_previous_output() {
    let (state, id) = running("x");
    let state = state
        .output_arrived(id, Stream::Stdout, "kept\n")
        .finished(id, Outcome::Exited(0))
        .edited("db.a.find(".into(), None);
    let (next, request) = state.run_requested();
    assert_eq!(request, None);
    assert_eq!(next.output, state.output);
    assert_eq!(next.result, state.result);
}

#[test]
fn run_anyway_skips_the_check_once() {
    let state = with_script("db.a.find({)", None);
    let (refused, _) = state.run_requested();
    let (next, request) = refused.run_anyway();
    assert_eq!(request.expect("a request").script, "db.a.find({)");
    assert_eq!(next.status, ShellStatus::Running { run_id: 1 });
    assert_eq!(next.notice, None);
    let (_, again) = next.finished(1, Outcome::Exited(0)).run_requested();
    assert_eq!(again, None, "the next Run checks again");
}

#[test]
fn run_anyway_still_refuses_an_empty_script_and_a_second_run() {
    let (state, request) = with_script("  ", None).run_anyway();
    assert_eq!(request, None);
    assert_eq!(state.notice, Some(ShellNotice::EmptyScript));
    let (busy, _) = with_script("x", None).run_requested();
    assert_eq!(busy.run_anyway(), (busy.clone(), None));
}

#[test]
fn a_good_selection_of_a_broken_script_runs() {
    let script = "db.a.find()\ndb.b.find(";
    let (_, request) = with_script(script, Some(0..11)).run_requested();
    assert_eq!(request.expect("a request").script, "db.a.find()");
}

#[test]
fn a_problem_in_the_selection_is_placed_in_the_whole_script() {
    let script = "db.a.find()\nlet s = 'é\nx";
    let start = script.find("let").unwrap();
    let state = with_script(script, Some(start..script.len()));
    let (next, request) = state.run_requested();
    assert_eq!(request, None);
    let problem = syntax_problem(&next);
    assert_eq!(problem.kind, ProblemKind::UnterminatedString);
    let quote = script.find('\'').unwrap();
    assert_eq!(problem.range, quote..quote + 1);
    assert_eq!((problem.line, problem.column), (2, 9));
}

#[test]
fn go_to_problem_selects_the_problem_and_keeps_the_rest() {
    let script = "db.a.find()\ndb.b.find(";
    let (state, _) = with_script(script, None).run_requested();
    let next = state.problem_selected();
    assert_eq!(next.selection, Some(21..22));
    assert_eq!(&next.script[21..22], "(");
    assert_eq!(next.notice, state.notice);
    assert_eq!(next.output, state.output);
    assert_eq!(next.status, state.status);
}

#[test]
fn every_go_to_problem_asks_the_editor_for_the_focus_once_more() {
    let (state, _) = with_script("db.b.find(", None).run_requested();
    let once = state.problem_selected();
    let twice = once.problem_selected();
    assert_ne!(once.editor_focus, state.editor_focus);
    assert_ne!(
        twice.editor_focus, once.editor_focus,
        "a second click asks again"
    );
    assert_eq!(twice.selection, once.selection);
    let typed = twice.edited("db.b.find()".into(), None);
    assert_eq!(
        typed.editor_focus, twice.editor_focus,
        "typing asks nothing"
    );
    assert_eq!(typed.formatted().editor_focus, twice.editor_focus);
}

#[test]
fn go_to_problem_without_a_syntax_notice_changes_nothing() {
    let state = with_script("db.a.find()", Some(0..2));
    assert_eq!(state.problem_selected(), state);
    let (empty, _) = with_script(" ", None).run_requested();
    assert_eq!(empty.problem_selected(), empty);
}

#[test]
fn go_to_problem_keeps_the_range_inside_the_script_and_on_char_boundaries() {
    let problem = SyntaxProblem {
        kind: ProblemKind::UnterminatedComment,
        range: 1..40,
        line: 1,
        column: 2,
    };
    let state = ShellState {
        notice: Some(ShellNotice::SyntaxProblem(problem)),
        ..with_script("éab", None)
    };
    assert_eq!(state.problem_selected().selection, Some(0..4));
}

#[test]
fn editing_the_text_or_formatting_clears_a_syntax_notice() {
    let (state, _) = with_script("if (a) {\nb();", None).run_requested();
    assert!(matches!(state.notice, Some(ShellNotice::SyntaxProblem(_))));
    let typed = state.edited("if (a) {\nb();}".into(), None);
    assert_eq!(typed.notice, None);
    let moved = state.edited(state.script.clone(), Some(0..2));
    assert_eq!(
        moved.notice, state.notice,
        "moving the caret or the selection keeps it"
    );
    let formatted = state.formatted();
    assert_ne!(formatted.script, state.script);
    assert_eq!(formatted.notice, None);
}

#[test]
fn editing_keeps_the_other_notices_as_before() {
    let (state, _) = with_script(" ", None).run_requested();
    let edited = state.edited("x".into(), None);
    assert_eq!(edited.notice, Some(ShellNotice::EmptyScript));
    let (state, id) = running("x");
    let done = state.finished(id, Outcome::Exited(0));
    assert_eq!(done.edited("y".into(), None).notice, done.notice);
    assert_eq!(done.formatted().notice, done.notice);
}
