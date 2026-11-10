use super::*;

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
    let next = state.output_arrived(id, Stream::Stdout, "a\nb\n");
    assert!(state.output.is_empty());
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
    assert_eq!(state.output[1].text, "next!");
    assert_eq!(state.output[2].text, "z");
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
    assert_eq!(state.output.len(), MAX_OUTPUT_LINES);
    assert_eq!(state.output[0].text, "l5");
    assert_eq!(
        state.output.last().unwrap().text,
        format!("l{}", MAX_OUTPUT_LINES + 4)
    );
}

#[test]
fn a_selection_off_a_char_boundary_does_not_panic() {
    // 'é' is two bytes at 1..3; both bounds can fall inside a char.
    let (_, request) = with_script("aébc", Some(2..3)).run_requested();
    assert_eq!(request.expect("request").script, "é");
    let (_, request) = with_script("éé", Some(1..3)).run_requested();
    assert_eq!(request.expect("request").script, "éé");
    let _ = with_script("éé", Some(1..3)).formatted();
}
